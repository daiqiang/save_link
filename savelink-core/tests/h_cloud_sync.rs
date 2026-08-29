//! H 组：设备 A -> Fake 云端 -> 设备 B 的协议级闭环。

use savelink_core::cloud_archive::{
    CloudArchiveCodec, CloudArchiveError, SnapshotContentExpectation, ZipCloudArchiveCodec,
};
use savelink_core::cloud_model::{
    CloudAccount, CloudMetadataSyncStatus, CloudSnapshotMetadataState, CloudSyncStatus,
};
use savelink_core::cloud_protocol::{
    game_path, snapshot_metadata_path, snapshot_ok_path, snapshot_zip_path, CloudGameDocument,
    CloudManifest, SnapshotCommitDocument, SnapshotMetadataDocument,
};
use savelink_core::cloud_repo::CloudStateRepository;
use savelink_core::cloud_service::{
    CloudSyncError, CloudSyncService, ReceiveOutcome, UploadOutcome,
};
use savelink_core::cloud_store::{
    CloudEntry, CloudFile, CloudObjectStore, CloudStoreError, CloudStoreResult,
    FakeCloudObjectStore, PutMode,
};
use savelink_core::error::SaveLinkError;
use savelink_core::model::{
    EmulatorGameIdentity, Game, GameLaunchBinding, Reason, RomIdentity, ScanResult, Snapshot,
    SnapshotStatus,
};
use savelink_core::repo::{Clock, Repository};
use savelink_core::scan;
use savelink_core::sqlite_repo::SqliteRepo;
use savelink_core::store::{FsStore, SnapshotStore};
use savelink_core::testkit::TempDir;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

struct FixedClock(&'static str);

impl Clock for FixedClock {
    fn now_stamp(&self) -> String {
        self.0.into()
    }
}

struct Device {
    repo: Arc<SqliteRepo>,
    store: Arc<FsStore>,
    service: CloudSyncService<SqliteRepo>,
    root: PathBuf,
}

struct DeleteFailingCloudStore {
    inner: Arc<dyn CloudObjectStore>,
}

struct PutFailingCloudStore {
    inner: Arc<dyn CloudObjectStore>,
}

impl CloudObjectStore for PutFailingCloudStore {
    fn put_file(
        &self,
        _remote_path: &str,
        _local_file: &Path,
        _mode: PutMode,
    ) -> CloudStoreResult<CloudFile> {
        Err(CloudStoreError::NetworkUnavailable)
    }

    fn get_file(&self, remote_path: &str, local_file: &Path) -> CloudStoreResult<()> {
        self.inner.get_file(remote_path, local_file)
    }

    fn list_directory(&self, remote_path: &str) -> CloudStoreResult<Vec<CloudEntry>> {
        self.inner.list_directory(remote_path)
    }

    fn stat_file(&self, remote_path: &str) -> CloudStoreResult<Option<CloudFile>> {
        self.inner.stat_file(remote_path)
    }

    fn delete_file(&self, remote_path: &str) -> CloudStoreResult<()> {
        self.inner.delete_file(remote_path)
    }
}

impl CloudObjectStore for DeleteFailingCloudStore {
    fn put_file(
        &self,
        remote_path: &str,
        local_file: &Path,
        mode: PutMode,
    ) -> CloudStoreResult<CloudFile> {
        self.inner.put_file(remote_path, local_file, mode)
    }

    fn get_file(&self, remote_path: &str, local_file: &Path) -> CloudStoreResult<()> {
        self.inner.get_file(remote_path, local_file)
    }

    fn list_directory(&self, remote_path: &str) -> CloudStoreResult<Vec<CloudEntry>> {
        self.inner.list_directory(remote_path)
    }

    fn stat_file(&self, remote_path: &str) -> CloudStoreResult<Option<CloudFile>> {
        self.inner.stat_file(remote_path)
    }

    fn delete_file(&self, _remote_path: &str) -> CloudStoreResult<()> {
        Err(CloudStoreError::NetworkUnavailable)
    }
}

fn create_device(
    root: PathBuf,
    name: &str,
    cloud: Arc<dyn CloudObjectStore>,
    codec: Arc<dyn CloudArchiveCodec>,
) -> Device {
    fs::create_dir_all(&root).unwrap();
    let repo = Arc::new(SqliteRepo::open(root.join("savelink.db")).unwrap());
    repo.upsert_cloud_account(CloudAccount {
        id: "account_1".into(),
        provider: "fake".into(),
        account_identity: Some("fake-user".into()),
        display_name: Some("Fake Cloud".into()),
        token_ref: None,
        created_at: "2026-07-14T18:00:00+08:00".into(),
        updated_at: "2026-07-14T18:00:00+08:00".into(),
    })
    .unwrap();
    repo.set_setting("device_id", &format!("device_{name}"))
        .unwrap();
    let store = Arc::new(FsStore::new(root.join("repository")));
    let service = CloudSyncService::new(
        repo.clone(),
        store.clone(),
        cloud,
        codec,
        Arc::new(FixedClock("2026-07-14T18:30:00+08:00")),
        root.join("cloud-work"),
        "account_1",
        format!("device_{name}"),
        format!("repo_{name}"),
    )
    .unwrap();
    Device {
        repo,
        store,
        service,
        root,
    }
}

fn seed_device_a(device: &Device, contents: &[(&str, &[u8])]) -> PathBuf {
    let save_dir = device.root.join("real-save");
    write_files(&save_dir, contents);
    device
        .repo
        .insert_game(Game {
            id: "game_1".into(),
            name: "Elden Ring".into(),
            icon: None,
            repo_path: PathBuf::new(),
            save_paths: vec![save_dir.clone()],
            save_sources: Vec::new(),
            emulator_identity: None,
            emulator_binding: None,
            launch_binding: None,
            created_at: "2026-07-14T18:00:00+08:00".into(),
            updated_at: "2026-07-14T18:00:00+08:00".into(),
        })
        .unwrap();
    let scan_result = scan::fingerprint_dir(&save_dir).unwrap();
    let stored = device
        .store
        .create("snap_1", std::slice::from_ref(&save_dir), &scan_result)
        .unwrap();
    device
        .repo
        .insert_snapshot(Snapshot {
            id: "snap_1".into(),
            game_id: "game_1".into(),
            created_at: "2026-07-14T18:10:00+08:00".into(),
            note: Some("Boss 前备份".into()),
            note_updated_at: "2026-07-14T18:10:00+08:00".into(),
            reason: Reason::Manual,
            locked: true,
            locked_updated_at: "2026-07-14T18:10:00+08:00".into(),
            display_zone: savelink_core::model::SnapshotDisplayZone::Locked,
            file_count: scan_result.file_count,
            total_size: scan_result.total_size,
            source_count: 1,
            content_hash: scan_result.content_hash,
            storage_key: stored.storage_key,
            status: SnapshotStatus::Complete,
        })
        .unwrap();
    save_dir
}

fn seed_multi_source_device_a(device: &Device) -> (Vec<PathBuf>, ScanResult) {
    let sources = vec![
        device.root.join("real-save-0"),
        device.root.join("real-save-1"),
    ];
    write_files(
        &sources[0],
        &[("slot/save.dat", b"SAVE-ROOT-0"), ("same.dat", b"ZERO")],
    );
    write_files(
        &sources[1],
        &[("profile.dat", b"SAVE-ROOT-1"), ("same.dat", b"ONE")],
    );
    device
        .repo
        .insert_game(Game {
            id: "game_1".into(),
            name: "Multi Root Game".into(),
            icon: None,
            repo_path: PathBuf::new(),
            save_paths: sources.clone(),
            save_sources: Vec::new(),
            emulator_identity: None,
            emulator_binding: None,
            launch_binding: None,
            created_at: "2026-07-14T18:00:00+08:00".into(),
            updated_at: "2026-07-14T18:00:00+08:00".into(),
        })
        .unwrap();
    let scan_result = scan::scan(&sources).unwrap();
    let stored = device
        .store
        .create("snap_1", &sources, &scan_result)
        .unwrap();
    device
        .repo
        .insert_snapshot(Snapshot {
            id: "snap_1".into(),
            game_id: "game_1".into(),
            created_at: "2026-07-14T18:10:00+08:00".into(),
            note: Some("multi-source snapshot".into()),
            note_updated_at: "2026-07-14T18:10:00+08:00".into(),
            reason: Reason::Manual,
            locked: false,
            locked_updated_at: "2026-07-14T18:10:00+08:00".into(),
            display_zone: savelink_core::model::SnapshotDisplayZone::Normal,
            file_count: scan_result.file_count,
            total_size: scan_result.total_size,
            source_count: 2,
            content_hash: scan_result.content_hash.clone(),
            storage_key: stored.storage_key,
            status: SnapshotStatus::Complete,
        })
        .unwrap();
    (sources, scan_result)
}

fn setup() -> (
    TempDir,
    Arc<dyn CloudObjectStore>,
    Arc<dyn CloudArchiveCodec>,
    Device,
    Device,
) {
    let tmp = TempDir::new();
    let cloud: Arc<dyn CloudObjectStore> =
        Arc::new(FakeCloudObjectStore::new(tmp.path().join("fake-cloud")).unwrap());
    let codec: Arc<dyn CloudArchiveCodec> = Arc::new(ZipCloudArchiveCodec::new());
    let device_a = create_device(
        tmp.path().join("device-a"),
        "a",
        cloud.clone(),
        codec.clone(),
    );
    let device_b = create_device(
        tmp.path().join("device-b"),
        "b",
        cloud.clone(),
        codec.clone(),
    );
    (tmp, cloud, codec, device_a, device_b)
}

fn service_for_device(
    device: &Device,
    cloud: Arc<dyn CloudObjectStore>,
) -> CloudSyncService<SqliteRepo> {
    CloudSyncService::new(
        device.repo.clone(),
        device.store.clone(),
        cloud,
        Arc::new(ZipCloudArchiveCodec::new()),
        Arc::new(FixedClock("2026-07-14T18:30:00+08:00")),
        device.root.join("cloud-work-delete"),
        "account_1",
        "device_a",
        "repo_a",
    )
    .unwrap()
}

fn unlock_snapshot(device: &Device, snapshot_id: &str) {
    let mut snapshot = device.repo.get_snapshot(snapshot_id).unwrap().unwrap();
    snapshot.locked = false;
    device.repo.update_snapshot(snapshot).unwrap();
}

fn edit_snapshot_metadata(
    device: &Device,
    snapshot_id: &str,
    note: Option<&str>,
    note_updated_at: Option<&str>,
    locked: Option<bool>,
    locked_updated_at: Option<&str>,
) {
    let mut snapshot = device.repo.get_snapshot(snapshot_id).unwrap().unwrap();
    if let Some(value) = note {
        snapshot.note = Some(value.into());
        snapshot.note_updated_at = note_updated_at.unwrap().into();
    }
    if let Some(value) = locked {
        snapshot.locked = value;
        snapshot.locked_updated_at = locked_updated_at.unwrap().into();
    }
    device.repo.update_snapshot(snapshot).unwrap();
    device
        .repo
        .update_cloud_snapshot_metadata_status(
            "account_1",
            snapshot_id,
            CloudSnapshotMetadataState {
                status: CloudMetadataSyncStatus::Pending,
                last_synced_at: None,
                last_error_code: None,
                remote_note_updated_at: None,
                remote_locked_updated_at: None,
            },
        )
        .unwrap();
}

#[test]
fn h1_protocol_json_roundtrip_and_path_mismatch_are_checked() {
    let manifest = CloudManifest {
        protocol: "savelink-cloud-snapshot".into(),
        protocol_version: 1,
        repository_id: "repo_1".into(),
        created_at: "2026-07-14T18:00:00+08:00".into(),
        created_by_device_id: "device_a".into(),
    };
    assert_eq!(
        CloudManifest::from_json(&manifest.to_json().unwrap()).unwrap(),
        manifest
    );

    let (_, _, _, device_a, _) = setup();
    seed_device_a(&device_a, &[("ER0000.sl2", b"save-v1")]);
    device_a
        .service
        .upload_snapshot("game_1", "snap_1")
        .unwrap();
    let ok_local = device_a.root.join("read.ok");
    let cloud_path = snapshot_ok_path("game_1", "snap_1").unwrap();
    device_a
        .service
        .ensure_manifest()
        .expect("manifest remains valid");
    let fake_root = device_a.root.parent().unwrap().join("fake-cloud");
    fs::copy(fake_root.join(&cloud_path), &ok_local).unwrap();
    let bytes = fs::read(ok_local).unwrap();
    assert!(SnapshotCommitDocument::from_json(&bytes, "other_game", "snap_1").is_err());
}

#[test]
fn h2_archive_roundtrip_and_hash_tampering_are_detected() {
    let tmp = TempDir::new();
    let source = tmp.path().join("source");
    write_files(
        &source,
        &[("ER0000.sl2", b"main-save"), ("sub/backup.bin", b"backup")],
    );
    let expected_scan = scan::fingerprint_dir(&source).unwrap();
    let archive = tmp.path().join("snapshot.zip");
    let codec = ZipCloudArchiveCodec::new();
    let info = codec.create_archive(&source, &archive).unwrap();
    codec
        .verify_archive(&archive, info.size, &info.sha256)
        .unwrap();
    let extracted = tmp.path().join("extracted");
    let actual = codec
        .extract_verified(
            &archive,
            &extracted,
            &SnapshotContentExpectation {
                file_count: expected_scan.file_count,
                total_size: expected_scan.total_size,
                content_hash: expected_scan.content_hash.clone(),
                source_count: 1,
            },
        )
        .unwrap();
    assert_eq!(actual, expected_scan);

    let mut bytes = fs::read(&archive).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    fs::write(&archive, bytes).unwrap();
    assert!(matches!(
        codec.verify_archive(&archive, info.size, &info.sha256),
        Err(CloudArchiveError::ArchiveHashMismatch)
    ));
}

#[test]
fn h3_unsafe_zip_entry_is_rejected_without_writing_outside_target() {
    let tmp = TempDir::new();
    let archive_path = tmp.path().join("unsafe.zip");
    let file = fs::File::create(&archive_path).unwrap();
    let mut writer = ZipWriter::new(file);
    writer
        .start_file("../escape.sav", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"bad").unwrap();
    writer.finish().unwrap();

    let codec = ZipCloudArchiveCodec::new();
    let result = codec.extract_verified(
        &archive_path,
        &tmp.path().join("out"),
        &SnapshotContentExpectation {
            file_count: 1,
            total_size: 3,
            content_hash: "0000000000000000".into(),
            source_count: 1,
        },
    );
    assert!(matches!(result, Err(CloudArchiveError::UnsafeEntry(_))));
    assert!(!tmp.path().join("escape.sav").exists());
}

#[test]
fn h4_device_a_uploads_and_device_b_discovers_downloads_and_lands_snapshot() {
    let (_tmp, cloud, _codec, device_a, device_b) = setup();
    let save_a = seed_device_a(
        &device_a,
        &[
            ("ER0000.sl2", b"progress-42"),
            ("ER0000.sl2.bak", b"backup"),
        ],
    );
    let source_scan = scan::fingerprint_dir(&save_a).unwrap();
    let untouched_b = device_b.root.join("real-save-not-bound");
    write_files(&untouched_b, &[("current.sav", b"do-not-touch")]);
    let untouched_before = scan::fingerprint_dir(&untouched_b).unwrap();

    assert_eq!(
        device_a
            .service
            .upload_snapshot("game_1", "snap_1")
            .unwrap(),
        UploadOutcome::Uploaded
    );
    assert_eq!(
        device_a
            .service
            .upload_snapshot("game_1", "snap_1")
            .unwrap(),
        UploadOutcome::AlreadyPresent
    );
    assert_eq!(scan::fingerprint_dir(&save_a).unwrap(), source_scan);
    assert!(cloud
        .stat_file(&snapshot_zip_path("game_1", "snap_1").unwrap())
        .unwrap()
        .is_some());
    assert!(cloud
        .stat_file(&snapshot_ok_path("game_1", "snap_1").unwrap())
        .unwrap()
        .is_some());

    let discovered = device_b.service.discover_remote_catalog().unwrap();
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].game_name, "Elden Ring");
    assert_eq!(
        discovered[0].snapshot.sync_status,
        CloudSyncStatus::RemoteOnly
    );
    assert_eq!(
        device_b.service.discover_remote_snapshots().unwrap().len(),
        1
    );
    assert!(
        device_b.repo.get_game("game_1").unwrap().is_none(),
        "仅发现云端目录时不能污染本机游戏列表"
    );

    assert_eq!(
        device_b.service.receive_remote_snapshot("snap_1").unwrap(),
        ReceiveOutcome::Downloaded
    );
    let cloud_game = device_b.repo.get_game("game_1").unwrap().unwrap();
    assert!(
        cloud_game.save_paths.is_empty(),
        "云端游戏不能携带设备 A 的路径"
    );
    assert_eq!(
        device_b.service.receive_remote_snapshot("snap_1").unwrap(),
        ReceiveOutcome::AlreadyPresent
    );
    let local = device_b.repo.get_snapshot("snap_1").unwrap().unwrap();
    assert_eq!(local.status, SnapshotStatus::Complete);
    assert_eq!(
        local.created_at, "2026-07-14T10:10:00Z",
        "设备 B 落地时应使用统一 UTC 时间"
    );
    assert!(device_b.store.verify(&local.storage_key).unwrap());
    let landed = device_b.root.join("landed-copy");
    device_b.store.restore(&local.storage_key, &landed).unwrap();
    assert_eq!(scan::fingerprint_dir(&landed).unwrap(), source_scan);
    assert_eq!(
        scan::fingerprint_dir(&untouched_b).unwrap(),
        untouched_before
    );
}

#[test]
fn h5_orphan_zip_is_ignored_during_discovery() {
    let (_tmp, cloud, _codec, device_a, device_b) = setup();
    seed_device_a(&device_a, &[("ER0000.sl2", b"save-v1")]);
    device_a
        .service
        .upload_snapshot("game_1", "snap_1")
        .unwrap();
    cloud
        .delete_file(&snapshot_ok_path("game_1", "snap_1").unwrap())
        .unwrap();

    let discovered = device_b.service.discover_remote_snapshots().unwrap();
    assert!(discovered.is_empty());
    assert!(device_b.repo.get_snapshot("snap_1").unwrap().is_none());
}

#[test]
fn h6_same_size_zip_tampering_is_rejected_before_extraction() {
    let (tmp, _cloud, _codec, device_a, device_b) = setup();
    seed_device_a(&device_a, &[("ER0000.sl2", b"save-v1")]);
    device_a
        .service
        .upload_snapshot("game_1", "snap_1")
        .unwrap();

    let remote_zip = tmp
        .path()
        .join("fake-cloud")
        .join(snapshot_zip_path("game_1", "snap_1").unwrap());
    let mut bytes = fs::read(&remote_zip).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0x01;
    fs::write(&remote_zip, bytes).unwrap();

    device_b.service.discover_remote_snapshots().unwrap();
    let error = device_b
        .service
        .receive_remote_snapshot("snap_1")
        .unwrap_err();
    assert_eq!(error.code(), "archive_hash_mismatch");
    assert!(device_b.repo.get_snapshot("snap_1").unwrap().is_none());
    assert_eq!(
        device_b
            .repo
            .get_cloud_snapshot("account_1", "snap_1")
            .unwrap()
            .unwrap()
            .sync_status,
        CloudSyncStatus::Error
    );
}

#[test]
fn h7_valid_zip_with_wrong_snapshot_content_is_rejected_after_extraction() {
    let (tmp, cloud, codec, device_a, device_b) = setup();
    seed_device_a(&device_a, &[("ER0000.sl2", b"original")]);
    device_a
        .service
        .upload_snapshot("game_1", "snap_1")
        .unwrap();

    let replacement_dir = tmp.path().join("replacement");
    write_files(&replacement_dir, &[("ER0000.sl2", b"different-and-longer")]);
    let replacement_zip = tmp.path().join("replacement.zip");
    let replacement_info = codec
        .create_archive(&replacement_dir, &replacement_zip)
        .unwrap();
    let ok_download = tmp.path().join("commit.ok");
    cloud
        .get_file(&snapshot_ok_path("game_1", "snap_1").unwrap(), &ok_download)
        .unwrap();
    let mut commit =
        SnapshotCommitDocument::from_json(&fs::read(&ok_download).unwrap(), "game_1", "snap_1")
            .unwrap();
    commit.archive.size = replacement_info.size;
    commit.archive.sha256 = replacement_info.sha256;
    fs::write(&ok_download, commit.to_json().unwrap()).unwrap();
    cloud
        .put_file(
            &snapshot_zip_path("game_1", "snap_1").unwrap(),
            &replacement_zip,
            PutMode::Overwrite,
        )
        .unwrap();
    cloud
        .put_file(
            &snapshot_ok_path("game_1", "snap_1").unwrap(),
            &ok_download,
            PutMode::Overwrite,
        )
        .unwrap();

    device_b.service.discover_remote_snapshots().unwrap();
    let error = device_b
        .service
        .receive_remote_snapshot("snap_1")
        .unwrap_err();
    assert_eq!(error.code(), "snapshot_content_mismatch");
    assert!(device_b.repo.get_snapshot("snap_1").unwrap().is_none());
}

#[test]
fn h8_same_snapshot_id_with_different_local_content_is_a_hard_conflict() {
    let (_tmp, _cloud, _codec, device_a, device_b) = setup();
    seed_device_a(&device_a, &[("ER0000.sl2", b"device-a")]);
    device_a
        .service
        .upload_snapshot("game_1", "snap_1")
        .unwrap();
    seed_device_a(&device_b, &[("ER0000.sl2", b"device-b-different")]);

    let error = device_b.service.discover_remote_snapshots().unwrap_err();
    assert!(matches!(error, CloudSyncError::SnapshotIdConflict(id) if id == "snap_1"));
    let local = device_b.repo.get_snapshot("snap_1").unwrap().unwrap();
    let restored = device_b.root.join("conflict-local-copy");
    device_b
        .store
        .restore(&local.storage_key, &restored)
        .unwrap();
    assert_eq!(
        fs::read(restored.join("ER0000.sl2")).unwrap(),
        b"device-b-different"
    );
}

#[test]
fn h9_retention_delete_removes_remote_objects_before_local_snapshot() {
    let (_tmp, cloud, _codec, device_a, _) = setup();
    seed_device_a(&device_a, &[("ER0000.sl2", b"save-v1")]);
    device_a
        .service
        .upload_snapshot("game_1", "snap_1")
        .unwrap();
    unlock_snapshot(&device_a, "snap_1");

    device_a
        .service
        .delete_snapshot_everywhere("snap_1")
        .unwrap();

    assert!(cloud
        .stat_file(&snapshot_ok_path("game_1", "snap_1").unwrap())
        .unwrap()
        .is_none());
    assert!(cloud
        .stat_file(&snapshot_zip_path("game_1", "snap_1").unwrap())
        .unwrap()
        .is_none());
    assert!(device_a.repo.get_snapshot("snap_1").unwrap().is_none());
    assert!(device_a
        .repo
        .get_cloud_snapshot("account_1", "snap_1")
        .unwrap()
        .is_none());
}

#[test]
fn h10_remote_delete_failure_keeps_local_data_and_can_be_retried() {
    let (_tmp, cloud, _codec, device_a, _) = setup();
    seed_device_a(&device_a, &[("ER0000.sl2", b"save-v1")]);
    device_a
        .service
        .upload_snapshot("game_1", "snap_1")
        .unwrap();
    unlock_snapshot(&device_a, "snap_1");
    let failing_cloud: Arc<dyn CloudObjectStore> = Arc::new(DeleteFailingCloudStore {
        inner: cloud.clone(),
    });
    let failing_service = service_for_device(&device_a, failing_cloud);

    let error = failing_service
        .delete_snapshot_everywhere("snap_1")
        .unwrap_err();
    assert_eq!(error.code(), "network_unavailable");
    let local = device_a.repo.get_snapshot("snap_1").unwrap().unwrap();
    assert_eq!(local.status, SnapshotStatus::Complete);
    assert!(device_a.store.verify(&local.storage_key).unwrap());
    let cloud_record = device_a
        .repo
        .get_cloud_snapshot("account_1", "snap_1")
        .unwrap()
        .unwrap();
    assert_eq!(cloud_record.sync_status, CloudSyncStatus::DeleteFailed);
    assert_eq!(
        cloud_record.last_error_code.as_deref(),
        Some("network_unavailable")
    );

    service_for_device(&device_a, cloud)
        .delete_snapshot_everywhere("snap_1")
        .expect("网络恢复后应可幂等重试删除");
    assert!(device_a.repo.get_snapshot("snap_1").unwrap().is_none());
    assert!(device_a
        .repo
        .get_cloud_snapshot("account_1", "snap_1")
        .unwrap()
        .is_none());
}

#[test]
fn h11_existing_offset_marker_is_idempotent_with_canonical_local_time() {
    let (tmp, _cloud, _codec, device_a, _) = setup();
    seed_device_a(&device_a, &[("ER0000.sl2", b"save-v1")]);
    device_a
        .service
        .upload_snapshot("game_1", "snap_1")
        .unwrap();

    let marker_path = tmp
        .path()
        .join("fake-cloud")
        .join(snapshot_ok_path("game_1", "snap_1").unwrap());
    let mut marker =
        SnapshotCommitDocument::from_json(&fs::read(&marker_path).unwrap(), "game_1", "snap_1")
            .unwrap();
    marker.created_at = "2026-07-14T18:10:00+08:00".into();
    fs::write(&marker_path, marker.to_json().unwrap()).unwrap();

    assert_eq!(
        device_a
            .service
            .upload_snapshot("game_1", "snap_1")
            .expect("同一时间点的旧云端标记不得产生 ID 冲突"),
        UploadOutcome::AlreadyPresent
    );
}

#[test]
fn h12_legacy_cloud_marker_without_source_count_remains_readable() {
    let (_tmp, cloud, _codec, device_a, _) = setup();
    seed_device_a(&device_a, &[("ER0000.sl2", b"save-v1")]);
    device_a
        .service
        .upload_snapshot("game_1", "snap_1")
        .unwrap();

    let marker = device_a.root.join("legacy-marker.ok");
    cloud
        .get_file(&snapshot_ok_path("game_1", "snap_1").unwrap(), &marker)
        .unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(marker).unwrap()).unwrap();
    value.as_object_mut().unwrap().remove("source_count");
    let legacy =
        SnapshotCommitDocument::from_json(&serde_json::to_vec(&value).unwrap(), "game_1", "snap_1")
            .unwrap();
    assert_eq!(legacy.source_count, 1);
    assert_eq!(legacy.archive.layout_version, 1);
}

#[test]
fn h13_multi_source_snapshot_round_trips_through_fake_cloud() {
    let (_tmp, cloud, _codec, device_a, device_b) = setup();
    let (device_a_sources, expected) = seed_multi_source_device_a(&device_a);

    assert_eq!(
        device_a
            .service
            .upload_snapshot("game_1", "snap_1")
            .unwrap(),
        UploadOutcome::Uploaded
    );
    let marker_path = device_a.root.join("multi-marker.ok");
    cloud
        .get_file(&snapshot_ok_path("game_1", "snap_1").unwrap(), &marker_path)
        .unwrap();
    let commit =
        SnapshotCommitDocument::from_json(&fs::read(marker_path).unwrap(), "game_1", "snap_1")
            .unwrap();
    assert_eq!(commit.source_count, 2);
    assert_eq!(commit.archive.layout_version, 2);
    assert_eq!(
        commit.content_hash.algorithm,
        "savelink-fnv1a64-multi-tree-v1"
    );
    let serialized = String::from_utf8(commit.to_json().unwrap()).unwrap();
    for source in &device_a_sources {
        assert!(
            !serialized.contains(&source.to_string_lossy().to_string()),
            "云端标记不得携带设备 A 的绝对路径"
        );
    }

    let mut wrong_layout = commit.clone();
    wrong_layout.archive.layout_version = 1;
    assert!(wrong_layout.validate("game_1", "snap_1").is_err());

    let discovered = device_b.service.discover_remote_catalog().unwrap();
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].snapshot.source_count, 2);
    assert_eq!(
        device_b.service.receive_remote_snapshot("snap_1").unwrap(),
        ReceiveOutcome::Downloaded
    );
    let local = device_b.repo.get_snapshot("snap_1").unwrap().unwrap();
    assert_eq!(local.source_count, 2);
    assert!(device_b.store.verify(&local.storage_key).unwrap());

    let restored = vec![
        device_b.root.join("restored-0"),
        device_b.root.join("restored-1"),
    ];
    device_b
        .store
        .restore_sources(&local.storage_key, &restored)
        .unwrap();
    assert_eq!(scan::scan(&restored).unwrap(), expected);
    assert_eq!(fs::read(restored[0].join("same.dat")).unwrap(), b"ZERO");
    assert_eq!(fs::read(restored[1].join("same.dat")).unwrap(), b"ONE");
}

#[test]
fn h14_emulator_identity_reaches_device_b_without_local_paths() {
    let (_tmp, _cloud, _codec, device_a, device_b) = setup();
    seed_device_a(&device_a, &[("save.dsv", b"desmume-save")]);
    let identity = EmulatorGameIdentity {
        emulator: "desmume".into(),
        rom: RomIdentity {
            file_name: "zzjb2r ver0.99.nds".into(),
            sha256: "a".repeat(64),
            header_title: "METALMAX2R".into(),
            game_code: "TMXJ".into(),
        },
    };
    let mut game = device_a.repo.get_game("game_1").unwrap().unwrap();
    game.emulator_identity = Some(identity.clone());
    device_a.repo.update_game(game).unwrap();

    device_a
        .service
        .upload_snapshot("game_1", "snap_1")
        .unwrap();
    device_b.service.discover_remote_snapshots().unwrap();
    device_b.service.receive_remote_snapshot("snap_1").unwrap();

    let landed = device_b.repo.get_game("game_1").unwrap().unwrap();
    assert_eq!(landed.emulator_identity, Some(identity));
    assert!(landed.emulator_binding.is_none());
    assert!(landed.save_paths.is_empty());
    assert!(landed.save_sources.is_empty());
}

#[test]
fn h15_existing_game_document_is_upgraded_with_emulator_identity() {
    let (_tmp, cloud, _codec, device_a, device_b) = setup();
    seed_device_a(&device_a, &[("save.dsv", b"desmume-save")]);
    device_a
        .service
        .upload_snapshot("game_1", "snap_1")
        .unwrap();

    let identity = EmulatorGameIdentity {
        emulator: "desmume".into(),
        rom: RomIdentity {
            file_name: "zzjb2r ver0.99.nds".into(),
            sha256: "b".repeat(64),
            header_title: "METALMAX2R".into(),
            game_code: "TMXJ".into(),
        },
    };
    let mut game = device_a.repo.get_game("game_1").unwrap().unwrap();
    game.emulator_identity = Some(identity.clone());
    device_a.repo.update_game(game).unwrap();

    assert_eq!(
        device_a
            .service
            .upload_snapshot("game_1", "snap_1")
            .unwrap(),
        UploadOutcome::AlreadyPresent
    );
    let local_document = device_a.root.join("upgraded-game.json");
    cloud
        .get_file(&game_path("game_1").unwrap(), &local_document)
        .unwrap();
    let document =
        CloudGameDocument::from_json(&fs::read(local_document).unwrap(), "game_1").unwrap();
    assert_eq!(document.revision, 2);
    assert_eq!(document.emulator_identity, Some(identity.clone()));

    device_b.service.discover_remote_snapshots().unwrap();
    device_b.service.receive_remote_snapshot("snap_1").unwrap();
    let landed = device_b.repo.get_game("game_1").unwrap().unwrap();
    assert_eq!(landed.emulator_identity, Some(identity));
    assert!(landed.emulator_binding.is_none());
    assert!(landed.save_paths.is_empty());
    assert!(landed.save_sources.is_empty());
}

#[test]
fn h16_local_launch_paths_never_enter_cloud_game_document() {
    let (_tmp, cloud, _codec, device_a, _device_b) = setup();
    seed_device_a(&device_a, &[("save.dat", b"private-path-test")]);
    let install_dir = device_a.root.join("secret-launch-marker-install");
    fs::create_dir_all(&install_dir).unwrap();
    let executable_path = install_dir.join("secret-launch-marker.exe");
    fs::write(&executable_path, b"fake executable").unwrap();
    let mut game = device_a.repo.get_game("game_1").unwrap().unwrap();
    game.launch_binding = Some(GameLaunchBinding::executable(executable_path, install_dir));
    device_a.repo.update_game(game).unwrap();

    device_a
        .service
        .upload_snapshot("game_1", "snap_1")
        .unwrap();
    let local_document = device_a.root.join("launch-private-game.json");
    cloud
        .get_file(&game_path("game_1").unwrap(), &local_document)
        .unwrap();
    let json = fs::read_to_string(local_document).unwrap();
    assert!(!json.contains("secret-launch-marker"));
    assert!(!json.contains("launch_binding"));
    assert!(!json.contains("executable_path"));
    assert!(!json.contains("install_dir"));
}

#[test]
fn h17_pending_game_cannot_start_manual_cloud_upload() {
    let (_tmp, cloud, _codec, device_a, _device_b) = setup();
    let install_dir = device_a.root.join("pending-install");
    fs::create_dir_all(&install_dir).unwrap();
    let executable_path = install_dir.join("pending.exe");
    fs::write(&executable_path, b"fake executable").unwrap();
    device_a
        .repo
        .insert_game(Game {
            id: "pending_game".into(),
            name: "Pending Game".into(),
            icon: None,
            repo_path: PathBuf::new(),
            save_paths: Vec::new(),
            save_sources: Vec::new(),
            emulator_identity: None,
            emulator_binding: None,
            launch_binding: Some(GameLaunchBinding::executable(executable_path, install_dir)),
            created_at: "2026-08-24T10:00:00Z".into(),
            updated_at: "2026-08-24T10:00:00Z".into(),
        })
        .unwrap();

    let error = device_a
        .service
        .upload_snapshot("pending_game", "missing")
        .unwrap_err();
    assert!(matches!(
        &error,
        CloudSyncError::Local(SaveLinkError::SaveSourcesNotConfigured)
    ));
    assert_eq!(error.code(), "save_sources_not_configured");
    assert!(
        cloud
            .stat_file(&savelink_core::cloud_protocol::manifest_path())
            .unwrap()
            .is_none(),
        "拒绝待配置游戏时不应先写入任何云端对象"
    );
}

#[test]
fn h18_two_devices_merge_name_and_lock_independently() {
    let (_tmp, cloud, _codec, device_a, device_b) = setup();
    seed_device_a(&device_a, &[("save.dat", b"metadata-sync")]);
    let mut initial = device_a.repo.get_snapshot("snap_1").unwrap().unwrap();
    initial.locked = false;
    device_a.repo.update_snapshot(initial).unwrap();
    device_a
        .service
        .upload_snapshot("game_1", "snap_1")
        .unwrap();
    device_b.service.discover_remote_catalog().unwrap();
    device_b.service.receive_remote_snapshot("snap_1").unwrap();

    edit_snapshot_metadata(
        &device_a,
        "snap_1",
        Some("设备 A 的名称"),
        Some("2026-07-14T12:00:00Z"),
        None,
        None,
    );
    device_a.service.sync_snapshot_metadata("snap_1").unwrap();

    edit_snapshot_metadata(
        &device_b,
        "snap_1",
        None,
        None,
        Some(true),
        Some("2026-07-14T12:01:00Z"),
    );
    device_b.service.sync_snapshot_metadata("snap_1").unwrap();
    device_a.service.sync_known_snapshot_metadata().unwrap();

    for device in [&device_a, &device_b] {
        let snapshot = device.repo.get_snapshot("snap_1").unwrap().unwrap();
        assert_eq!(snapshot.note.as_deref(), Some("设备 A 的名称"));
        assert!(snapshot.locked);
        let cloud_record = device
            .repo
            .get_cloud_snapshot("account_1", "snap_1")
            .unwrap()
            .unwrap();
        assert_eq!(
            cloud_record.metadata_sync_status,
            CloudMetadataSyncStatus::Synced
        );
    }

    let local = device_a.root.join("metadata.json");
    cloud
        .get_file(&snapshot_metadata_path("game_1", "snap_1").unwrap(), &local)
        .unwrap();
    let document =
        SnapshotMetadataDocument::from_json(&fs::read(local).unwrap(), "game_1", "snap_1").unwrap();
    assert_eq!(document.note.value.as_deref(), Some("设备 A 的名称"));
    assert!(document.locked.value);
}

#[test]
fn h19_equal_time_conflicts_are_deterministic() {
    let (_tmp, _cloud, _codec, device_a, device_b) = setup();
    seed_device_a(&device_a, &[("save.dat", b"tie")]);
    device_a
        .service
        .upload_snapshot("game_1", "snap_1")
        .unwrap();
    device_b.service.discover_remote_catalog().unwrap();
    device_b.service.receive_remote_snapshot("snap_1").unwrap();

    let tie_time = "2026-07-14T12:00:00Z";
    edit_snapshot_metadata(
        &device_a,
        "snap_1",
        Some("先到云端的名称"),
        Some(tie_time),
        Some(false),
        Some(tie_time),
    );
    edit_snapshot_metadata(
        &device_b,
        "snap_1",
        Some("同时产生的另一名称"),
        Some(tie_time),
        Some(true),
        Some(tie_time),
    );
    device_a.service.sync_snapshot_metadata("snap_1").unwrap();
    device_b.service.sync_snapshot_metadata("snap_1").unwrap();
    device_a.service.sync_known_snapshot_metadata().unwrap();

    for device in [&device_a, &device_b] {
        let snapshot = device.repo.get_snapshot("snap_1").unwrap().unwrap();
        assert_eq!(snapshot.note.as_deref(), Some("先到云端的名称"));
        assert!(snapshot.locked, "同一时刻的锁定冲突必须优先保护数据");
    }
}

#[test]
fn h20_metadata_error_survives_reopen_and_retries() {
    let (_tmp, cloud, _codec, device_a, _) = setup();
    seed_device_a(&device_a, &[("save.dat", b"retry")]);
    device_a
        .service
        .upload_snapshot("game_1", "snap_1")
        .unwrap();
    edit_snapshot_metadata(
        &device_a,
        "snap_1",
        Some("断网期间修改"),
        Some("2026-07-14T12:00:00Z"),
        None,
        None,
    );

    let failing: Arc<dyn CloudObjectStore> = Arc::new(PutFailingCloudStore {
        inner: cloud.clone(),
    });
    let failing_service = service_for_device(&device_a, failing);
    let error = failing_service
        .sync_snapshot_metadata("snap_1")
        .unwrap_err();
    assert_eq!(error.code(), "network_unavailable");
    assert_eq!(
        device_a
            .repo
            .get_cloud_snapshot("account_1", "snap_1")
            .unwrap()
            .unwrap()
            .metadata_sync_status,
        CloudMetadataSyncStatus::Error
    );

    let reopened_repo = Arc::new(SqliteRepo::open(device_a.root.join("savelink.db")).unwrap());
    assert_eq!(
        reopened_repo
            .get_cloud_snapshot("account_1", "snap_1")
            .unwrap()
            .unwrap()
            .metadata_sync_status,
        CloudMetadataSyncStatus::Error,
        "元数据错误状态必须跨重启保留"
    );
    let reopened_service = CloudSyncService::new(
        reopened_repo.clone(),
        Arc::new(FsStore::new(device_a.root.join("repository"))),
        cloud,
        Arc::new(ZipCloudArchiveCodec::new()),
        Arc::new(FixedClock("2026-07-14T20:00:00+08:00")),
        device_a.root.join("cloud-work-reopened"),
        "account_1",
        "device_a",
        "repo_a",
    )
    .unwrap();
    assert_eq!(reopened_service.sync_known_snapshot_metadata().unwrap(), 1);
    let record = reopened_repo
        .get_cloud_snapshot("account_1", "snap_1")
        .unwrap()
        .unwrap();
    assert_eq!(record.note.as_deref(), Some("断网期间修改"));
    assert_eq!(record.metadata_sync_status, CloudMetadataSyncStatus::Synced);
}

#[test]
fn h21_pending_metadata_blocks_cloud_cleanup() {
    let (_tmp, cloud, _codec, device_a, _) = setup();
    seed_device_a(&device_a, &[("save.dat", b"protected")]);
    device_a
        .service
        .upload_snapshot("game_1", "snap_1")
        .unwrap();
    edit_snapshot_metadata(
        &device_a,
        "snap_1",
        None,
        None,
        Some(false),
        Some("2026-07-14T12:00:00Z"),
    );

    let error = device_a
        .service
        .delete_snapshot_everywhere("snap_1")
        .unwrap_err();
    assert_eq!(error.code(), "cloud_state_invalid");
    assert!(device_a.repo.get_snapshot("snap_1").unwrap().is_some());
    assert!(cloud
        .stat_file(&snapshot_ok_path("game_1", "snap_1").unwrap())
        .unwrap()
        .is_some());

    device_a.service.sync_snapshot_metadata("snap_1").unwrap();
    device_a
        .service
        .delete_snapshot_everywhere("snap_1")
        .unwrap();
    assert!(cloud
        .stat_file(&snapshot_metadata_path("game_1", "snap_1").unwrap())
        .unwrap()
        .is_none());
}

fn write_files(root: &Path, files: &[(&str, &[u8])]) {
    fs::create_dir_all(root).unwrap();
    for (relative, bytes) in files {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, bytes).unwrap();
    }
}
