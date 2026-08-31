//! O 组：一个游戏包含多个存档根目录时的本地快照与兼容性验证。

mod common;

use common::*;
use rusqlite::Connection;
use savelink_core::cloud_repo::CloudStateRepository;
use savelink_core::error::SaveLinkError;
use savelink_core::model::{CreateOutcome, Reason};
use savelink_core::repo::Repository;
use savelink_core::scan;
use savelink_core::sqlite_repo::SqliteRepo;
use savelink_core::store::{FsStore, SnapshotStore};
use savelink_core::testkit::{write_files, TempDir};
use std::fs;
use std::path::PathBuf;

#[test]
fn o1_multi_source_fingerprint_changes_when_either_root_changes() {
    let temp = TempDir::new();
    let first = temp.child("first");
    let second = temp.child("second");
    write_files(&first, &[("save.dat", b"FIRST-V1")]);
    write_files(&second, &[("profile/config.dat", b"SECOND-V1")]);

    let original = scan::scan(&[first.clone(), second.clone()]).unwrap();
    fs::write(first.join("save.dat"), b"FIRST-V2").unwrap();
    let first_changed = scan::scan(&[first.clone(), second.clone()]).unwrap();
    assert_ne!(original.content_hash, first_changed.content_hash);

    fs::write(first.join("save.dat"), b"FIRST-V1").unwrap();
    fs::write(second.join("profile/config.dat"), b"SECOND-V2").unwrap();
    let second_changed = scan::scan(&[first, second]).unwrap();
    assert_ne!(original.content_hash, second_changed.content_hash);
}

#[test]
fn o2_fs_store_multi_source_round_trip_preserves_root_boundaries() {
    let temp = TempDir::new();
    let first = temp.child("source-0");
    let second = temp.child("source-1");
    write_files(
        &first,
        &[("slot/save.dat", b"SAVE"), ("shared.dat", b"FIRST")],
    );
    write_files(
        &second,
        &[("prefs/settings.json", b"{}"), ("shared.dat", b"SECOND")],
    );
    let sources = vec![first, second];
    let expected = scan::scan(&sources).unwrap();
    let store = FsStore::new(temp.path().join("repository"));

    store.create("multi", &sources, &expected).unwrap();
    assert!(store.verify("multi").unwrap());

    let targets = vec![
        temp.path().join("restored-0"),
        temp.path().join("restored-1"),
    ];
    store.restore_sources("multi", &targets).unwrap();
    assert_eq!(scan::scan(&targets).unwrap(), expected);
    assert_eq!(fs::read(targets[0].join("shared.dat")).unwrap(), b"FIRST");
    assert_eq!(fs::read(targets[1].join("shared.dat")).unwrap(), b"SECOND");
    assert!(matches!(
        store.restore_sources("multi", &targets[..1]),
        Err(SaveLinkError::SnapshotCorrupt)
    ));
}

#[test]
fn o3_restore_replaces_every_source_without_cross_contamination() {
    let h = Harness::new(&[]);
    let second = h.tmp.child("second-save");
    let mut game = h.repo.get_game(&h.game_id).unwrap().unwrap();
    game.save_paths = vec![h.save_dir.clone(), second.clone()];
    h.repo.update_game(game).unwrap();

    write_files(
        &h.save_dir,
        &[("slot/save.dat", b"TARGET-0"), ("same.dat", b"ROOT-0")],
    );
    write_files(
        &second,
        &[("profile.dat", b"TARGET-1"), ("same.dat", b"ROOT-1")],
    );
    let target = match h
        .snapshots()
        .create_snapshot(&h.game_id, Some("双目录目标".into()), Reason::Manual)
        .unwrap()
    {
        CreateOutcome::Created(snapshot) => snapshot,
        other => panic!("expected created snapshot, got {other:?}"),
    };
    assert_eq!(target.source_count, 2);

    fs::remove_dir_all(&h.save_dir).unwrap();
    fs::remove_dir_all(&second).unwrap();
    write_files(
        &h.save_dir,
        &[("slot/save.dat", b"CURRENT-0"), ("obsolete.bin", b"REMOVE")],
    );
    write_files(
        &second,
        &[("profile.dat", b"CURRENT-1"), ("obsolete.bin", b"REMOVE")],
    );

    let outcome = h
        .restore()
        .restore_snapshot(&h.game_id, &target.id, &no_progress())
        .unwrap();
    assert!(outcome.restored);
    assert_eq!(
        scan::scan(&[h.save_dir.clone(), second.clone()])
            .unwrap()
            .content_hash,
        target.content_hash
    );
    assert_eq!(fs::read(h.save_dir.join("same.dat")).unwrap(), b"ROOT-0");
    assert_eq!(fs::read(second.join("same.dat")).unwrap(), b"ROOT-1");
    assert!(!h.save_dir.join("obsolete.bin").exists());
    assert!(!second.join("obsolete.bin").exists());
}

#[test]
fn o4_old_sqlite_rows_migrate_to_one_source() {
    let temp = TempDir::new();
    let database = temp.path().join("legacy.db");
    {
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
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
                    'legacy_snapshot', 'legacy_game', '2026-08-01 10:00', NULL,
                    'manual', 0, 1, 4, '0123456789abcdef', 'legacy_snapshot', 'complete'
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
                    sync_status TEXT NOT NULL CHECK (sync_status IN ('uploaded', 'delete_pending')),
                    last_synced_at TEXT,
                    last_error_code TEXT,
                    PRIMARY KEY (account_id, snapshot_id)
                );
                INSERT INTO cloud_snapshot_sync VALUES (
                    'account_1', 'legacy_game', 'legacy_snapshot',
                    '2026-08-01T02:00:00Z', 'manual', NULL, 0, 1, 4,
                    '0123456789abcdef', 100, 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                    '2026-08-01T02:01:00Z', 'device_a', 'uploaded',
                    '2026-08-01T02:01:00Z', NULL
                );",
            )
            .unwrap();
    }

    let repo = SqliteRepo::open(&database).unwrap();
    assert_eq!(
        repo.get_snapshot("legacy_snapshot")
            .unwrap()
            .unwrap()
            .source_count,
        1
    );
    assert_eq!(
        repo.get_cloud_snapshot("account_1", "legacy_snapshot")
            .unwrap()
            .unwrap()
            .source_count,
        1
    );
}

#[test]
fn o5_overlapping_sources_are_rejected_before_scanning() {
    let temp = TempDir::new();
    let outer = temp.child("outer");
    let inner = outer.join("inner");
    write_files(&inner, &[("save.dat", b"SAVE")]);

    let error = scan::validate_save_paths(&[outer.clone(), inner.clone()]).unwrap_err();
    assert!(matches!(error, SaveLinkError::OverlappingSavePaths { .. }));

    let reverse = scan::validate_save_paths(&[inner, outer]).unwrap_err();
    assert!(matches!(
        reverse,
        SaveLinkError::OverlappingSavePaths { .. }
    ));
}

#[test]
fn o6_path_overlap_treats_windows_verbatim_prefix_as_the_same_path() {
    let regular = PathBuf::from(r"C:\Games\Arcane Trigger\Processes");
    let verbatim = PathBuf::from(r"\\?\C:\Games\Arcane Trigger\Processes");
    let child = PathBuf::from(r"\\?\C:\Games\Arcane Trigger\Processes\Save");

    assert!(scan::save_paths_overlap(&regular, &verbatim));
    assert!(scan::path_is_same_or_descendant(&regular, &child));
}

#[test]
fn o7_current_save_match_checks_every_configured_source() {
    let h = Harness::new(&[("save.dat", b"FIRST")]);
    let second = h.tmp.child("second-save");
    write_files(&second, &[("profile.dat", b"SECOND")]);

    let mut game = h.repo.get_game(&h.game_id).unwrap().unwrap();
    game.save_paths = vec![h.save_dir.clone(), second.clone()];
    h.repo.update_game(game).unwrap();

    let snapshot = match h
        .snapshots()
        .create_snapshot(&h.game_id, None, Reason::Manual)
        .unwrap()
    {
        CreateOutcome::Created(snapshot) => snapshot,
        CreateOutcome::NoChange => panic!("first multi-source snapshot should be created"),
    };

    assert!(h
        .snapshots()
        .current_save_matches(&snapshot.id)
        .expect("unchanged multi-source save should match"));

    fs::write(second.join("profile.dat"), b"CHANGED").unwrap();
    assert!(!h
        .snapshots()
        .current_save_matches(&snapshot.id)
        .expect("a change in any source should be detected"));
}
