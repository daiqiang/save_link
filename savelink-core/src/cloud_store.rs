//! 云对象存储抽象，以及测试/本地实验使用的文件系统假实现。

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudStoreError {
    AuthRequired,
    RateLimited,
    NetworkUnavailable,
    NotFound(String),
    AlreadyExists(String),
    InvalidPath(String),
    Provider(String),
    Io(String),
}

impl fmt::Display for CloudStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AuthRequired => write!(f, "云账号需要重新授权"),
            Self::RateLimited => write!(f, "云端请求被限流"),
            Self::NetworkUnavailable => write!(f, "网络不可用"),
            Self::NotFound(path) => write!(f, "云端对象不存在: {path}"),
            Self::AlreadyExists(path) => write!(f, "云端对象已存在: {path}"),
            Self::InvalidPath(path) => write!(f, "非法云端路径: {path}"),
            Self::Provider(message) => write!(f, "云服务错误: {message}"),
            Self::Io(message) => write!(f, "云端存储 IO 错误: {message}"),
        }
    }
}

impl std::error::Error for CloudStoreError {}

pub type CloudStoreResult<T> = std::result::Result<T, CloudStoreError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutMode {
    CreateOnly,
    Overwrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudEntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudFile {
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudEntry {
    pub path: String,
    pub name: String,
    pub kind: CloudEntryKind,
    pub size: u64,
    /// 云服务提供的最后修改时间（Unix 秒）。目录或服务不支持时为空。
    pub modified_at: Option<u64>,
}

pub trait CloudObjectStore: Send + Sync {
    fn put_file(
        &self,
        remote_path: &str,
        local_file: &Path,
        mode: PutMode,
    ) -> CloudStoreResult<CloudFile>;

    fn get_file(&self, remote_path: &str, local_file: &Path) -> CloudStoreResult<()>;

    fn list_directory(&self, remote_path: &str) -> CloudStoreResult<Vec<CloudEntry>>;

    fn stat_file(&self, remote_path: &str) -> CloudStoreResult<Option<CloudFile>>;

    /// 删除按幂等语义处理：目标不存在也算成功。
    fn delete_file(&self, remote_path: &str) -> CloudStoreResult<()>;
}

/// 文件系统支持的假云端。它只实现云对象语义，不承担 SaveLink 协议判断。
pub struct FakeCloudObjectStore {
    root: PathBuf,
}

impl FakeCloudObjectStore {
    pub fn new(root: impl Into<PathBuf>) -> CloudStoreResult<Self> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|e| CloudStoreError::Io(e.to_string()))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn resolve(&self, remote_path: &str, allow_root: bool) -> CloudStoreResult<PathBuf> {
        if remote_path.is_empty() {
            return if allow_root {
                Ok(self.root.clone())
            } else {
                Err(CloudStoreError::InvalidPath(remote_path.to_string()))
            };
        }
        if remote_path.starts_with('/')
            || remote_path.ends_with('/')
            || remote_path.contains('\\')
            || remote_path.contains('\0')
        {
            return Err(CloudStoreError::InvalidPath(remote_path.to_string()));
        }

        let mut resolved = self.root.clone();
        for segment in remote_path.split('/') {
            if segment.is_empty() || segment == "." || segment == ".." || segment.contains(':') {
                return Err(CloudStoreError::InvalidPath(remote_path.to_string()));
            }
            resolved.push(segment);
        }
        Ok(resolved)
    }

    fn logical_child(parent: &str, name: &str) -> String {
        if parent.is_empty() {
            name.to_string()
        } else {
            format!("{parent}/{name}")
        }
    }
}

impl CloudObjectStore for FakeCloudObjectStore {
    fn put_file(
        &self,
        remote_path: &str,
        local_file: &Path,
        mode: PutMode,
    ) -> CloudStoreResult<CloudFile> {
        if !local_file.is_file() {
            return Err(CloudStoreError::Io(format!(
                "本机上传源不是文件: {}",
                local_file.display()
            )));
        }
        let target = self.resolve(remote_path, false)?;
        if mode == PutMode::CreateOnly && target.exists() {
            return Err(CloudStoreError::AlreadyExists(remote_path.to_string()));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| CloudStoreError::Io(e.to_string()))?;
        }
        fs::copy(local_file, &target).map_err(|e| CloudStoreError::Io(e.to_string()))?;
        let size = fs::metadata(&target)
            .map_err(|e| CloudStoreError::Io(e.to_string()))?
            .len();
        Ok(CloudFile {
            path: remote_path.to_string(),
            size,
        })
    }

    fn get_file(&self, remote_path: &str, local_file: &Path) -> CloudStoreResult<()> {
        let source = self.resolve(remote_path, false)?;
        if !source.is_file() {
            return Err(CloudStoreError::NotFound(remote_path.to_string()));
        }
        if let Some(parent) = local_file.parent() {
            fs::create_dir_all(parent).map_err(|e| CloudStoreError::Io(e.to_string()))?;
        }
        fs::copy(source, local_file).map_err(|e| CloudStoreError::Io(e.to_string()))?;
        Ok(())
    }

    fn list_directory(&self, remote_path: &str) -> CloudStoreResult<Vec<CloudEntry>> {
        let directory = self.resolve(remote_path, true)?;
        if !directory.is_dir() {
            return Err(CloudStoreError::NotFound(remote_path.to_string()));
        }

        let mut entries = Vec::new();
        for entry in fs::read_dir(directory).map_err(|e| CloudStoreError::Io(e.to_string()))? {
            let entry = entry.map_err(|e| CloudStoreError::Io(e.to_string()))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let metadata = entry
                .metadata()
                .map_err(|e| CloudStoreError::Io(e.to_string()))?;
            let kind = if metadata.is_dir() {
                CloudEntryKind::Directory
            } else if metadata.is_file() {
                CloudEntryKind::File
            } else {
                continue;
            };
            entries.push(CloudEntry {
                path: Self::logical_child(remote_path, &name),
                name,
                kind,
                size: if kind == CloudEntryKind::File {
                    metadata.len()
                } else {
                    0
                },
                modified_at: metadata
                    .modified()
                    .ok()
                    .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                    .map(|value| value.as_secs()),
            });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    fn stat_file(&self, remote_path: &str) -> CloudStoreResult<Option<CloudFile>> {
        let path = self.resolve(remote_path, false)?;
        if !path.exists() {
            return Ok(None);
        }
        if !path.is_file() {
            return Err(CloudStoreError::InvalidPath(remote_path.to_string()));
        }
        let size = fs::metadata(path)
            .map_err(|e| CloudStoreError::Io(e.to_string()))?
            .len();
        Ok(Some(CloudFile {
            path: remote_path.to_string(),
            size,
        }))
    }

    fn delete_file(&self, remote_path: &str) -> CloudStoreResult<()> {
        let path = self.resolve(remote_path, false)?;
        if !path.exists() {
            return Ok(());
        }
        if !path.is_file() {
            return Err(CloudStoreError::InvalidPath(remote_path.to_string()));
        }
        fs::remove_file(path).map_err(|e| CloudStoreError::Io(e.to_string()))
    }
}
