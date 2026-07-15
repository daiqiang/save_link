//! 单快照 zip 的创建、SHA-256 校验和安全解压。

use crate::model::ScanResult;
use crate::scan;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudArchiveError {
    SourceInvalid(String),
    ArchiveSizeMismatch,
    ArchiveHashMismatch,
    UnsafeEntry(String),
    ContentMismatch,
    Io(String),
}

impl CloudArchiveError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::SourceInvalid(_) => "archive_source_invalid",
            Self::ArchiveSizeMismatch => "archive_size_mismatch",
            Self::ArchiveHashMismatch => "archive_hash_mismatch",
            Self::UnsafeEntry(_) => "unsafe_archive_entry",
            Self::ContentMismatch => "snapshot_content_mismatch",
            Self::Io(_) => "archive_io_failed",
        }
    }
}

impl fmt::Display for CloudArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceInvalid(message) => write!(f, "快照打包源无效: {message}"),
            Self::ArchiveSizeMismatch => write!(f, "zip 文件大小与云端 .ok 不一致"),
            Self::ArchiveHashMismatch => write!(f, "zip SHA-256 与云端 .ok 不一致"),
            Self::UnsafeEntry(path) => write!(f, "zip 包含不安全路径或文件类型: {path}"),
            Self::ContentMismatch => write!(f, "解压后的快照内容与云端 .ok 不一致"),
            Self::Io(message) => write!(f, "zip 处理失败: {message}"),
        }
    }
}

impl std::error::Error for CloudArchiveError {}

pub type CloudArchiveResult<T> = std::result::Result<T, CloudArchiveError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveInfo {
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotContentExpectation {
    pub file_count: u64,
    pub total_size: u64,
    pub content_hash: String,
}

pub trait CloudArchiveCodec: Send + Sync {
    fn create_archive(
        &self,
        source_dir: &Path,
        archive_path: &Path,
    ) -> CloudArchiveResult<ArchiveInfo>;

    fn verify_archive(
        &self,
        archive_path: &Path,
        expected_size: u64,
        expected_sha256: &str,
    ) -> CloudArchiveResult<()>;

    fn extract_verified(
        &self,
        archive_path: &Path,
        target_dir: &Path,
        expected: &SnapshotContentExpectation,
    ) -> CloudArchiveResult<ScanResult>;
}

pub struct ZipCloudArchiveCodec;

impl ZipCloudArchiveCodec {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ZipCloudArchiveCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl CloudArchiveCodec for ZipCloudArchiveCodec {
    fn create_archive(
        &self,
        source_dir: &Path,
        archive_path: &Path,
    ) -> CloudArchiveResult<ArchiveInfo> {
        if !source_dir.is_dir() {
            return Err(CloudArchiveError::SourceInvalid(
                source_dir.display().to_string(),
            ));
        }
        let files = collect_source_files(source_dir)?;
        if let Some(parent) = archive_path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        if archive_path.exists() {
            fs::remove_file(archive_path).map_err(io_error)?;
        }

        let archive_file = File::create(archive_path).map_err(io_error)?;
        let mut writer = ZipWriter::new(archive_file);
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o600);

        for (name, path) in files {
            writer.start_file(name, options).map_err(zip_error)?;
            let mut input = File::open(path).map_err(io_error)?;
            io::copy(&mut input, &mut writer).map_err(io_error)?;
        }
        writer.finish().map_err(zip_error)?;

        let size = fs::metadata(archive_path).map_err(io_error)?.len();
        let sha256 = sha256_file(archive_path)?;
        Ok(ArchiveInfo { size, sha256 })
    }

    fn verify_archive(
        &self,
        archive_path: &Path,
        expected_size: u64,
        expected_sha256: &str,
    ) -> CloudArchiveResult<()> {
        let actual_size = fs::metadata(archive_path).map_err(io_error)?.len();
        if actual_size != expected_size {
            return Err(CloudArchiveError::ArchiveSizeMismatch);
        }
        if sha256_file(archive_path)? != expected_sha256 {
            return Err(CloudArchiveError::ArchiveHashMismatch);
        }
        Ok(())
    }

    fn extract_verified(
        &self,
        archive_path: &Path,
        target_dir: &Path,
        expected: &SnapshotContentExpectation,
    ) -> CloudArchiveResult<ScanResult> {
        if target_dir.exists() {
            return Err(CloudArchiveError::Io(format!(
                "解压目标已存在: {}",
                target_dir.display()
            )));
        }
        fs::create_dir_all(target_dir).map_err(io_error)?;
        let result = extract_inner(archive_path, target_dir, expected);
        if result.is_err() {
            let _ = fs::remove_dir_all(target_dir);
        }
        result
    }
}

fn collect_source_files(root: &Path) -> CloudArchiveResult<Vec<(String, PathBuf)>> {
    fn walk(
        root: &Path,
        current: &Path,
        out: &mut Vec<(String, PathBuf)>,
        seen: &mut HashSet<String>,
    ) -> CloudArchiveResult<()> {
        for entry in fs::read_dir(current).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(io_error)?;
            if file_type.is_symlink() {
                return Err(CloudArchiveError::SourceInvalid(path.display().to_string()));
            }
            if file_type.is_dir() {
                walk(root, &path, out, seen)?;
                continue;
            }
            if !file_type.is_file() {
                return Err(CloudArchiveError::SourceInvalid(path.display().to_string()));
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|_| CloudArchiveError::SourceInvalid(path.display().to_string()))?;
            let name = relative
                .components()
                .map(|component| component.as_os_str().to_str())
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| CloudArchiveError::SourceInvalid(path.display().to_string()))?
                .join("/");
            let normalized = validate_archive_name(&name, false)?;
            let collision_key = normalized.to_lowercase();
            if !seen.insert(collision_key) {
                return Err(CloudArchiveError::SourceInvalid(format!(
                    "路径大小写冲突: {name}"
                )));
            }
            out.push((normalized, path));
        }
        Ok(())
    }

    let mut files = Vec::new();
    let mut seen = HashSet::new();
    walk(root, root, &mut files, &mut seen)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

fn extract_inner(
    archive_path: &Path,
    target_dir: &Path,
    expected: &SnapshotContentExpectation,
) -> CloudArchiveResult<ScanResult> {
    let archive_file = File::open(archive_path).map_err(io_error)?;
    let mut archive = ZipArchive::new(archive_file).map_err(zip_error)?;
    let mut seen = HashSet::new();
    let mut file_count = 0u64;
    let mut total_size = 0u64;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(zip_error)?;
        let is_directory = entry.is_dir();
        if is_symlink_or_special(&entry) {
            return Err(CloudArchiveError::UnsafeEntry(entry.name().to_string()));
        }
        let normalized = validate_archive_name(entry.name(), is_directory)?;
        let collision_key = normalized.to_lowercase();
        if !seen.insert(collision_key) {
            return Err(CloudArchiveError::UnsafeEntry(entry.name().to_string()));
        }
        let target = target_dir.join(Path::new(&normalized));
        if is_directory {
            fs::create_dir_all(target).map_err(io_error)?;
            continue;
        }

        file_count += 1;
        if file_count > expected.file_count {
            return Err(CloudArchiveError::ContentMismatch);
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let remaining = expected.total_size.saturating_sub(total_size);
        let mut output = File::create(target).map_err(io_error)?;
        let copied = io::copy(
            &mut (&mut entry).take(remaining.saturating_add(1)),
            &mut output,
        )
        .map_err(io_error)?;
        if copied > remaining {
            return Err(CloudArchiveError::ContentMismatch);
        }
        total_size += copied;
    }

    if file_count != expected.file_count || total_size != expected.total_size {
        return Err(CloudArchiveError::ContentMismatch);
    }
    let actual = scan::fingerprint_dir(target_dir)
        .map_err(|error| CloudArchiveError::Io(error.to_string()))?;
    if actual.file_count != expected.file_count
        || actual.total_size != expected.total_size
        || actual.content_hash != expected.content_hash
    {
        return Err(CloudArchiveError::ContentMismatch);
    }
    Ok(actual)
}

fn validate_archive_name(name: &str, is_directory: bool) -> CloudArchiveResult<String> {
    if name.is_empty()
        || name.starts_with('/')
        || name.starts_with('\\')
        || name.contains('\\')
        || name.contains('\0')
        || name.contains(':')
    {
        return Err(CloudArchiveError::UnsafeEntry(name.to_string()));
    }
    let trimmed = if is_directory {
        name.strip_suffix('/').unwrap_or(name)
    } else {
        name
    };
    if trimmed.is_empty() {
        return Err(CloudArchiveError::UnsafeEntry(name.to_string()));
    }

    let mut normalized = Vec::new();
    for segment in trimmed.split('/') {
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || segment.ends_with('.')
            || segment.ends_with(' ')
            || is_windows_reserved_name(segment)
        {
            return Err(CloudArchiveError::UnsafeEntry(name.to_string()));
        }
        normalized.push(segment);
    }
    Ok(normalized.join("/"))
}

fn is_windows_reserved_name(segment: &str) -> bool {
    let base = segment
        .split('.')
        .next()
        .unwrap_or(segment)
        .to_ascii_uppercase();
    matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && matches!(base.as_bytes()[3], b'1'..=b'9'))
}

fn is_symlink_or_special(entry: &zip::read::ZipFile<'_>) -> bool {
    let Some(mode) = entry.unix_mode() else {
        return false;
    };
    let kind = mode & 0o170000;
    kind != 0 && kind != 0o040000 && kind != 0o100000
}

fn sha256_file(path: &Path) -> CloudArchiveResult<String> {
    let mut file = File::open(path).map_err(io_error)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn io_error(error: io::Error) -> CloudArchiveError {
    CloudArchiveError::Io(error.to_string())
}

fn zip_error(error: zip::result::ZipError) -> CloudArchiveError {
    CloudArchiveError::Io(error.to_string())
}
