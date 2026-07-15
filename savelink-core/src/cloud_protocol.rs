//! SaveLink 云端快照协议 v1 的 JSON 契约与逻辑路径。

use crate::model::Reason;
use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const CLOUD_ROOT: &str = "savelink/v1";
pub const PROTOCOL_NAME: &str = "savelink-cloud-snapshot";
pub const PROTOCOL_VERSION: u32 = 1;
pub const CONTENT_HASH_ALGORITHM: &str = "savelink-fnv1a64-tree-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudProtocolError {
    pub code: &'static str,
    pub message: String,
}

impl CloudProtocolError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for CloudProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CloudProtocolError {}

pub type CloudProtocolResult<T> = std::result::Result<T, CloudProtocolError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudManifest {
    pub protocol: String,
    pub protocol_version: u32,
    pub repository_id: String,
    pub created_at: String,
    pub created_by_device_id: String,
}

impl CloudManifest {
    pub fn validate(&self) -> CloudProtocolResult<()> {
        if self.protocol != PROTOCOL_NAME || self.protocol_version != PROTOCOL_VERSION {
            return Err(CloudProtocolError::new(
                "protocol_not_supported",
                format!("不支持的协议: {}/{}", self.protocol, self.protocol_version),
            ));
        }
        validate_id(&self.repository_id, "repository_id")?;
        validate_id(&self.created_by_device_id, "created_by_device_id")?;
        validate_timestamp(&self.created_at, "created_at")
    }

    pub fn to_json(&self) -> CloudProtocolResult<Vec<u8>> {
        self.validate()?;
        encode_json(self)
    }

    pub fn from_json(bytes: &[u8]) -> CloudProtocolResult<Self> {
        let value: Self = decode_json(bytes)?;
        value.validate()?;
        Ok(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudGameDocument {
    pub schema_version: u32,
    pub object_type: String,
    pub cloud_game_id: String,
    pub name: String,
    pub created_at: String,
    pub revision: u64,
    pub updated_at: String,
    pub updated_by_device_id: String,
}

impl CloudGameDocument {
    pub fn validate(&self, expected_game_id: &str) -> CloudProtocolResult<()> {
        if self.schema_version != 1 || self.object_type != "game" {
            return Err(CloudProtocolError::new(
                "game_metadata_invalid",
                "game.json 类型或版本不正确",
            ));
        }
        validate_id(&self.cloud_game_id, "cloud_game_id")?;
        if self.cloud_game_id != expected_game_id {
            return Err(CloudProtocolError::new(
                "game_metadata_invalid",
                "game.json 中的 cloud_game_id 与目录不一致",
            ));
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 256 {
            return Err(CloudProtocolError::new(
                "game_metadata_invalid",
                "游戏名称为空或过长",
            ));
        }
        if self.revision == 0 {
            return Err(CloudProtocolError::new(
                "game_metadata_invalid",
                "game.json revision 必须从 1 开始",
            ));
        }
        validate_timestamp(&self.created_at, "created_at")?;
        validate_timestamp(&self.updated_at, "updated_at")?;
        validate_id(&self.updated_by_device_id, "updated_by_device_id")
    }

    pub fn to_json(&self) -> CloudProtocolResult<Vec<u8>> {
        self.validate(&self.cloud_game_id)?;
        encode_json(self)
    }

    pub fn from_json(bytes: &[u8], expected_game_id: &str) -> CloudProtocolResult<Self> {
        let value: Self = decode_json(bytes)?;
        value.validate(expected_game_id)?;
        Ok(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentHashDocument {
    pub algorithm: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveDocument {
    pub file_name: String,
    pub format: String,
    pub layout_version: u32,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotCommitDocument {
    pub schema_version: u32,
    pub object_type: String,
    pub snapshot_id: String,
    pub cloud_game_id: String,
    pub created_at: String,
    pub reason: String,
    pub note: Option<String>,
    pub locked: bool,
    pub file_count: u64,
    pub total_size: u64,
    pub content_hash: ContentHashDocument,
    pub archive: ArchiveDocument,
    pub published_at: String,
    pub created_by_device_id: String,
}

impl SnapshotCommitDocument {
    pub fn validate(
        &self,
        expected_game_id: &str,
        expected_snapshot_id: &str,
    ) -> CloudProtocolResult<()> {
        if self.schema_version != 1 || self.object_type != "snapshot_commit" {
            return Err(CloudProtocolError::new(
                "snapshot_marker_invalid",
                "云端 .ok 类型或版本不正确",
            ));
        }
        validate_id(&self.snapshot_id, "snapshot_id")?;
        validate_id(&self.cloud_game_id, "cloud_game_id")?;
        if self.snapshot_id != expected_snapshot_id || self.cloud_game_id != expected_game_id {
            return Err(CloudProtocolError::new(
                "snapshot_marker_invalid",
                "云端 .ok 的游戏或快照 ID 与路径不一致",
            ));
        }
        reason_from_protocol(&self.reason)?;
        if self
            .note
            .as_ref()
            .is_some_and(|note| note.chars().count() > 2000)
        {
            return Err(CloudProtocolError::new(
                "snapshot_marker_invalid",
                "快照备注超过 2000 个字符",
            ));
        }
        if self.content_hash.algorithm != CONTENT_HASH_ALGORITHM
            || !is_lower_hex(&self.content_hash.value, 16)
        {
            return Err(CloudProtocolError::new(
                "snapshot_marker_invalid",
                "内容指纹算法或格式不正确",
            ));
        }
        if self.archive.file_name != format!("{}.zip", self.snapshot_id)
            || self.archive.format != "zip"
            || self.archive.layout_version != 1
            || self.archive.size == 0
            || !is_lower_hex(&self.archive.sha256, 64)
        {
            return Err(CloudProtocolError::new(
                "snapshot_marker_invalid",
                "zip 描述字段不正确",
            ));
        }
        validate_timestamp(&self.created_at, "created_at")?;
        validate_timestamp(&self.published_at, "published_at")?;
        validate_id(&self.created_by_device_id, "created_by_device_id")
    }

    pub fn to_json(&self) -> CloudProtocolResult<Vec<u8>> {
        self.validate(&self.cloud_game_id, &self.snapshot_id)?;
        encode_json(self)
    }

    pub fn from_json(
        bytes: &[u8],
        expected_game_id: &str,
        expected_snapshot_id: &str,
    ) -> CloudProtocolResult<Self> {
        if bytes.len() > 64 * 1024 {
            return Err(CloudProtocolError::new(
                "snapshot_marker_invalid",
                "云端 .ok 超过 64KiB",
            ));
        }
        let value: Self = decode_json(bytes)?;
        value.validate(expected_game_id, expected_snapshot_id)?;
        Ok(value)
    }

    pub fn reason(&self) -> CloudProtocolResult<Reason> {
        reason_from_protocol(&self.reason)
    }

    pub fn same_logical_snapshot(&self, other: &Self) -> bool {
        self.snapshot_id == other.snapshot_id
            && self.cloud_game_id == other.cloud_game_id
            && self.created_at == other.created_at
            && self.reason == other.reason
            && self.file_count == other.file_count
            && self.total_size == other.total_size
            && self.content_hash == other.content_hash
    }
}

pub fn manifest_path() -> String {
    format!("{CLOUD_ROOT}/manifest.json")
}

pub fn games_path() -> String {
    format!("{CLOUD_ROOT}/games")
}

pub fn game_path(cloud_game_id: &str) -> CloudProtocolResult<String> {
    validate_id(cloud_game_id, "cloud_game_id")?;
    Ok(format!("{}/{cloud_game_id}/game.json", games_path()))
}

pub fn snapshots_path(cloud_game_id: &str) -> CloudProtocolResult<String> {
    validate_id(cloud_game_id, "cloud_game_id")?;
    Ok(format!("{}/{cloud_game_id}/snapshots", games_path()))
}

pub fn snapshot_zip_path(cloud_game_id: &str, snapshot_id: &str) -> CloudProtocolResult<String> {
    validate_id(snapshot_id, "snapshot_id")?;
    Ok(format!(
        "{}/{}.zip",
        snapshots_path(cloud_game_id)?,
        snapshot_id
    ))
}

pub fn snapshot_ok_path(cloud_game_id: &str, snapshot_id: &str) -> CloudProtocolResult<String> {
    validate_id(snapshot_id, "snapshot_id")?;
    Ok(format!(
        "{}/{}.ok",
        snapshots_path(cloud_game_id)?,
        snapshot_id
    ))
}

pub fn snapshot_id_from_ok_name(name: &str) -> CloudProtocolResult<String> {
    let Some(id) = name.strip_suffix(".ok") else {
        return Err(CloudProtocolError::new(
            "snapshot_marker_invalid",
            "文件名不是 .ok",
        ));
    };
    validate_id(id, "snapshot_id")?;
    Ok(id.to_string())
}

pub fn reason_to_protocol(reason: Reason) -> &'static str {
    match reason {
        Reason::Manual => "manual",
        Reason::BeforeRestore => "before_restore",
        Reason::Auto => "auto",
    }
}

pub fn reason_from_protocol(value: &str) -> CloudProtocolResult<Reason> {
    match value {
        "manual" => Ok(Reason::Manual),
        "before_restore" => Ok(Reason::BeforeRestore),
        "auto" => Ok(Reason::Auto),
        _ => Err(CloudProtocolError::new(
            "snapshot_marker_invalid",
            format!("未知快照原因: {value}"),
        )),
    }
}

pub fn normalize_timestamp(value: &str) -> CloudProtocolResult<String> {
    if let Ok(timestamp) = DateTime::parse_from_rfc3339(value) {
        return Ok(timestamp.to_rfc3339());
    }
    let parsed = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S"))
        .map_err(|_| {
            CloudProtocolError::new("timestamp_invalid", format!("无法识别时间格式: {value}"))
        })?;
    let local = Local
        .from_local_datetime(&parsed)
        .earliest()
        .ok_or_else(|| CloudProtocolError::new("timestamp_invalid", "本地时间无效"))?;
    Ok(local.to_rfc3339())
}

pub fn validate_id(value: &str, field: &str) -> CloudProtocolResult<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(CloudProtocolError::new(
            "identifier_invalid",
            format!("{field} 不是合法 ID"),
        ));
    }
    Ok(())
}

fn validate_timestamp(value: &str, field: &str) -> CloudProtocolResult<()> {
    DateTime::parse_from_rfc3339(value).map_err(|_| {
        CloudProtocolError::new("timestamp_invalid", format!("{field} 不是 RFC 3339 时间"))
    })?;
    Ok(())
}

fn is_lower_hex(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn encode_json<T: Serialize>(value: &T) -> CloudProtocolResult<Vec<u8>> {
    serde_json::to_vec_pretty(value).map_err(|error| {
        CloudProtocolError::new("json_invalid", format!("JSON 序列化失败: {error}"))
    })
}

fn decode_json<T: DeserializeOwned>(bytes: &[u8]) -> CloudProtocolResult<T> {
    serde_json::from_slice(bytes)
        .map_err(|error| CloudProtocolError::new("json_invalid", format!("JSON 解析失败: {error}")))
}
