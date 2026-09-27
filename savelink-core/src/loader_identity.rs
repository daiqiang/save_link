//! Resolve the real game executable behind supported third-party loaders.

use crate::scan::path_is_same_or_descendant;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

const COLD_CLIENT_LOADER_CONFIG: &str = "ColdClientLoader.ini";
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderProgramIdentity {
    pub config_path: PathBuf,
    pub executable_path: PathBuf,
    pub identity_hints: Vec<String>,
    pub company_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoaderIdentityError {
    Io(String),
    ConfigTooLarge(PathBuf),
    MissingExecutableSetting(PathBuf),
    UnsafeExecutablePath(PathBuf),
    ExecutableMissing(PathBuf),
    ExecutableNotExe(PathBuf),
}

impl fmt::Display for LoaderIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(formatter, "读取加载器配置失败：{message}"),
            Self::ConfigTooLarge(path) => {
                write!(formatter, "加载器配置文件过大：{}", path.display())
            }
            Self::MissingExecutableSetting(path) => write!(
                formatter,
                "加载器配置缺少 [SteamClient] Exe：{}",
                path.display()
            ),
            Self::UnsafeExecutablePath(path) => {
                write!(formatter, "加载器真实程序路径不安全：{}", path.display())
            }
            Self::ExecutableMissing(path) => {
                write!(formatter, "加载器真实程序不存在：{}", path.display())
            }
            Self::ExecutableNotExe(path) => {
                write!(formatter, "加载器真实程序不是 EXE：{}", path.display())
            }
        }
    }
}

impl std::error::Error for LoaderIdentityError {}

pub type LoaderIdentityResult<T> = Result<T, LoaderIdentityError>;

/// Resolve a real executable from a supported loader configuration.
///
/// Absence of a supported configuration is not an error. Invalid configurations
/// are returned as errors so callers may diagnose them without trusting them.
pub fn resolve_loader_program_identity(
    launch_executable: &Path,
    install_dir: &Path,
) -> LoaderIdentityResult<Option<LoaderProgramIdentity>> {
    let Some(config_path) = find_cold_client_loader_config(launch_executable, install_dir) else {
        return Ok(None);
    };
    let metadata = fs::metadata(&config_path).map_err(io_error)?;
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err(LoaderIdentityError::ConfigTooLarge(config_path));
    }
    let bytes = fs::read(&config_path).map_err(io_error)?;
    let content = decode_config(&bytes);
    let executable_setting = parse_cold_client_executable(&content)
        .ok_or_else(|| LoaderIdentityError::MissingExecutableSetting(config_path.clone()))?;
    let relative_path = PathBuf::from(executable_setting);
    if !is_safe_relative_executable(&relative_path) {
        return Err(LoaderIdentityError::UnsafeExecutablePath(relative_path));
    }

    let config_dir = config_path.parent().unwrap_or(install_dir);
    let unresolved_executable = config_dir.join(&relative_path);
    if !unresolved_executable.is_file() {
        return Err(LoaderIdentityError::ExecutableMissing(
            unresolved_executable,
        ));
    }
    if !unresolved_executable
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("exe"))
    {
        return Err(LoaderIdentityError::ExecutableNotExe(unresolved_executable));
    }

    let canonical_install = fs::canonicalize(install_dir).map_err(io_error)?;
    let executable_path = fs::canonicalize(&unresolved_executable).map_err(io_error)?;
    if !path_is_same_or_descendant(&canonical_install, &executable_path) {
        return Err(LoaderIdentityError::UnsafeExecutablePath(executable_path));
    }

    let version_identity = executable_version_identity(&executable_path);
    let mut identity_hints = Vec::new();
    push_identity_hint(
        &mut identity_hints,
        executable_path.file_stem().and_then(|value| value.to_str()),
    );
    push_identity_hint(
        &mut identity_hints,
        version_identity.product_name.as_deref(),
    );
    push_identity_hint(
        &mut identity_hints,
        version_identity.file_description.as_deref(),
    );
    push_identity_hint(
        &mut identity_hints,
        version_identity.original_filename.as_deref(),
    );

    Ok(Some(LoaderProgramIdentity {
        config_path,
        executable_path,
        identity_hints,
        company_name: version_identity.company_name,
    }))
}

fn find_cold_client_loader_config(launch_executable: &Path, install_dir: &Path) -> Option<PathBuf> {
    let mut directories = Vec::new();
    if let Some(parent) = launch_executable.parent() {
        directories.push(parent.to_path_buf());
    }
    if !directories
        .iter()
        .any(|directory| normalized_path(directory) == normalized_path(install_dir))
    {
        directories.push(install_dir.to_path_buf());
    }
    directories
        .into_iter()
        .map(|directory| directory.join(COLD_CLIENT_LOADER_CONFIG))
        .find(|path| path.is_file())
}

fn parse_cold_client_executable(content: &str) -> Option<String> {
    let mut in_steam_client = false;
    for raw_line in content.lines() {
        let line = raw_line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_steam_client = line[1..line.len() - 1]
                .trim()
                .eq_ignore_ascii_case("SteamClient");
            continue;
        }
        if !in_steam_client {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("Exe") {
            continue;
        }
        let value = trim_matching_quotes(value.trim()).trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn trim_matching_quotes(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
        {
            return &value[1..value.len() - 1];
        }
    }
    value
}

fn decode_config(bytes: &[u8]) -> String {
    if let Some(body) = bytes.strip_prefix(&[0xff, 0xfe]) {
        let words = body
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        return String::from_utf16_lossy(&words);
    }
    if let Some(body) = bytes.strip_prefix(&[0xfe, 0xff]) {
        let words = body
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        return String::from_utf16_lossy(&words);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

fn is_safe_relative_executable(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && !path.components().any(|component| {
            matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::ParentDir
            )
        })
}

fn push_identity_hint(hints: &mut Vec<String>, value: Option<&str>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    let compact = compact_identity(value);
    if compact.chars().count() < 4
        || matches!(
            compact.as_str(),
            "game" | "game64" | "launcher" | "steamclientloader" | "coldclientloader" | "startgame"
        )
    {
        return;
    }
    if !hints
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(value))
    {
        hints.push(value.to_string());
    }
}

fn compact_identity(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase()
}

fn io_error(error: std::io::Error) -> LoaderIdentityError {
    LoaderIdentityError::Io(error.to_string())
}

#[derive(Default)]
struct ExecutableVersionIdentity {
    product_name: Option<String>,
    file_description: Option<String>,
    original_filename: Option<String>,
    company_name: Option<String>,
}

#[cfg(not(windows))]
fn executable_version_identity(_path: &Path) -> ExecutableVersionIdentity {
    ExecutableVersionIdentity::default()
}

#[cfg(windows)]
fn executable_version_identity(path: &Path) -> ExecutableVersionIdentity {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW};

    let wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(wide_path.as_ptr()), None) };
    if size == 0 {
        return ExecutableVersionIdentity::default();
    }
    let mut data = vec![0u8; size as usize];
    if unsafe {
        GetFileVersionInfoW(
            PCWSTR(wide_path.as_ptr()),
            None,
            size,
            data.as_mut_ptr().cast::<c_void>(),
        )
    }
    .is_err()
    {
        return ExecutableVersionIdentity::default();
    }

    let mut translations = unsafe { version_translations(&data) };
    if translations.is_empty() {
        translations.extend([(0x0409, 0x04b0), (0x0409, 0x04e4)]);
    }
    let query = |field: &str| {
        translations
            .iter()
            .find_map(|(language, code_page)| unsafe {
                query_version_string(&data, *language, *code_page, field)
            })
    };
    ExecutableVersionIdentity {
        product_name: query("ProductName"),
        file_description: query("FileDescription"),
        original_filename: query("OriginalFilename"),
        company_name: query("CompanyName"),
    }
}

#[cfg(windows)]
unsafe fn version_translations(data: &[u8]) -> Vec<(u16, u16)> {
    use std::ffi::c_void;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::VerQueryValueW;

    let query = wide_string(r"\VarFileInfo\Translation");
    let mut pointer = std::ptr::null_mut::<c_void>();
    let mut length = 0u32;
    if !VerQueryValueW(
        data.as_ptr().cast::<c_void>(),
        PCWSTR(query.as_ptr()),
        &mut pointer,
        &mut length,
    )
    .as_bool()
        || pointer.is_null()
        || length < 4
    {
        return Vec::new();
    }
    std::slice::from_raw_parts(pointer.cast::<u16>(), length as usize / 2)
        .chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        .collect()
}

#[cfg(windows)]
unsafe fn query_version_string(
    data: &[u8],
    language: u16,
    code_page: u16,
    field: &str,
) -> Option<String> {
    use std::ffi::c_void;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::VerQueryValueW;

    let query = wide_string(&format!(
        r"\StringFileInfo\{language:04x}{code_page:04x}\{field}"
    ));
    let mut pointer = std::ptr::null_mut::<c_void>();
    let mut length = 0u32;
    if !VerQueryValueW(
        data.as_ptr().cast::<c_void>(),
        PCWSTR(query.as_ptr()),
        &mut pointer,
        &mut length,
    )
    .as_bool()
        || pointer.is_null()
        || length == 0
    {
        return None;
    }
    let value = std::slice::from_raw_parts(pointer.cast::<u16>(), length as usize);
    let end = value
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(value.len());
    let value = String::from_utf16_lossy(&value[..end]).trim().to_string();
    (!value.is_empty()).then_some(value)
}

#[cfg(windows)]
fn wide_string(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "savelink-loader-identity-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn resolves_cold_client_loader_real_executable() {
        let root = temp_root("valid");
        let loader = root.join("steamclient_loader.exe");
        let game = root.join("Kingdom Rush Genesis.exe");
        fs::write(&loader, b"loader").unwrap();
        fs::write(&game, b"game").unwrap();
        fs::write(
            root.join(COLD_CLIENT_LOADER_CONFIG),
            b"[SteamClient]\nExe=Kingdom Rush Genesis.exe\nAppId=2115380\n",
        )
        .unwrap();

        let identity = resolve_loader_program_identity(&loader, &root)
            .unwrap()
            .unwrap();

        assert_eq!(identity.executable_path, fs::canonicalize(&game).unwrap());
        assert!(identity
            .identity_hints
            .iter()
            .any(|hint| hint == "Kingdom Rush Genesis"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn supports_utf16_and_quoted_executable_values() {
        let root = temp_root("utf16");
        let loader = root.join("steamclient_loader.exe");
        let bin = root.join("bin");
        let game = bin.join("Real Game.exe");
        fs::create_dir_all(&bin).unwrap();
        fs::write(&loader, b"loader").unwrap();
        fs::write(&game, b"game").unwrap();
        let content = "[steamclient]\r\nexe=\"bin\\\\Real Game.exe\"\r\n";
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend(content.encode_utf16().flat_map(u16::to_le_bytes));
        fs::write(root.join(COLD_CLIENT_LOADER_CONFIG), bytes).unwrap();

        let identity = resolve_loader_program_identity(&loader, &root)
            .unwrap()
            .unwrap();

        assert_eq!(identity.executable_path, fs::canonicalize(&game).unwrap());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_executable_paths_that_escape_install_directory() {
        let root = temp_root("escape");
        let install = root.join("install");
        fs::create_dir_all(&install).unwrap();
        let loader = install.join("steamclient_loader.exe");
        let outside = root.join("outside.exe");
        fs::write(&loader, b"loader").unwrap();
        fs::write(&outside, b"outside").unwrap();
        fs::write(
            install.join(COLD_CLIENT_LOADER_CONFIG),
            b"[SteamClient]\nExe=..\\outside.exe\n",
        )
        .unwrap();

        let error = resolve_loader_program_identity(&loader, &install).unwrap_err();

        assert!(matches!(
            error,
            LoaderIdentityError::UnsafeExecutablePath(_)
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_config_keeps_existing_behavior() {
        let root = temp_root("missing");
        let loader = root.join("game.exe");
        fs::write(&loader, b"game").unwrap();

        assert_eq!(
            resolve_loader_program_identity(&loader, &root).unwrap(),
            None
        );
        let _ = fs::remove_dir_all(root);
    }
}
