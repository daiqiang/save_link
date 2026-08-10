//! 云端快照协议 v1 的上传、发现和接收落地编排。

use crate::cloud_archive::{CloudArchiveCodec, CloudArchiveError, SnapshotContentExpectation};
use crate::cloud_model::{CloudGameBinding, CloudSnapshotRecord, CloudSyncStatus};
use crate::cloud_protocol::{
    archive_layout_version, content_hash_algorithm, game_path, games_path, manifest_path,
    normalize_timestamp, reason_to_protocol, snapshot_id_from_ok_name, snapshot_ok_path,
    snapshot_zip_path, snapshots_path, ArchiveDocument, CloudGameDocument, CloudManifest,
    CloudProtocolError, ContentHashDocument, SnapshotCommitDocument, PROTOCOL_NAME,
    PROTOCOL_VERSION,
};
use crate::cloud_repo::CloudStateRepository;
use crate::cloud_store::{CloudEntryKind, CloudObjectStore, CloudStoreError, PutMode};
use crate::error::SaveLinkError;
use crate::model::{Game, ScanResult, Snapshot, SnapshotStatus};
use crate::repo::{Clock, Repository};
use crate::scan;
use crate::store::SnapshotStore;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug)]
pub enum CloudSyncError {
    Local(SaveLinkError),
    Store(CloudStoreError),
    Protocol(CloudProtocolError),
    Archive(CloudArchiveError),
    SnapshotIdConflict(String),
    RemoteZipMissing(String),
    InvalidState(String),
    Io(String),
}

impl CloudSyncError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Local(_) => "local_store_failed",
            Self::Store(error) => match error {
                CloudStoreError::AuthRequired => "auth_required",
                CloudStoreError::RateLimited => "rate_limited",
                CloudStoreError::NetworkUnavailable => "network_unavailable",
                CloudStoreError::NotFound(_) => "remote_object_missing",
                CloudStoreError::AlreadyExists(_) => "remote_object_exists",
                CloudStoreError::InvalidPath(_) => "remote_path_invalid",
                CloudStoreError::Provider(_) => "provider_error",
                CloudStoreError::Io(_) => "cloud_io_failed",
            },
            Self::Protocol(error) => error.code,
            Self::Archive(error) => error.code(),
            Self::SnapshotIdConflict(_) => "snapshot_id_conflict",
            Self::RemoteZipMissing(_) => "remote_zip_missing",
            Self::InvalidState(_) => "cloud_state_invalid",
            Self::Io(_) => "cloud_io_failed",
        }
    }
}

impl fmt::Display for CloudSyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local(error) => write!(f, "本机数据错误: {error}"),
            Self::Store(error) => write!(f, "云对象存储错误: {error}"),
            Self::Protocol(error) => write!(f, "云端协议错误: {error}"),
            Self::Archive(error) => write!(f, "云端归档错误: {error}"),
            Self::SnapshotIdConflict(id) => write!(f, "快照 ID 冲突: {id}"),
            Self::RemoteZipMissing(id) => write!(f, "云端快照缺少 zip: {id}"),
            Self::InvalidState(message) => write!(f, "云同步状态错误: {message}"),
            Self::Io(message) => write!(f, "云同步 IO 错误: {message}"),
        }
    }
}

impl std::error::Error for CloudSyncError {}

impl From<SaveLinkError> for CloudSyncError {
    fn from(value: SaveLinkError) -> Self {
        Self::Local(value)
    }
}

impl From<CloudStoreError> for CloudSyncError {
    fn from(value: CloudStoreError) -> Self {
        Self::Store(value)
    }
}

impl From<CloudProtocolError> for CloudSyncError {
    fn from(value: CloudProtocolError) -> Self {
        Self::Protocol(value)
    }
}

impl From<CloudArchiveError> for CloudSyncError {
    fn from(value: CloudArchiveError) -> Self {
        Self::Archive(value)
    }
}

pub type CloudSyncResult<T> = std::result::Result<T, CloudSyncError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadOutcome {
    Uploaded,
    AlreadyPresent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveOutcome {
    Downloaded,
    AlreadyPresent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudSnapshotDiscovery {
    pub cloud_game_id: String,
    pub game_name: String,
    pub snapshot: CloudSnapshotRecord,
}

pub struct CloudSyncService<R>
where
    R: Repository + CloudStateRepository,
{
    repo: Arc<R>,
    snapshot_store: Arc<dyn SnapshotStore>,
    cloud_store: Arc<dyn CloudObjectStore>,
    archive_codec: Arc<dyn CloudArchiveCodec>,
    clock: Arc<dyn Clock>,
    work_dir: PathBuf,
    account_id: String,
    device_id: String,
    new_repository_id: String,
}

impl<R> CloudSyncService<R>
where
    R: Repository + CloudStateRepository + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo: Arc<R>,
        snapshot_store: Arc<dyn SnapshotStore>,
        cloud_store: Arc<dyn CloudObjectStore>,
        archive_codec: Arc<dyn CloudArchiveCodec>,
        clock: Arc<dyn Clock>,
        work_dir: impl Into<PathBuf>,
        account_id: impl Into<String>,
        device_id: impl Into<String>,
        new_repository_id: impl Into<String>,
    ) -> CloudSyncResult<Self> {
        let service = Self {
            repo,
            snapshot_store,
            cloud_store,
            archive_codec,
            clock,
            work_dir: work_dir.into(),
            account_id: account_id.into(),
            device_id: device_id.into(),
            new_repository_id: new_repository_id.into(),
        };
        crate::cloud_protocol::validate_id(&service.account_id, "account_id")?;
        crate::cloud_protocol::validate_id(&service.device_id, "device_id")?;
        crate::cloud_protocol::validate_id(&service.new_repository_id, "new_repository_id")?;
        fs::create_dir_all(&service.work_dir).map_err(io_error)?;
        Ok(service)
    }

    pub fn ensure_manifest(&self) -> CloudSyncResult<CloudManifest> {
        let remote_path = manifest_path();
        if self.cloud_store.stat_file(&remote_path)?.is_some() {
            return self.read_manifest();
        }

        let manifest = CloudManifest {
            protocol: PROTOCOL_NAME.into(),
            protocol_version: PROTOCOL_VERSION,
            repository_id: self.new_repository_id.clone(),
            created_at: self.now_timestamp()?,
            created_by_device_id: self.device_id.clone(),
        };
        let local_path = self.metadata_temp_path("manifest-upload.json")?;
        write_bytes(&local_path, &manifest.to_json()?)?;
        match self
            .cloud_store
            .put_file(&remote_path, &local_path, PutMode::CreateOnly)
        {
            Ok(_) => Ok(manifest),
            Err(CloudStoreError::AlreadyExists(_)) => self.read_manifest(),
            Err(error) => Err(error.into()),
        }
    }

    pub fn upload_snapshot(
        &self,
        game_id: &str,
        snapshot_id: &str,
    ) -> CloudSyncResult<UploadOutcome> {
        let result = self.upload_snapshot_inner(game_id, snapshot_id);
        if let Err(error) = &result {
            self.mark_snapshot_error(snapshot_id, error);
            self.cleanup_operation_dir("upload", snapshot_id);
        }
        result
    }

    pub fn discover_remote_snapshots(&self) -> CloudSyncResult<Vec<CloudSnapshotRecord>> {
        Ok(self
            .discover_remote_catalog()?
            .into_iter()
            .map(|entry| entry.snapshot)
            .collect())
    }

    pub fn discover_remote_catalog(&self) -> CloudSyncResult<Vec<CloudSnapshotDiscovery>> {
        self.ensure_manifest()?;
        let game_entries = match self.cloud_store.list_directory(&games_path()) {
            Ok(entries) => entries,
            Err(CloudStoreError::NotFound(_)) => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };

        let mut discovered = Vec::new();
        for entry in game_entries {
            if entry.kind != CloudEntryKind::Directory {
                continue;
            }
            let cloud_game_id = entry.name;
            crate::cloud_protocol::validate_id(&cloud_game_id, "cloud_game_id")?;
            let game = self.read_game_document(&cloud_game_id)?;

            let snapshot_entries = match self
                .cloud_store
                .list_directory(&snapshots_path(&cloud_game_id)?)
            {
                Ok(entries) => entries,
                Err(CloudStoreError::NotFound(_)) => continue,
                Err(error) => return Err(error.into()),
            };
            for snapshot_entry in snapshot_entries {
                if snapshot_entry.kind != CloudEntryKind::File
                    || !snapshot_entry.name.ends_with(".ok")
                {
                    continue;
                }
                let snapshot_id = snapshot_id_from_ok_name(&snapshot_entry.name)?;
                let commit = self.read_snapshot_commit(&cloud_game_id, &snapshot_id)?;
                self.ensure_remote_zip_exists(&commit)?;
                let status = self.discovery_status(&commit)?;
                let record = record_from_commit(&self.account_id, &commit, status, None);
                self.repo.upsert_cloud_snapshot(record.clone())?;
                discovered.push(CloudSnapshotDiscovery {
                    cloud_game_id: cloud_game_id.clone(),
                    game_name: game.name.clone(),
                    snapshot: record,
                });
            }
        }
        discovered.sort_by(|left, right| {
            crate::timestamp::compare_timestamps(
                &right.snapshot.created_at,
                &left.snapshot.created_at,
            )
            .then_with(|| right.snapshot.snapshot_id.cmp(&left.snapshot.snapshot_id))
        });
        Ok(discovered)
    }

    pub fn receive_remote_snapshot(&self, snapshot_id: &str) -> CloudSyncResult<ReceiveOutcome> {
        let result = self.receive_remote_snapshot_inner(snapshot_id);
        if let Err(error) = &result {
            self.mark_snapshot_error(snapshot_id, error);
            self.cleanup_operation_dir("download", snapshot_id);
        }
        result
    }

    /// 删除保留策略淘汰的未锁定快照。先撤销云端发布，再删除本机数据。
    pub fn delete_snapshot_everywhere(&self, snapshot_id: &str) -> CloudSyncResult<()> {
        let snapshot = self
            .repo
            .get_snapshot(snapshot_id)?
            .ok_or_else(|| CloudSyncError::InvalidState(format!("快照不存在: {snapshot_id}")))?;
        if snapshot.locked {
            return Err(SaveLinkError::SnapshotLocked.into());
        }

        if let Some(cloud) = self
            .repo
            .get_cloud_snapshot(&self.account_id, snapshot_id)?
        {
            if cloud.sync_status != CloudSyncStatus::RemoteDeleted {
                self.repo.update_cloud_snapshot_status(
                    &self.account_id,
                    snapshot_id,
                    CloudSyncStatus::DeletePending,
                    None,
                    None,
                )?;
                self.repo.update_cloud_snapshot_status(
                    &self.account_id,
                    snapshot_id,
                    CloudSyncStatus::Deleting,
                    None,
                    None,
                )?;

                let remote_result = (|| -> CloudSyncResult<()> {
                    self.cloud_store.delete_file(&snapshot_ok_path(
                        &cloud.cloud_game_id,
                        snapshot_id,
                    )?)?;
                    self.cloud_store.delete_file(&snapshot_zip_path(
                        &cloud.cloud_game_id,
                        snapshot_id,
                    )?)?;
                    Ok(())
                })();
                if let Err(error) = remote_result {
                    let _ = self.repo.update_cloud_snapshot_status(
                        &self.account_id,
                        snapshot_id,
                        CloudSyncStatus::DeleteFailed,
                        None,
                        Some(error.code()),
                    );
                    return Err(error);
                }

                self.repo.update_cloud_snapshot_status(
                    &self.account_id,
                    snapshot_id,
                    CloudSyncStatus::RemoteDeleted,
                    Some(&self.now_timestamp()?),
                    None,
                )?;
            }
        }

        let mut deleting = snapshot;
        let previous_status = deleting.status;
        deleting.status = SnapshotStatus::Deleting;
        self.repo.update_snapshot(deleting.clone())?;
        if let Err(error) = self.snapshot_store.delete(&deleting.storage_key) {
            deleting.status = previous_status;
            let _ = self.repo.update_snapshot(deleting);
            return Err(error.into());
        }
        self.repo.delete_snapshot(snapshot_id)?;
        self.repo
            .delete_cloud_snapshot(&self.account_id, snapshot_id)?;
        Ok(())
    }

    fn upload_snapshot_inner(
        &self,
        game_id: &str,
        snapshot_id: &str,
    ) -> CloudSyncResult<UploadOutcome> {
        self.ensure_manifest()?;
        let game = self
            .repo
            .get_game(game_id)?
            .ok_or_else(|| CloudSyncError::InvalidState(format!("游戏不存在: {game_id}")))?;
        let snapshot = self
            .repo
            .get_snapshot(snapshot_id)?
            .ok_or_else(|| CloudSyncError::InvalidState(format!("快照不存在: {snapshot_id}")))?;
        if snapshot.game_id != game.id || snapshot.status != SnapshotStatus::Complete {
            return Err(CloudSyncError::InvalidState(
                "快照不属于该游戏或尚未完成".into(),
            ));
        }
        if !self.snapshot_store.verify(&snapshot.storage_key)? {
            return Err(SaveLinkError::SnapshotCorrupt.into());
        }

        self.ensure_game_document(&game)?;
        let normalized_created_at = normalize_timestamp(&snapshot.created_at)?;
        let remote_ok = snapshot_ok_path(game_id, snapshot_id)?;
        if self.cloud_store.stat_file(&remote_ok)?.is_some() {
            let existing = self.read_snapshot_commit(game_id, snapshot_id)?;
            if !commit_matches_local(&existing, &snapshot, &normalized_created_at) {
                return Err(CloudSyncError::SnapshotIdConflict(snapshot_id.into()));
            }
            self.ensure_remote_zip_exists(&existing)?;
            self.repo.upsert_cloud_snapshot(record_from_commit(
                &self.account_id,
                &existing,
                CloudSyncStatus::Uploaded,
                Some(self.now_timestamp()?),
            ))?;
            return Ok(UploadOutcome::AlreadyPresent);
        }

        let operation_dir = self.prepare_operation_dir("upload", snapshot_id)?;
        let staging_dir = operation_dir.join("snapshot");
        self.snapshot_store
            .restore(&snapshot.storage_key, &staging_dir)?;
        let scan_result = scan::fingerprint_snapshot_payload(&staging_dir, snapshot.source_count)?;
        ensure_scan_matches_snapshot(&scan_result, &snapshot)?;

        let archive_path = operation_dir.join(format!("{snapshot_id}.zip"));
        let archive = self
            .archive_codec
            .create_archive(&staging_dir, &archive_path)?;
        let commit = SnapshotCommitDocument {
            schema_version: 1,
            object_type: "snapshot_commit".into(),
            snapshot_id: snapshot.id.clone(),
            cloud_game_id: game.id.clone(),
            created_at: normalized_created_at,
            reason: reason_to_protocol(snapshot.reason).into(),
            note: snapshot.note.clone(),
            locked: snapshot.locked,
            file_count: snapshot.file_count,
            total_size: snapshot.total_size,
            source_count: snapshot.source_count,
            content_hash: ContentHashDocument {
                algorithm: content_hash_algorithm(snapshot.source_count).into(),
                value: snapshot.content_hash.clone(),
            },
            archive: ArchiveDocument {
                file_name: format!("{snapshot_id}.zip"),
                format: "zip".into(),
                layout_version: archive_layout_version(snapshot.source_count),
                size: archive.size,
                sha256: archive.sha256,
            },
            published_at: self.now_timestamp()?,
            created_by_device_id: self.device_id.clone(),
        };
        commit.validate(game_id, snapshot_id)?;
        self.repo.upsert_cloud_snapshot(record_from_commit(
            &self.account_id,
            &commit,
            CloudSyncStatus::Uploading,
            None,
        ))?;

        let remote_zip = snapshot_zip_path(game_id, snapshot_id)?;
        self.cloud_store
            .put_file(&remote_zip, &archive_path, PutMode::Overwrite)?;
        let remote_zip_info = self
            .cloud_store
            .stat_file(&remote_zip)?
            .ok_or_else(|| CloudSyncError::RemoteZipMissing(snapshot_id.into()))?;
        if remote_zip_info.size != commit.archive.size {
            return Err(CloudArchiveError::ArchiveSizeMismatch.into());
        }

        let ok_path = operation_dir.join(format!("{snapshot_id}.ok"));
        write_bytes(&ok_path, &commit.to_json()?)?;
        match self
            .cloud_store
            .put_file(&remote_ok, &ok_path, PutMode::CreateOnly)
        {
            Ok(_) => {}
            Err(CloudStoreError::AlreadyExists(_)) => {
                let existing = self.read_snapshot_commit(game_id, snapshot_id)?;
                if !existing.same_logical_snapshot(&commit) {
                    return Err(CloudSyncError::SnapshotIdConflict(snapshot_id.into()));
                }
            }
            Err(error) => return Err(error.into()),
        }
        let published = self.read_snapshot_commit(game_id, snapshot_id)?;
        if published != commit {
            return Err(CloudSyncError::SnapshotIdConflict(snapshot_id.into()));
        }

        self.repo.upsert_cloud_snapshot(record_from_commit(
            &self.account_id,
            &commit,
            CloudSyncStatus::Uploaded,
            Some(self.now_timestamp()?),
        ))?;
        let _ = fs::remove_dir_all(operation_dir);
        Ok(UploadOutcome::Uploaded)
    }

    fn receive_remote_snapshot_inner(&self, snapshot_id: &str) -> CloudSyncResult<ReceiveOutcome> {
        let cached = self
            .repo
            .get_cloud_snapshot(&self.account_id, snapshot_id)?
            .ok_or_else(|| {
                CloudSyncError::InvalidState(format!("尚未发现云端快照: {snapshot_id}"))
            })?;
        let commit = self.read_snapshot_commit(&cached.cloud_game_id, snapshot_id)?;
        if !record_matches_commit(&cached, &commit) {
            return Err(CloudSyncError::SnapshotIdConflict(snapshot_id.into()));
        }
        self.ensure_remote_zip_exists(&commit)?;

        if let Some(local) = self.repo.get_snapshot(snapshot_id)? {
            if !snapshot_matches_commit(&local, &commit)? {
                return Err(CloudSyncError::SnapshotIdConflict(snapshot_id.into()));
            }
            if !self.snapshot_store.verify(&local.storage_key)? {
                return Err(SaveLinkError::SnapshotCorrupt.into());
            }
            self.repo.update_cloud_snapshot_status(
                &self.account_id,
                snapshot_id,
                CloudSyncStatus::Downloaded,
                Some(&self.now_timestamp()?),
                None,
            )?;
            return Ok(ReceiveOutcome::AlreadyPresent);
        }

        self.repo.update_cloud_snapshot_status(
            &self.account_id,
            snapshot_id,
            CloudSyncStatus::Downloading,
            None,
            None,
        )?;
        let operation_dir = self.prepare_operation_dir("download", snapshot_id)?;
        let archive_path = operation_dir.join(format!("{snapshot_id}.zip.part"));
        self.cloud_store.get_file(
            &snapshot_zip_path(&cached.cloud_game_id, snapshot_id)?,
            &archive_path,
        )?;
        self.archive_codec.verify_archive(
            &archive_path,
            commit.archive.size,
            &commit.archive.sha256,
        )?;
        let extracted_dir = operation_dir.join("extracted");
        let scan_result = self.archive_codec.extract_verified(
            &archive_path,
            &extracted_dir,
            &SnapshotContentExpectation {
                file_count: commit.file_count,
                total_size: commit.total_size,
                content_hash: commit.content_hash.value.clone(),
                source_count: commit.source_count,
            },
        )?;

        let game = self.ensure_local_game_from_cloud(&cached.cloud_game_id)?;
        let local_created_at = normalize_timestamp(&commit.created_at)?;
        let mut local_snapshot = Snapshot {
            id: commit.snapshot_id.clone(),
            game_id: game.id,
            created_at: local_created_at,
            note: commit.note.clone(),
            reason: commit.reason()?,
            locked: commit.locked,
            file_count: commit.file_count,
            total_size: commit.total_size,
            source_count: commit.source_count,
            content_hash: commit.content_hash.value.clone(),
            storage_key: commit.snapshot_id.clone(),
            status: SnapshotStatus::Writing,
        };
        self.repo.insert_snapshot(local_snapshot.clone())?;

        let extracted_sources = snapshot_payload_sources(&extracted_dir, commit.source_count);
        let store_result =
            self.snapshot_store
                .create(&commit.snapshot_id, &extracted_sources, &scan_result);
        let stored = match store_result {
            Ok(stored) => stored,
            Err(error) => {
                let _ = self.repo.delete_snapshot(&commit.snapshot_id);
                return Err(error.into());
            }
        };
        match self.snapshot_store.verify(&stored.storage_key) {
            Ok(true) => {}
            Ok(false) => {
                let _ = self.snapshot_store.delete(&stored.storage_key);
                let _ = self.repo.delete_snapshot(&commit.snapshot_id);
                return Err(SaveLinkError::SnapshotCorrupt.into());
            }
            Err(error) => {
                let _ = self.snapshot_store.delete(&stored.storage_key);
                let _ = self.repo.delete_snapshot(&commit.snapshot_id);
                return Err(error.into());
            }
        }
        local_snapshot.storage_key = stored.storage_key;
        local_snapshot.status = SnapshotStatus::Complete;
        if let Err(error) = self.repo.update_snapshot(local_snapshot.clone()) {
            let _ = self.snapshot_store.delete(&local_snapshot.storage_key);
            let _ = self.repo.delete_snapshot(&commit.snapshot_id);
            return Err(error.into());
        }
        self.repo.update_cloud_snapshot_status(
            &self.account_id,
            snapshot_id,
            CloudSyncStatus::Downloaded,
            Some(&self.now_timestamp()?),
            None,
        )?;
        let _ = fs::remove_dir_all(operation_dir);
        Ok(ReceiveOutcome::Downloaded)
    }

    fn read_manifest(&self) -> CloudSyncResult<CloudManifest> {
        let bytes = self.read_remote_bytes(&manifest_path(), "manifest-read.json")?;
        Ok(CloudManifest::from_json(&bytes)?)
    }

    fn ensure_game_document(&self, game: &Game) -> CloudSyncResult<CloudGameDocument> {
        let remote_path = game_path(&game.id)?;
        if self.cloud_store.stat_file(&remote_path)?.is_some() {
            let existing = self.read_game_document(&game.id)?;
            self.upsert_binding(&existing)?;
            return Ok(existing);
        }
        let created_at = normalize_timestamp(&game.created_at)?;
        let document = CloudGameDocument {
            schema_version: 1,
            object_type: "game".into(),
            cloud_game_id: game.id.clone(),
            name: game.name.clone(),
            created_at,
            revision: 1,
            updated_at: normalize_timestamp(&game.updated_at)?,
            updated_by_device_id: self.device_id.clone(),
        };
        let local_path = self.metadata_temp_path(&format!("game-{}.json", game.id))?;
        write_bytes(&local_path, &document.to_json()?)?;
        match self
            .cloud_store
            .put_file(&remote_path, &local_path, PutMode::CreateOnly)
        {
            Ok(_) => {}
            Err(CloudStoreError::AlreadyExists(_)) => {
                let existing = self.read_game_document(&game.id)?;
                self.upsert_binding(&existing)?;
                return Ok(existing);
            }
            Err(error) => return Err(error.into()),
        }
        self.upsert_binding(&document)?;
        Ok(document)
    }

    fn read_game_document(&self, cloud_game_id: &str) -> CloudSyncResult<CloudGameDocument> {
        let bytes = self.read_remote_bytes(
            &game_path(cloud_game_id)?,
            &format!("game-{cloud_game_id}-read.json"),
        )?;
        Ok(CloudGameDocument::from_json(&bytes, cloud_game_id)?)
    }

    fn read_snapshot_commit(
        &self,
        cloud_game_id: &str,
        snapshot_id: &str,
    ) -> CloudSyncResult<SnapshotCommitDocument> {
        let bytes = self.read_remote_bytes(
            &snapshot_ok_path(cloud_game_id, snapshot_id)?,
            &format!("snapshot-{snapshot_id}-read.ok"),
        )?;
        Ok(SnapshotCommitDocument::from_json(
            &bytes,
            cloud_game_id,
            snapshot_id,
        )?)
    }

    fn ensure_remote_zip_exists(&self, commit: &SnapshotCommitDocument) -> CloudSyncResult<()> {
        let remote_zip = snapshot_zip_path(&commit.cloud_game_id, &commit.snapshot_id)?;
        let info = self
            .cloud_store
            .stat_file(&remote_zip)?
            .ok_or_else(|| CloudSyncError::RemoteZipMissing(commit.snapshot_id.clone()))?;
        if info.size != commit.archive.size {
            return Err(CloudArchiveError::ArchiveSizeMismatch.into());
        }
        Ok(())
    }

    fn discovery_status(
        &self,
        commit: &SnapshotCommitDocument,
    ) -> CloudSyncResult<CloudSyncStatus> {
        if let Some(existing) = self
            .repo
            .get_cloud_snapshot(&self.account_id, &commit.snapshot_id)?
        {
            if !record_matches_commit(&existing, commit) {
                return Err(CloudSyncError::SnapshotIdConflict(
                    commit.snapshot_id.clone(),
                ));
            }
            if matches!(
                existing.sync_status,
                CloudSyncStatus::Uploaded | CloudSyncStatus::Downloaded | CloudSyncStatus::Ignored
            ) {
                return Ok(existing.sync_status);
            }
        }
        if let Some(local) = self.repo.get_snapshot(&commit.snapshot_id)? {
            if !snapshot_matches_commit(&local, commit)? {
                return Err(CloudSyncError::SnapshotIdConflict(
                    commit.snapshot_id.clone(),
                ));
            }
            return Ok(CloudSyncStatus::Downloaded);
        }
        Ok(CloudSyncStatus::RemoteOnly)
    }

    fn materialize_cloud_game(&self, document: &CloudGameDocument) -> CloudSyncResult<()> {
        let created_at = normalize_timestamp(&document.created_at)?;
        let updated_at = normalize_timestamp(&document.updated_at)?;
        let binding = self
            .repo
            .get_cloud_game_binding(&self.account_id, &document.cloud_game_id)?;
        match self.repo.get_game(&document.cloud_game_id)? {
            Some(mut game) => {
                if binding
                    .as_ref()
                    .is_none_or(|value| document.revision > value.remote_revision)
                {
                    game.name = document.name.clone();
                    game.updated_at = updated_at.clone();
                    self.repo.update_game(game)?;
                }
            }
            None => {
                self.repo.insert_game(Game {
                    id: document.cloud_game_id.clone(),
                    name: document.name.clone(),
                    icon: None,
                    repo_path: PathBuf::new(),
                    save_paths: Vec::new(),
                    created_at,
                    updated_at,
                })?;
            }
        }
        self.upsert_binding(document)
    }

    fn ensure_local_game_from_cloud(&self, cloud_game_id: &str) -> CloudSyncResult<Game> {
        if let Some(game) = self.repo.get_game(cloud_game_id)? {
            return Ok(game);
        }
        let document = self.read_game_document(cloud_game_id)?;
        self.materialize_cloud_game(&document)?;
        self.repo
            .get_game(cloud_game_id)?
            .ok_or_else(|| CloudSyncError::InvalidState("云端游戏落地失败".into()))
    }

    fn upsert_binding(&self, document: &CloudGameDocument) -> CloudSyncResult<()> {
        self.repo.upsert_cloud_game_binding(CloudGameBinding {
            account_id: self.account_id.clone(),
            cloud_game_id: document.cloud_game_id.clone(),
            local_game_id: document.cloud_game_id.clone(),
            remote_revision: document.revision,
            sync_enabled: true,
            last_scanned_at: Some(self.now_timestamp()?),
        })?;
        Ok(())
    }

    fn read_remote_bytes(&self, remote_path: &str, local_name: &str) -> CloudSyncResult<Vec<u8>> {
        let local_path = self.metadata_temp_path(local_name)?;
        self.cloud_store.get_file(remote_path, &local_path)?;
        let bytes = fs::read(&local_path).map_err(io_error)?;
        let _ = fs::remove_file(local_path);
        Ok(bytes)
    }

    fn metadata_temp_path(&self, name: &str) -> CloudSyncResult<PathBuf> {
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            return Err(CloudSyncError::Io("非法临时文件名".into()));
        }
        let directory = self.work_dir.join("metadata");
        fs::create_dir_all(&directory).map_err(io_error)?;
        Ok(directory.join(name))
    }

    fn prepare_operation_dir(&self, kind: &str, snapshot_id: &str) -> CloudSyncResult<PathBuf> {
        crate::cloud_protocol::validate_id(snapshot_id, "snapshot_id")?;
        let directory = self.work_dir.join(kind).join(snapshot_id);
        if directory.exists() {
            fs::remove_dir_all(&directory).map_err(io_error)?;
        }
        fs::create_dir_all(&directory).map_err(io_error)?;
        Ok(directory)
    }

    fn cleanup_operation_dir(&self, kind: &str, snapshot_id: &str) {
        if crate::cloud_protocol::validate_id(snapshot_id, "snapshot_id").is_ok() {
            let _ = fs::remove_dir_all(self.work_dir.join(kind).join(snapshot_id));
        }
    }

    fn now_timestamp(&self) -> CloudSyncResult<String> {
        Ok(normalize_timestamp(&self.clock.now_stamp())?)
    }

    fn mark_snapshot_error(&self, snapshot_id: &str, error: &CloudSyncError) {
        if self
            .repo
            .get_cloud_snapshot(&self.account_id, snapshot_id)
            .ok()
            .flatten()
            .is_some()
        {
            let _ = self.repo.update_cloud_snapshot_status(
                &self.account_id,
                snapshot_id,
                CloudSyncStatus::Error,
                None,
                Some(error.code()),
            );
        }
    }
}

fn record_from_commit(
    account_id: &str,
    commit: &SnapshotCommitDocument,
    sync_status: CloudSyncStatus,
    last_synced_at: Option<String>,
) -> CloudSnapshotRecord {
    CloudSnapshotRecord {
        account_id: account_id.into(),
        cloud_game_id: commit.cloud_game_id.clone(),
        snapshot_id: commit.snapshot_id.clone(),
        created_at: commit.created_at.clone(),
        reason: commit.reason().unwrap_or(crate::model::Reason::Manual),
        note: commit.note.clone(),
        locked: commit.locked,
        file_count: commit.file_count,
        total_size: commit.total_size,
        source_count: commit.source_count,
        content_hash: commit.content_hash.value.clone(),
        archive_size: commit.archive.size,
        archive_sha256: commit.archive.sha256.clone(),
        published_at: commit.published_at.clone(),
        created_by_device_id: commit.created_by_device_id.clone(),
        sync_status,
        last_synced_at,
        last_error_code: None,
    }
}

fn commit_matches_local(
    commit: &SnapshotCommitDocument,
    snapshot: &Snapshot,
    normalized_created_at: &str,
) -> bool {
    commit.snapshot_id == snapshot.id
        && commit.cloud_game_id == snapshot.game_id
        && crate::timestamp::same_instant(&commit.created_at, normalized_created_at)
        && commit.reason == reason_to_protocol(snapshot.reason)
        && commit.file_count == snapshot.file_count
        && commit.total_size == snapshot.total_size
        && commit.source_count == snapshot.source_count
        && commit.content_hash.value == snapshot.content_hash
}

fn snapshot_matches_commit(
    snapshot: &Snapshot,
    commit: &SnapshotCommitDocument,
) -> CloudSyncResult<bool> {
    Ok(snapshot.id == commit.snapshot_id
        && snapshot.game_id == commit.cloud_game_id
        && crate::timestamp::same_instant(&snapshot.created_at, &commit.created_at)
        && reason_to_protocol(snapshot.reason) == commit.reason
        && snapshot.file_count == commit.file_count
        && snapshot.total_size == commit.total_size
        && snapshot.source_count == commit.source_count
        && snapshot.content_hash == commit.content_hash.value)
}

fn record_matches_commit(record: &CloudSnapshotRecord, commit: &SnapshotCommitDocument) -> bool {
    record.cloud_game_id == commit.cloud_game_id
        && record.snapshot_id == commit.snapshot_id
        && crate::timestamp::same_instant(&record.created_at, &commit.created_at)
        && reason_to_protocol(record.reason) == commit.reason
        && record.file_count == commit.file_count
        && record.total_size == commit.total_size
        && record.source_count == commit.source_count
        && record.content_hash == commit.content_hash.value
        && record.archive_size == commit.archive.size
        && record.archive_sha256 == commit.archive.sha256
}

fn ensure_scan_matches_snapshot(scan: &ScanResult, snapshot: &Snapshot) -> CloudSyncResult<()> {
    if scan.file_count != snapshot.file_count
        || scan.total_size != snapshot.total_size
        || scan.content_hash != snapshot.content_hash
    {
        return Err(SaveLinkError::SnapshotCorrupt.into());
    }
    Ok(())
}

fn snapshot_payload_sources(root: &Path, source_count: u32) -> Vec<PathBuf> {
    if source_count <= 1 {
        return vec![root.to_path_buf()];
    }
    (0..source_count)
        .map(|index| root.join("sources").join(index.to_string()))
        .collect()
}

fn write_bytes(path: &Path, bytes: &[u8]) -> CloudSyncResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    fs::write(path, bytes).map_err(io_error)
}

fn io_error(error: std::io::Error) -> CloudSyncError {
    CloudSyncError::Io(error.to_string())
}
