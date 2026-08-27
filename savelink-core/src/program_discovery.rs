//! 从用户指定的游戏目录、可执行文件或 Windows 快捷方式识别游戏存档。
//!
//! 这里不扫描整块磁盘。程序身份来自安装目录内的 Steam AppID 配置，
//! 缺失时再使用快捷方式、可执行文件和目录名称保守匹配 Ludusavi Manifest。

use crate::steam_discovery::{
    identities_match, ProgramManifestMatchKind, SteamDiscoveryError, SteamDiscoveryService,
};
use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_DIRECTORY_SCAN_DEPTH: usize = 3;
const MAX_SCANNED_DIRECTORIES: usize = 512;
const MAX_IDENTITY_HINTS: usize = 64;
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramSelectionKind {
    Directory,
    Executable,
    Shortcut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramMatchKind {
    AppId,
    Name,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramDiscoveredGame {
    pub name: String,
    pub app_id: u32,
    pub match_kind: ProgramMatchKind,
    pub save_paths: Vec<PathBuf>,
    pub config_paths: Vec<PathBuf>,
    pub current_system_unresolved_rules: usize,
    pub other_environment_rules: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramDiscoveryReport {
    pub selected_path: PathBuf,
    pub selection_kind: ProgramSelectionKind,
    pub resolved_program_path: Option<PathBuf>,
    pub install_dir: PathBuf,
    pub detected_app_id: Option<u32>,
    pub app_id_source: Option<PathBuf>,
    pub ignored_app_id_game_names: Vec<String>,
    pub identity_hints: Vec<String>,
    pub games: Vec<ProgramDiscoveredGame>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgramDiscoveryError {
    SelectionMissing(PathBuf),
    UnsupportedSelection(PathBuf),
    InvalidProgramPath(PathBuf),
    ShortcutUnavailable,
    ShortcutRead(String),
    Io(String),
    Manifest(SteamDiscoveryError),
}

impl fmt::Display for ProgramDiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SelectionMissing(path) => write!(formatter, "所选路径不存在：{}", path.display()),
            Self::UnsupportedSelection(path) => write!(
                formatter,
                "请选择游戏目录、EXE 或 Windows 快捷方式：{}",
                path.display()
            ),
            Self::InvalidProgramPath(path) => {
                write!(formatter, "无法确定游戏所在目录：{}", path.display())
            }
            Self::ShortcutUnavailable => write!(formatter, "当前系统不支持读取 Windows 快捷方式"),
            Self::ShortcutRead(message) => write!(formatter, "读取快捷方式失败：{message}"),
            Self::Io(message) => write!(formatter, "读取游戏目录失败：{message}"),
            Self::Manifest(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for ProgramDiscoveryError {}

impl From<SteamDiscoveryError> for ProgramDiscoveryError {
    fn from(value: SteamDiscoveryError) -> Self {
        Self::Manifest(value)
    }
}

pub type ProgramDiscoveryResult<T> = std::result::Result<T, ProgramDiscoveryError>;

pub struct ProgramDiscoveryService {
    manifest_database: PathBuf,
}

impl ProgramDiscoveryService {
    pub fn new(manifest_database: impl Into<PathBuf>) -> Self {
        Self {
            manifest_database: manifest_database.into(),
        }
    }

    pub fn scan(&self, selected_path: &Path) -> ProgramDiscoveryResult<ProgramDiscoveryReport> {
        let selection = resolve_selection(selected_path)?;
        let mut install_dir = selection.install_dir;
        let mut app_id = None;
        let mut app_id_source = None;

        let nearby_app_id = find_app_id_near(&install_dir, selection.program_path.is_some())?;
        if let Some(found) = nearby_app_id {
            app_id = Some(found.app_id);
            app_id_source = Some(found.source);
            if selection.program_path.is_some() {
                install_dir = found.install_dir;
            }
        }

        let installation_scan = scan_installation(&install_dir)?;
        if app_id.is_none() {
            if let Some(found) = installation_scan.app_id {
                app_id = Some(found.app_id);
                app_id_source = Some(found.source);
            }
        }

        let mut trusted_identity_hints = Vec::new();
        push_hint(&mut trusted_identity_hints, selected_path.file_stem());
        if let Some(program_path) = selection.program_path.as_deref() {
            push_hint(&mut trusted_identity_hints, program_path.file_stem());
        }
        push_hint(&mut trusted_identity_hints, install_dir.file_name());

        let mut hints = trusted_identity_hints.clone();
        for hint in installation_scan.executable_hints {
            push_hint_text(&mut hints, hint);
        }

        let discovery = SteamDiscoveryService::new(&self.manifest_database);
        let mut manifest_matches =
            discovery.scan_program_installation(&install_dir, &hints, app_id)?;
        let mut ignored_app_id_game_names = Vec::new();
        if manifest_matches
            .iter()
            .any(|game| game.match_kind == ProgramManifestMatchKind::AppId)
        {
            manifest_matches.retain(|game| {
                let corroborated = trusted_identity_hints
                    .iter()
                    .any(|hint| identities_match(&game.name, hint));
                if !corroborated {
                    ignored_app_id_game_names.push(game.name.clone());
                }
                corroborated
            });
            ignored_app_id_game_names.sort_by_key(|name| name.to_lowercase());
            ignored_app_id_game_names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));

            if manifest_matches.is_empty() {
                manifest_matches =
                    discovery.scan_program_installation(&install_dir, &hints, None)?;
            }
        }

        let games = manifest_matches
            .into_iter()
            .map(|game| ProgramDiscoveredGame {
                name: game.name,
                app_id: game.app_id,
                match_kind: match game.match_kind {
                    ProgramManifestMatchKind::AppId => ProgramMatchKind::AppId,
                    ProgramManifestMatchKind::Name => ProgramMatchKind::Name,
                },
                save_paths: game.save_paths,
                config_paths: game.config_paths,
                current_system_unresolved_rules: game.current_system_unresolved_rules,
                other_environment_rules: game.other_environment_rules,
            })
            .collect();

        Ok(ProgramDiscoveryReport {
            selected_path: selected_path.to_path_buf(),
            selection_kind: selection.kind,
            resolved_program_path: selection.program_path,
            install_dir,
            detected_app_id: app_id,
            app_id_source,
            ignored_app_id_game_names,
            identity_hints: hints,
            games,
        })
    }
}

struct ResolvedSelection {
    kind: ProgramSelectionKind,
    program_path: Option<PathBuf>,
    install_dir: PathBuf,
}

fn resolve_selection(selected_path: &Path) -> ProgramDiscoveryResult<ResolvedSelection> {
    if !selected_path.exists() {
        return Err(ProgramDiscoveryError::SelectionMissing(
            selected_path.to_path_buf(),
        ));
    }
    if selected_path.is_dir() {
        return Ok(ResolvedSelection {
            kind: ProgramSelectionKind::Directory,
            program_path: None,
            install_dir: selected_path.to_path_buf(),
        });
    }

    let extension = selected_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let (kind, program_path) = match extension.as_str() {
        "exe" => (
            ProgramSelectionKind::Executable,
            selected_path.to_path_buf(),
        ),
        "lnk" => (
            ProgramSelectionKind::Shortcut,
            resolve_shortcut(selected_path)?,
        ),
        _ => {
            return Err(ProgramDiscoveryError::UnsupportedSelection(
                selected_path.to_path_buf(),
            ))
        }
    };
    if !program_path.is_file() {
        return Err(ProgramDiscoveryError::SelectionMissing(program_path));
    }
    let parent = program_path
        .parent()
        .ok_or_else(|| ProgramDiscoveryError::InvalidProgramPath(program_path.clone()))?;
    let install_dir = infer_install_dir(parent);
    Ok(ResolvedSelection {
        kind,
        program_path: Some(program_path),
        install_dir,
    })
}

fn infer_install_dir(start: &Path) -> PathBuf {
    let mut current = start.to_path_buf();
    while current
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(is_generic_binary_directory)
    {
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent.to_path_buf();
    }
    current
}

fn is_generic_binary_directory(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "bin" | "binaries" | "win32" | "win64" | "x86" | "x64" | "shipping"
    )
}

#[derive(Debug)]
struct AppIdFinding {
    app_id: u32,
    source: PathBuf,
    install_dir: PathBuf,
}

fn find_app_id_near(
    start: &Path,
    include_ancestors: bool,
) -> ProgramDiscoveryResult<Option<AppIdFinding>> {
    let mut current = Some(start);
    let mut checked = 0;
    while let Some(directory) = current {
        if let Some((app_id, source)) = read_app_id_in_directory(directory)? {
            return Ok(Some(AppIdFinding {
                app_id,
                source,
                install_dir: directory.to_path_buf(),
            }));
        }
        checked += 1;
        if !include_ancestors || checked >= 5 {
            break;
        }
        current = directory.parent();
    }
    Ok(None)
}

#[derive(Default)]
struct InstallationScan {
    app_id: Option<AppIdFinding>,
    executable_hints: Vec<String>,
}

fn scan_installation(root: &Path) -> ProgramDiscoveryResult<InstallationScan> {
    let mut result = InstallationScan::default();
    let mut queue = VecDeque::from([(root.to_path_buf(), 0usize)]);
    let mut visited = HashSet::new();

    while let Some((directory, depth)) = queue.pop_front() {
        if visited.len() >= MAX_SCANNED_DIRECTORIES {
            break;
        }
        let canonical = directory
            .canonicalize()
            .unwrap_or_else(|_| directory.clone());
        if !visited.insert(normalized_path(&canonical)) {
            continue;
        }

        if result.app_id.is_none() {
            if let Some((app_id, source)) = read_app_id_in_directory(&directory)? {
                result.app_id = Some(AppIdFinding {
                    app_id,
                    source,
                    install_dir: root.to_path_buf(),
                });
            }
        }

        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotFound
                ) =>
            {
                continue
            }
            Err(error) => return Err(io_error(error)),
        };
        let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase());
        for entry in entries {
            let file_type = entry.file_type().map_err(io_error)?;
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_file()
                && path
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case("exe"))
                && result.executable_hints.len() < MAX_IDENTITY_HINTS
            {
                if let Some(stem) = path.file_stem().and_then(|value| value.to_str()) {
                    result.executable_hints.push(stem.to_string());
                }
            } else if file_type.is_dir() && depth < MAX_DIRECTORY_SCAN_DEPTH {
                queue.push_back((path, depth + 1));
            }
        }
    }
    Ok(result)
}

fn read_app_id_in_directory(directory: &Path) -> ProgramDiscoveryResult<Option<(u32, PathBuf)>> {
    let mut candidates = Vec::new();
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return Ok(None),
        Err(error) => return Err(io_error(error)),
    };
    for entry in entries {
        let entry = entry.map_err(io_error)?;
        if !entry.file_type().map_err(io_error)?.is_file() {
            continue;
        }
        let lower = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if is_app_id_file(&lower) {
            candidates.push(entry.path());
        }
    }
    let steam_settings = directory.join("steam_settings").join("steam_appid.txt");
    if steam_settings.is_file() {
        candidates.push(steam_settings);
    }
    candidates.sort_by_key(|path| {
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        (name != "steam_appid.txt", name)
    });
    for path in candidates {
        if let Some(app_id) = parse_app_id_file(&path)? {
            return Ok(Some((app_id, path)));
        }
    }
    Ok(None)
}

fn is_app_id_file(file_name: &str) -> bool {
    matches!(
        file_name,
        "steam_appid.txt"
            | "steam_emu.ini"
            | "smartsteamemu.ini"
            | "coldclientloader.ini"
            | "configs.app.ini"
    )
}

fn parse_app_id_file(path: &Path) -> ProgramDiscoveryResult<Option<u32>> {
    let metadata = fs::metadata(path).map_err(io_error)?;
    if metadata.len() > MAX_CONFIG_BYTES {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(io_error)?;
    let content = String::from_utf8_lossy(&bytes);
    let plain_text = path
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("steam_appid.txt"));
    if plain_text {
        return Ok(first_positive_u32(&content));
    }
    for line in content.lines() {
        let line = line.split([';', '#']).next().unwrap_or_default().trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>();
        if matches!(
            key.as_str(),
            "appid" | "applicationid" | "steamappid" | "appsteamid"
        ) {
            if let Some(app_id) = first_positive_u32(value) {
                return Ok(Some(app_id));
            }
        }
    }
    Ok(None)
}

fn first_positive_u32(value: &str) -> Option<u32> {
    value
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .find_map(|part| part.parse::<u32>().ok().filter(|value| *value > 0))
}

fn push_hint(hints: &mut Vec<String>, value: Option<&std::ffi::OsStr>) {
    if let Some(value) = value.and_then(|value| value.to_str()) {
        push_hint_text(hints, value.to_string());
    }
}

fn push_hint_text(hints: &mut Vec<String>, value: String) {
    let value = value.trim();
    if value.is_empty() || is_generic_identity_hint(value) {
        return;
    }
    if !hints
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(value))
    {
        hints.push(value.to_string());
    }
}

fn is_generic_identity_hint(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "app"
            | "game"
            | "game64"
            | "launcher"
            | "start"
            | "setup"
            | "uninstall"
            | "unins000"
            | "crashreportclient"
            | "unitycrashhandler64"
    )
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase()
}

fn io_error(error: std::io::Error) -> ProgramDiscoveryError {
    ProgramDiscoveryError::Io(error.to_string())
}

#[cfg(windows)]
fn resolve_shortcut(path: &Path) -> ProgramDiscoveryResult<PathBuf> {
    let path = path.to_path_buf();
    std::thread::spawn(move || resolve_shortcut_on_sta(&path))
        .join()
        .map_err(|_| ProgramDiscoveryError::ShortcutRead("快捷方式解析线程异常结束".into()))?
}

#[cfg(windows)]
fn resolve_shortcut_on_sta(path: &Path) -> ProgramDiscoveryResult<PathBuf> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{Interface, PCWSTR};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED, STGM_READ,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink, SLGP_RAWPATH};

    struct ComGuard;
    impl Drop for ComGuard {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|error| ProgramDiscoveryError::ShortcutRead(error.to_string()))?;
        let _guard = ComGuard;
        let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .map_err(|error| ProgramDiscoveryError::ShortcutRead(error.to_string()))?;
        let persist: IPersistFile = shell_link
            .cast()
            .map_err(|error| ProgramDiscoveryError::ShortcutRead(error.to_string()))?;
        let wide_path = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        persist
            .Load(PCWSTR(wide_path.as_ptr()), STGM_READ)
            .map_err(|error| ProgramDiscoveryError::ShortcutRead(error.to_string()))?;

        let mut target = vec![0u16; 32_768];
        shell_link
            .GetPath(&mut target, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)
            .map_err(|error| ProgramDiscoveryError::ShortcutRead(error.to_string()))?;
        let length = target
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(target.len());
        if length == 0 {
            return Err(ProgramDiscoveryError::ShortcutRead(
                "快捷方式没有文件目标".into(),
            ));
        }
        Ok(PathBuf::from(String::from_utf16_lossy(&target[..length])))
    }
}

#[cfg(not(windows))]
fn resolve_shortcut(_path: &Path) -> ProgramDiscoveryResult<PathBuf> {
    Err(ProgramDiscoveryError::ShortcutUnavailable)
}
