//! DeSmuME 0.9.x 绿色版发现。
//!
//! DeSmuME 以 ROM 文件名作为 `.dsv` 文件名，并让所有游戏共用 `Battery`
//! 目录。本模块只负责只读发现和 ROM 身份计算；快照层通过 `SaveSource::Files`
//! 保证每个游戏只保护自己的 `.dsv`。

use crate::model::{
    EmulatorGameIdentity, EmulatorLocalBinding, RomIdentity, SaveFileMapping, SaveSource,
};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub const DESMUME_EMULATOR_ID: &str = "desmume";
const ROM_HASH_BUFFER_SIZE: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesmumeDiscoveredGame {
    pub name: String,
    pub rom_path: PathBuf,
    pub save_path: PathBuf,
    pub has_save: bool,
    pub identity: EmulatorGameIdentity,
    pub binding: EmulatorLocalBinding,
}

impl DesmumeDiscoveredGame {
    pub fn save_source(&self) -> DesmumeDiscoveryResult<SaveSource> {
        let root = self
            .save_path
            .parent()
            .ok_or_else(|| DesmumeDiscoveryError::Io("存档文件没有父目录".into()))?;
        let local_name = self
            .save_path
            .file_name()
            .ok_or_else(|| DesmumeDiscoveryError::Io("存档文件没有文件名".into()))?;
        Ok(SaveSource::Files {
            root: root.to_path_buf(),
            files: vec![SaveFileMapping {
                local_relative_path: PathBuf::from(local_name),
                snapshot_relative_path: PathBuf::from("save.dsv"),
            }],
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RomMatch {
    Exact,
    Possible,
    None,
}

pub fn compare_rom_identity(
    expected: &EmulatorGameIdentity,
    actual: &EmulatorGameIdentity,
) -> RomMatch {
    if expected.emulator != actual.emulator {
        return RomMatch::None;
    }
    if expected.rom.sha256 == actual.rom.sha256 {
        return RomMatch::Exact;
    }
    if !expected.rom.game_code.is_empty()
        && expected.rom.game_code == actual.rom.game_code
        && expected.rom.header_title == actual.rom.header_title
    {
        return RomMatch::Possible;
    }
    RomMatch::None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesmumeDiscoveryReport {
    pub emulator_root: PathBuf,
    pub configured_rom_root: Option<PathBuf>,
    pub rom_root: Option<PathBuf>,
    pub configured_rom_root_missing: bool,
    pub battery_dir: PathBuf,
    pub games: Vec<DesmumeDiscoveredGame>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesmumeDiscoveryError {
    EmulatorDirectoryMissing(PathBuf),
    ExecutableMissing(PathBuf),
    RomDirectoryMissing(PathBuf),
    Io(String),
    InvalidRom(PathBuf),
}

impl fmt::Display for DesmumeDiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmulatorDirectoryMissing(path) => {
                write!(formatter, "DeSmuME 目录不存在：{}", path.display())
            }
            Self::ExecutableMissing(path) => {
                write!(
                    formatter,
                    "所选目录中没有找到 DeSmuME 可执行文件：{}",
                    path.display()
                )
            }
            Self::RomDirectoryMissing(path) => {
                write!(formatter, "ROM 目录不存在：{}", path.display())
            }
            Self::Io(message) => write!(formatter, "读取 DeSmuME 数据失败：{message}"),
            Self::InvalidRom(path) => write!(formatter, "NDS ROM 文件头不完整：{}", path.display()),
        }
    }
}

impl std::error::Error for DesmumeDiscoveryError {}

pub type DesmumeDiscoveryResult<T> = std::result::Result<T, DesmumeDiscoveryError>;

pub struct DesmumeDiscoveryService;

impl DesmumeDiscoveryService {
    pub fn scan(
        emulator_root: &Path,
        explicit_rom_root: Option<&Path>,
    ) -> DesmumeDiscoveryResult<DesmumeDiscoveryReport> {
        Self::scan_with_cache(emulator_root, explicit_rom_root, &[])
    }

    pub fn scan_with_cache(
        emulator_root: &Path,
        explicit_rom_root: Option<&Path>,
        cached_bindings: &[EmulatorLocalBinding],
    ) -> DesmumeDiscoveryResult<DesmumeDiscoveryReport> {
        if !emulator_root.is_dir() {
            return Err(DesmumeDiscoveryError::EmulatorDirectoryMissing(
                emulator_root.to_path_buf(),
            ));
        }
        if !contains_desmume_executable(emulator_root)? {
            return Err(DesmumeDiscoveryError::ExecutableMissing(
                emulator_root.to_path_buf(),
            ));
        }

        let settings = read_path_settings(&emulator_root.join("desmume.ini"))?;
        let configured_rom_root = settings
            .get("roms")
            .filter(|value| !value.trim().is_empty())
            .map(|value| resolve_configured_path(emulator_root, value));
        let configured_rom_root_missing = configured_rom_root
            .as_ref()
            .is_some_and(|path| !path.is_dir());
        let rom_root = if let Some(explicit) = explicit_rom_root {
            if !explicit.is_dir() {
                return Err(DesmumeDiscoveryError::RomDirectoryMissing(
                    explicit.to_path_buf(),
                ));
            }
            Some(explicit.to_path_buf())
        } else {
            configured_rom_root
                .as_ref()
                .filter(|path| path.is_dir())
                .cloned()
        };

        let battery_dir = settings
            .get("battery")
            .filter(|value| !value.trim().is_empty())
            .map(|value| resolve_configured_path(emulator_root, value))
            .unwrap_or_else(|| emulator_root.join("Battery"));
        let mut games = Vec::new();
        if let Some(rom_root) = &rom_root {
            let mut roms = Vec::new();
            collect_nds_roms(rom_root, &mut roms)?;
            roms.sort_by(|left, right| {
                left.to_string_lossy()
                    .to_ascii_lowercase()
                    .cmp(&right.to_string_lossy().to_ascii_lowercase())
            });
            for rom_path in roms {
                let (identity, binding) =
                    inspect_rom_with_cache(emulator_root, &rom_path, cached_bindings)?;
                let stem = rom_path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| DesmumeDiscoveryError::InvalidRom(rom_path.clone()))?;
                let expected_name = format!("{stem}.dsv");
                let save_path = find_file_case_insensitive(&battery_dir, &expected_name)?
                    .unwrap_or_else(|| battery_dir.join(expected_name));
                games.push(DesmumeDiscoveredGame {
                    name: stem.to_string(),
                    rom_path,
                    has_save: save_path.is_file(),
                    save_path,
                    identity,
                    binding,
                });
            }
        }

        Ok(DesmumeDiscoveryReport {
            emulator_root: emulator_root.to_path_buf(),
            configured_rom_root,
            rom_root,
            configured_rom_root_missing,
            battery_dir,
            games,
        })
    }
}

fn inspect_rom_with_cache(
    emulator_root: &Path,
    rom_path: &Path,
    cached_bindings: &[EmulatorLocalBinding],
) -> DesmumeDiscoveryResult<(EmulatorGameIdentity, EmulatorLocalBinding)> {
    let metadata = fs::metadata(rom_path).map_err(io_error)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis() as u64);
    if let Some(cached) = cached_bindings.iter().find(|binding| {
        same_path(&binding.rom_path, rom_path)
            && binding.rom_size == metadata.len()
            && binding.rom_modified_unix_ms == modified
    }) {
        let mut binding = cached.clone();
        binding.emulator_root = emulator_root.to_path_buf();
        binding.rom_path = rom_path.to_path_buf();
        return Ok((
            EmulatorGameIdentity {
                emulator: DESMUME_EMULATOR_ID.into(),
                rom: binding.local_rom.clone(),
            },
            binding,
        ));
    }
    inspect_rom(emulator_root, rom_path)
}

pub fn inspect_rom(
    emulator_root: &Path,
    rom_path: &Path,
) -> DesmumeDiscoveryResult<(EmulatorGameIdentity, EmulatorLocalBinding)> {
    let mut file = fs::File::open(rom_path).map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    let mut header = [0u8; 18];
    file.read_exact(&mut header)
        .map_err(|_| DesmumeDiscoveryError::InvalidRom(rom_path.to_path_buf()))?;
    let header_title = ascii_field(&header[0..12]);
    let game_code = ascii_field(&header[12..16]);
    if game_code.len() != 4 {
        return Err(DesmumeDiscoveryError::InvalidRom(rom_path.to_path_buf()));
    }

    file.seek(SeekFrom::Start(0)).map_err(io_error)?;
    let mut hasher = Sha256::new();
    // Tauri commands can run on worker threads with a relatively small stack. Keep the
    // 1 MiB streaming buffer on the heap so scanning a ROM cannot overflow that stack.
    let mut buffer = vec![0u8; ROM_HASH_BUFFER_SIZE];
    loop {
        let count = file.read(&mut buffer).map_err(io_error)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let sha256 = format!("{:x}", hasher.finalize());
    let file_name = rom_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| DesmumeDiscoveryError::InvalidRom(rom_path.to_path_buf()))?
        .to_string();
    let rom = RomIdentity {
        file_name,
        sha256,
        header_title,
        game_code,
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis() as u64);
    Ok((
        EmulatorGameIdentity {
            emulator: DESMUME_EMULATOR_ID.into(),
            rom: rom.clone(),
        },
        EmulatorLocalBinding {
            emulator_root: emulator_root.to_path_buf(),
            rom_path: rom_path.to_path_buf(),
            rom_size: metadata.len(),
            rom_modified_unix_ms: modified,
            local_rom: rom,
        },
    ))
}

fn contains_desmume_executable(root: &Path) -> DesmumeDiscoveryResult<bool> {
    for entry in fs::read_dir(root).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        if !entry.file_type().map_err(io_error)?.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.starts_with("desmume") && name.ends_with(".exe") {
            return Ok(true);
        }
    }
    Ok(false)
}

fn read_path_settings(path: &Path) -> DesmumeDiscoveryResult<HashMap<String, String>> {
    if !path.is_file() {
        return Ok(HashMap::new());
    }
    let bytes = fs::read(path).map_err(io_error)?;
    let content = String::from_utf8_lossy(&bytes);
    let mut in_path_settings = false;
    let mut settings = HashMap::new();
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_path_settings = line[1..line.len() - 1].eq_ignore_ascii_case("PathSettings");
            continue;
        }
        if !in_path_settings || line.is_empty() || line.starts_with([';', '#']) {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            settings.insert(
                key.trim().to_ascii_lowercase(),
                value.trim().trim_matches('"').to_string(),
            );
        }
    }
    Ok(settings)
}

fn resolve_configured_path(emulator_root: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        emulator_root.join(path)
    }
}

fn collect_nds_roms(root: &Path, output: &mut Vec<PathBuf>) -> DesmumeDiscoveryResult<()> {
    // ROM 目录可能包含 junction 或目录链接。使用显式待处理队列和 canonical 路径去重，
    // 避免递归遍历环导致桌面进程栈溢出，同时仍然保持扫描只读。
    let mut pending = vec![root.to_path_buf()];
    let mut visited = HashSet::new();
    while let Some(directory) = pending.pop() {
        let canonical = fs::canonicalize(&directory).map_err(io_error)?;
        let visit_key = canonical
            .to_string_lossy()
            .replace('/', "\\")
            .to_ascii_lowercase();
        if !visited.insert(visit_key) {
            continue;
        }

        for entry in fs::read_dir(&directory).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let file_type = entry.file_type().map_err(io_error)?;
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("nds"))
            {
                output.push(path);
            }
        }
    }
    Ok(())
}

fn find_file_case_insensitive(
    root: &Path,
    expected_name: &str,
) -> DesmumeDiscoveryResult<Option<PathBuf>> {
    if !root.is_dir() {
        return Ok(None);
    }
    for entry in fs::read_dir(root).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        if entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(expected_name)
        {
            return Ok(Some(entry.path()));
        }
    }
    Ok(None)
}

fn ascii_field(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

fn same_path(left: &Path, right: &Path) -> bool {
    left.to_string_lossy()
        .replace('/', "\\")
        .eq_ignore_ascii_case(&right.to_string_lossy().replace('/', "\\"))
}

fn io_error(error: std::io::Error) -> DesmumeDiscoveryError {
    DesmumeDiscoveryError::Io(error.to_string())
}
