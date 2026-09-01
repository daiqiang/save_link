//! 云同步在本机保存的账号、游戏绑定和远端快照目录模型。

use crate::model::Reason;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudAccount {
    pub id: String,
    pub provider: String,
    pub account_identity: Option<String>,
    pub display_name: Option<String>,
    pub token_ref: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudGameBinding {
    pub account_id: String,
    pub cloud_game_id: String,
    pub local_game_id: String,
    pub remote_revision: u64,
    pub sync_enabled: bool,
    pub last_scanned_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudSyncStatus {
    Uploading,
    Uploaded,
    RemoteOnly,
    Downloading,
    Downloaded,
    Ignored,
    Error,
    DeletePending,
    Deleting,
    DeleteFailed,
    RemoteDeleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudMetadataSyncStatus {
    Synced,
    Pending,
    Error,
}

impl CloudMetadataSyncStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Synced => "synced",
            Self::Pending => "pending",
            Self::Error => "error",
        }
    }

    pub fn from_db_value(value: &str) -> Self {
        match value {
            "pending" => Self::Pending,
            "error" => Self::Error,
            _ => Self::Synced,
        }
    }
}

impl CloudSyncStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Uploading => "uploading",
            Self::Uploaded => "uploaded",
            Self::RemoteOnly => "remote_only",
            Self::Downloading => "downloading",
            Self::Downloaded => "downloaded",
            Self::Ignored => "ignored",
            Self::Error => "error",
            Self::DeletePending => "delete_pending",
            Self::Deleting => "deleting",
            Self::DeleteFailed => "delete_failed",
            Self::RemoteDeleted => "remote_deleted",
        }
    }

    pub fn from_db_value(value: &str) -> Self {
        match value {
            "uploading" => Self::Uploading,
            "uploaded" => Self::Uploaded,
            "remote_only" => Self::RemoteOnly,
            "downloading" => Self::Downloading,
            "downloaded" => Self::Downloaded,
            "ignored" => Self::Ignored,
            "delete_pending" => Self::DeletePending,
            "deleting" => Self::Deleting,
            "delete_failed" => Self::DeleteFailed,
            "remote_deleted" => Self::RemoteDeleted,
            _ => Self::Error,
        }
    }
}

/// 云端 `.ok` 的本机缓存，同时记录该快照在当前设备上的同步状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudSnapshotRecord {
    pub account_id: String,
    pub cloud_game_id: String,
    pub snapshot_id: String,
    pub created_at: String,
    pub reason: Reason,
    pub note: Option<String>,
    pub locked: bool,
    pub file_count: u64,
    pub total_size: u64,
    pub source_count: u32,
    pub content_hash: String,
    pub archive_size: u64,
    pub archive_sha256: String,
    pub published_at: String,
    pub created_by_device_id: String,
    pub sync_status: CloudSyncStatus,
    pub last_synced_at: Option<String>,
    pub last_error_code: Option<String>,
    /// 名称/锁定状态的同步状态，与 zip 内容生命周期相互独立。
    pub metadata_sync_status: CloudMetadataSyncStatus,
    pub metadata_last_synced_at: Option<String>,
    pub metadata_last_error_code: Option<String>,
    /// 最近一次已读取的远端 metadata 文件修改时间（Unix 秒）。
    pub remote_metadata_modified_at: Option<u64>,
    /// 最近一次已知云端字段版本；本机修改时间保存在 `Snapshot`。
    pub remote_note_updated_at: String,
    pub remote_locked_updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudSnapshotMetadataState {
    pub status: CloudMetadataSyncStatus,
    pub last_synced_at: Option<String>,
    pub last_error_code: Option<String>,
    pub remote_note_updated_at: Option<String>,
    pub remote_locked_updated_at: Option<String>,
}
