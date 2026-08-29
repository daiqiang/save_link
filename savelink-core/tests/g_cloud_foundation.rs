//! G 组：云同步本机状态与 CloudObjectStore 基础设施。

use savelink_core::cloud_model::{
    CloudAccount, CloudGameBinding, CloudMetadataSyncStatus, CloudSnapshotRecord, CloudSyncStatus,
};
use savelink_core::cloud_repo::CloudStateRepository;
use savelink_core::cloud_store::{
    CloudEntryKind, CloudObjectStore, CloudStoreError, FakeCloudObjectStore, PutMode,
};
use savelink_core::model::Reason;
use savelink_core::repo::Repository;
use savelink_core::sqlite_repo::SqliteRepo;
use savelink_core::testkit::TempDir;
use savelink_core::timestamp::normalize_timestamp;
use std::fs;

fn account() -> CloudAccount {
    CloudAccount {
        id: "account_baidu_1".into(),
        provider: "baidu_netdisk".into(),
        account_identity: Some("uk_123".into()),
        display_name: Some("测试账号".into()),
        token_ref: Some("credentials/baidu.json".into()),
        created_at: "2026-07-14T18:00:00+08:00".into(),
        updated_at: "2026-07-14T18:00:00+08:00".into(),
    }
}

fn binding() -> CloudGameBinding {
    CloudGameBinding {
        account_id: "account_baidu_1".into(),
        cloud_game_id: "game_1".into(),
        local_game_id: "game_1".into(),
        remote_revision: 1,
        sync_enabled: true,
        last_scanned_at: Some("2026-07-14T18:10:00+08:00".into()),
    }
}

fn snapshot(id: &str, created_at: &str, status: CloudSyncStatus) -> CloudSnapshotRecord {
    CloudSnapshotRecord {
        account_id: "account_baidu_1".into(),
        cloud_game_id: "game_1".into(),
        snapshot_id: id.into(),
        created_at: created_at.into(),
        reason: Reason::Manual,
        note: Some(format!("备注-{id}")),
        locked: false,
        file_count: 2,
        total_size: 1024,
        source_count: 1,
        content_hash: "0123456789abcdef".into(),
        archive_size: 800,
        archive_sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
        published_at: "2026-07-14T18:20:00+08:00".into(),
        created_by_device_id: "device_1".into(),
        sync_status: status,
        last_synced_at: None,
        last_error_code: None,
        metadata_sync_status: CloudMetadataSyncStatus::Synced,
        metadata_last_synced_at: None,
        metadata_last_error_code: None,
        remote_note_updated_at: created_at.into(),
        remote_locked_updated_at: created_at.into(),
    }
}

#[test]
fn g1_cloud_state_survives_database_reopen() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("savelink.db");
    let mut expected_snapshot = snapshot(
        "snap_1",
        "2026-07-14T18:15:00+08:00",
        CloudSyncStatus::RemoteOnly,
    );

    {
        let repo = SqliteRepo::open(&db_path).unwrap();
        repo.set_setting("device_id", "device_1").unwrap();
        repo.upsert_cloud_account(account()).unwrap();
        repo.upsert_cloud_game_binding(binding()).unwrap();
        repo.upsert_cloud_snapshot(expected_snapshot.clone())
            .unwrap();
    }

    expected_snapshot.created_at =
        normalize_timestamp(&expected_snapshot.created_at).expect("测试时间应可规范化");
    expected_snapshot.remote_note_updated_at =
        normalize_timestamp(&expected_snapshot.remote_note_updated_at)
            .expect("名称更新时间应可规范化");
    expected_snapshot.remote_locked_updated_at =
        normalize_timestamp(&expected_snapshot.remote_locked_updated_at)
            .expect("锁定更新时间应可规范化");

    let repo = SqliteRepo::open(&db_path).unwrap();
    assert_eq!(
        repo.get_setting("device_id").unwrap().as_deref(),
        Some("device_1")
    );
    assert_eq!(
        repo.get_cloud_account("account_baidu_1").unwrap(),
        Some(account())
    );
    assert_eq!(
        repo.get_cloud_game_binding("account_baidu_1", "game_1")
            .unwrap(),
        Some(binding())
    );
    assert_eq!(
        repo.get_cloud_snapshot("account_baidu_1", "snap_1")
            .unwrap(),
        Some(expected_snapshot)
    );
}

#[test]
fn g2_cloud_snapshot_status_transitions_are_persisted() {
    let repo = SqliteRepo::open_in_memory().unwrap();
    repo.upsert_cloud_snapshot(snapshot(
        "snap_1",
        "2026-07-14T18:15:00+08:00",
        CloudSyncStatus::RemoteOnly,
    ))
    .unwrap();

    repo.update_cloud_snapshot_status(
        "account_baidu_1",
        "snap_1",
        CloudSyncStatus::Downloading,
        None,
        None,
    )
    .unwrap();
    let downloading = repo
        .get_cloud_snapshot("account_baidu_1", "snap_1")
        .unwrap()
        .unwrap();
    assert_eq!(downloading.sync_status, CloudSyncStatus::Downloading);

    repo.update_cloud_snapshot_status(
        "account_baidu_1",
        "snap_1",
        CloudSyncStatus::Error,
        None,
        Some("network_unavailable"),
    )
    .unwrap();
    let failed = repo
        .get_cloud_snapshot("account_baidu_1", "snap_1")
        .unwrap()
        .unwrap();
    assert_eq!(failed.sync_status, CloudSyncStatus::Error);
    assert_eq!(
        failed.last_error_code.as_deref(),
        Some("network_unavailable")
    );

    repo.update_cloud_snapshot_status(
        "account_baidu_1",
        "snap_1",
        CloudSyncStatus::Downloaded,
        Some("2026-07-14T18:30:00+08:00"),
        None,
    )
    .unwrap();
    let downloaded = repo
        .get_cloud_snapshot("account_baidu_1", "snap_1")
        .unwrap()
        .unwrap();
    assert_eq!(downloaded.sync_status, CloudSyncStatus::Downloaded);
    assert_eq!(
        downloaded.last_synced_at.as_deref(),
        Some("2026-07-14T18:30:00+08:00")
    );
    assert_eq!(downloaded.last_error_code, None);
    assert!(repo
        .update_cloud_snapshot_status(
            "account_baidu_1",
            "missing",
            CloudSyncStatus::Error,
            None,
            Some("operation_interrupted"),
        )
        .is_err());
}

#[test]
fn g3_cloud_catalog_is_sorted_and_kept_without_local_snapshot_fk() {
    let repo = SqliteRepo::open_in_memory().unwrap();
    repo.upsert_cloud_snapshot(snapshot(
        "snap_old",
        "2026-07-14T18:00:00+08:00",
        CloudSyncStatus::Downloaded,
    ))
    .unwrap();
    repo.upsert_cloud_snapshot(snapshot(
        "snap_new",
        "2026-07-14T19:00:00+08:00",
        CloudSyncStatus::RemoteOnly,
    ))
    .unwrap();

    let listed = repo
        .list_cloud_snapshots("account_baidu_1", "game_1")
        .unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|s| s.snapshot_id.as_str())
            .collect::<Vec<_>>(),
        ["snap_new", "snap_old"]
    );

    repo.update_cloud_snapshot_status(
        "account_baidu_1",
        "snap_old",
        CloudSyncStatus::Ignored,
        Some("2026-07-14T20:00:00+08:00"),
        None,
    )
    .unwrap();
    assert_eq!(
        repo.get_cloud_snapshot("account_baidu_1", "snap_old")
            .unwrap()
            .unwrap()
            .sync_status,
        CloudSyncStatus::Ignored,
        "云端目录记录不能依赖本机 snapshots 外键，否则删除本机副本后会丢失 ignored"
    );
    assert_eq!(
        repo.list_cloud_snapshots_by_status("account_baidu_1", CloudSyncStatus::Ignored)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn g4_cloud_account_and_binding_upsert_do_not_duplicate_rows() {
    let repo = SqliteRepo::open_in_memory().unwrap();
    let mut changed_account = account();
    repo.upsert_cloud_account(changed_account.clone()).unwrap();
    changed_account.display_name = Some("新显示名".into());
    changed_account.updated_at = "2026-07-14T19:00:00+08:00".into();
    repo.upsert_cloud_account(changed_account.clone()).unwrap();
    assert_eq!(repo.list_cloud_accounts().unwrap(), vec![changed_account]);

    let mut changed_binding = binding();
    repo.upsert_cloud_game_binding(changed_binding.clone())
        .unwrap();
    changed_binding.remote_revision = 2;
    changed_binding.sync_enabled = false;
    repo.upsert_cloud_game_binding(changed_binding.clone())
        .unwrap();
    assert_eq!(
        repo.list_cloud_game_bindings("account_baidu_1").unwrap(),
        vec![changed_binding]
    );
}

#[test]
fn g5_fake_cloud_store_supports_create_overwrite_list_download_and_delete() {
    let tmp = TempDir::new();
    let store: Box<dyn CloudObjectStore> =
        Box::new(FakeCloudObjectStore::new(tmp.child("cloud")).unwrap());
    let source = tmp.path().join("source.bin");
    let downloaded = tmp.path().join("downloaded.bin");
    fs::write(&source, b"v1").unwrap();

    let remote = "savelink/v1/games/game_1/snapshots/snap_1.ok";
    let uploaded = store
        .put_file(remote, &source, PutMode::CreateOnly)
        .unwrap();
    assert_eq!(uploaded.size, 2);
    assert!(matches!(
        store.put_file(remote, &source, PutMode::CreateOnly),
        Err(CloudStoreError::AlreadyExists(path)) if path == remote
    ));

    fs::write(&source, b"version-2").unwrap();
    let overwritten = store.put_file(remote, &source, PutMode::Overwrite).unwrap();
    assert_eq!(overwritten.size, 9);
    assert_eq!(store.stat_file(remote).unwrap().unwrap().size, 9);

    let entries = store
        .list_directory("savelink/v1/games/game_1/snapshots")
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "snap_1.ok");
    assert_eq!(entries[0].kind, CloudEntryKind::File);
    let root_entries = store.list_directory("").unwrap();
    assert_eq!(root_entries[0].name, "savelink");
    assert_eq!(root_entries[0].kind, CloudEntryKind::Directory);

    store.get_file(remote, &downloaded).unwrap();
    assert_eq!(fs::read(downloaded).unwrap(), b"version-2");

    store.delete_file(remote).unwrap();
    store.delete_file(remote).unwrap();
    assert_eq!(store.stat_file(remote).unwrap(), None);
}

#[test]
fn g6_fake_cloud_store_rejects_path_traversal() {
    let tmp = TempDir::new();
    let cloud_root = tmp.child("cloud");
    let source = tmp.path().join("source.bin");
    fs::write(&source, b"secret").unwrap();
    let store = FakeCloudObjectStore::new(&cloud_root).unwrap();

    for invalid in [
        "../escape.bin",
        "/absolute.bin",
        "a\\b.bin",
        "a//b.bin",
        "C:/x.bin",
    ] {
        assert!(matches!(
            store.put_file(invalid, &source, PutMode::Overwrite),
            Err(CloudStoreError::InvalidPath(_))
        ));
    }
    assert!(!tmp.path().join("escape.bin").exists());
}

#[test]
fn g7_existing_local_database_gets_cloud_tables_on_open() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("old-savelink.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE games (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                icon TEXT,
                repo_path TEXT NOT NULL,
                save_paths TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
             );
             CREATE TABLE snapshots (
                id TEXT PRIMARY KEY,
                game_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                note TEXT,
                reason TEXT NOT NULL,
                locked INTEGER NOT NULL DEFAULT 0,
                file_count INTEGER NOT NULL,
                total_size INTEGER NOT NULL,
                content_hash TEXT NOT NULL,
                storage_key TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'complete'
             );",
        )
        .unwrap();
    }

    let repo = SqliteRepo::open(&db_path).unwrap();
    repo.set_setting("device_id", "device_after_upgrade")
        .unwrap();
    assert_eq!(
        repo.get_setting("device_id").unwrap().as_deref(),
        Some("device_after_upgrade")
    );
}

#[test]
fn g8_v010_cloud_status_table_is_migrated_without_losing_records() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("v010-savelink.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE snapshots (
                id TEXT PRIMARY KEY,
                game_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                note TEXT,
                reason TEXT NOT NULL,
                locked INTEGER NOT NULL DEFAULT 0,
                file_count INTEGER NOT NULL,
                total_size INTEGER NOT NULL,
                content_hash TEXT NOT NULL,
                storage_key TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'complete'
             );
             INSERT INTO snapshots VALUES (
                'snap_1', 'game_1', '2026-07-14T18:10:00+08:00',
                '本机后来修改的名称', 'auto', 1, 1, 7, 'content-hash', 'snap_1', 'complete'
             );
             CREATE TABLE cloud_snapshot_sync (
                account_id TEXT NOT NULL,
                cloud_game_id TEXT NOT NULL,
                snapshot_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                reason TEXT NOT NULL,
                note TEXT,
                locked INTEGER NOT NULL DEFAULT 0,
                file_count INTEGER NOT NULL,
                total_size INTEGER NOT NULL,
                content_hash TEXT NOT NULL,
                archive_size INTEGER NOT NULL,
                archive_sha256 TEXT NOT NULL,
                published_at TEXT NOT NULL,
                created_by_device_id TEXT NOT NULL,
                sync_status TEXT NOT NULL CHECK (
                    sync_status IN (
                        'uploading', 'uploaded', 'remote_only', 'downloading',
                        'downloaded', 'ignored', 'error'
                    )
                ),
                last_synced_at TEXT,
                last_error_code TEXT,
                PRIMARY KEY (account_id, snapshot_id)
             );
             CREATE INDEX idx_cloud_snapshot_game
                ON cloud_snapshot_sync(account_id, cloud_game_id, created_at DESC);
             INSERT INTO cloud_snapshot_sync VALUES (
                'account_1', 'game_1', 'snap_1', '2026-07-14T18:10:00+08:00',
                'auto', NULL, 0, 1, 7, 'content-hash', 99, 'archive-hash',
                '2026-07-14T18:11:00+08:00', 'device_a', 'uploaded',
                '2026-07-14T18:12:00+08:00', NULL
             );",
        )
        .unwrap();
    }

    let repo = SqliteRepo::open(&db_path).unwrap();
    let migrated = repo
        .get_cloud_snapshot("account_1", "snap_1")
        .unwrap()
        .expect("旧云同步记录应保留");
    assert_eq!(migrated.sync_status, CloudSyncStatus::Uploaded);
    assert_eq!(
        migrated.metadata_sync_status,
        CloudMetadataSyncStatus::Pending,
        "旧本机元数据与云缓存不同时必须安排重试"
    );
    let local = repo.get_snapshot("snap_1").unwrap().unwrap();
    assert_eq!(local.note.as_deref(), Some("本机后来修改的名称"));
    assert!(local.locked);
    assert_ne!(local.note_updated_at, local.created_at);
    assert_ne!(local.locked_updated_at, local.created_at);

    repo.update_cloud_snapshot_status(
        "account_1",
        "snap_1",
        CloudSyncStatus::DeletePending,
        None,
        None,
    )
    .expect("迁移后应允许写入新删除状态");
    assert_eq!(
        repo.get_cloud_snapshot("account_1", "snap_1")
            .unwrap()
            .unwrap()
            .sync_status,
        CloudSyncStatus::DeletePending
    );
}
