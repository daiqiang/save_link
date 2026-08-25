//! Q 组：尚未确定存档目录的程序游戏。

use rusqlite::Connection;
use savelink_core::error::SaveLinkError;
use savelink_core::model::{
    Game, GameConfigurationState, GameLaunchBinding, MissingDirChoice, Reason,
};
use savelink_core::repo::{FakeClock, Repository, SeqIdGen};
use savelink_core::service::{AutoBackupService, RestoreService, SnapshotService};
use savelink_core::sqlite_repo::SqliteRepo;
use savelink_core::store::{FsStore, SnapshotStore};
use savelink_core::testkit::TempDir;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn game(id: &str, save_paths: Vec<PathBuf>, launch_binding: Option<GameLaunchBinding>) -> Game {
    Game {
        id: id.into(),
        name: "Pending Game".into(),
        icon: None,
        repo_path: PathBuf::new(),
        save_paths,
        save_sources: Vec::new(),
        emulator_identity: None,
        emulator_binding: None,
        launch_binding,
        created_at: "2026-08-24T10:00:00Z".into(),
        updated_at: "2026-08-24T10:00:00Z".into(),
    }
}

fn launch_binding(install_dir: &Path) -> GameLaunchBinding {
    GameLaunchBinding::executable(install_dir.join("game.exe"), install_dir.to_path_buf())
}

#[test]
fn q1_configuration_state_is_derived_from_sources_and_local_binding() {
    let configured = game("configured", vec![PathBuf::from("C:/save")], None);
    let pending_discovery = game(
        "pending_discovery",
        Vec::new(),
        Some(launch_binding(Path::new("C:/game"))),
    );
    let pending_binding = game("pending_binding", Vec::new(), None);

    assert_eq!(
        configured.configuration_state(),
        GameConfigurationState::Configured
    );
    assert!(configured.is_configured());
    assert_eq!(
        pending_discovery.configuration_state(),
        GameConfigurationState::PendingDiscovery
    );
    assert!(!pending_discovery.is_configured());
    assert_eq!(
        pending_binding.configuration_state(),
        GameConfigurationState::PendingBinding
    );
}

#[test]
fn q2_current_schema_migrates_launch_binding_without_changing_existing_game() {
    let temp = TempDir::new();
    let db_path = temp.path().join("current-schema.db");
    {
        let connection = Connection::open(&db_path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE games (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    icon TEXT,
                    repo_path TEXT NOT NULL,
                    save_paths TEXT NOT NULL,
                    save_sources TEXT NOT NULL DEFAULT '[]',
                    emulator_identity TEXT,
                    emulator_binding TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                 );
                 INSERT INTO games (
                    id, name, icon, repo_path, save_paths, save_sources,
                    emulator_identity, emulator_binding, created_at, updated_at
                 ) VALUES (
                    'existing', 'Existing Game', NULL, '', 'C:\\existing-save', '[]',
                    NULL, NULL, '2026-08-24T10:00:00Z', '2026-08-24T10:00:00Z'
                 );",
            )
            .unwrap();
    }

    let repo = SqliteRepo::open(&db_path).expect("当前 schema 应自动增加 launch_binding");
    let existing = repo.get_game("existing").unwrap().unwrap();
    assert_eq!(existing.name, "Existing Game");
    assert_eq!(
        existing.save_paths,
        vec![PathBuf::from("C:\\existing-save")]
    );
    assert!(existing.launch_binding.is_none());
    assert_eq!(
        existing.configuration_state(),
        GameConfigurationState::Configured
    );
}

#[test]
fn q3_empty_paths_and_launch_binding_survive_reopen() {
    let temp = TempDir::new();
    let db_path = temp.path().join("pending.db");
    let install_dir = temp.child("portable-game");
    fs::create_dir_all(&install_dir).unwrap();
    fs::write(install_dir.join("game.exe"), b"fake executable").unwrap();
    let expected_binding = launch_binding(&install_dir);

    {
        let repo = SqliteRepo::open(&db_path).unwrap();
        repo.insert_game(game("pending", Vec::new(), Some(expected_binding.clone())))
            .unwrap();
    }

    let repo = SqliteRepo::open(&db_path).unwrap();
    let reopened = repo.get_game("pending").unwrap().unwrap();
    assert!(reopened.save_paths.is_empty());
    assert_eq!(reopened.launch_binding, Some(expected_binding));
    assert_eq!(
        reopened.configuration_state(),
        GameConfigurationState::PendingDiscovery
    );
}

#[test]
fn q4_pending_game_is_skipped_and_manual_data_operations_are_rejected() {
    let temp = TempDir::new();
    let install_dir = temp.child("portable-game");
    fs::create_dir_all(&install_dir).unwrap();
    let executable = install_dir.join("game.exe");
    fs::write(&executable, b"fake executable").unwrap();

    let repo: Arc<dyn Repository> = Arc::new(SqliteRepo::open_in_memory().unwrap());
    repo.insert_game(game(
        "pending",
        Vec::new(),
        Some(launch_binding(&install_dir)),
    ))
    .unwrap();
    let store: Arc<dyn SnapshotStore> = Arc::new(FsStore::new(temp.child("repository")));
    let clock = Arc::new(FakeClock::new());
    let ids = Arc::new(SeqIdGen::new());
    let snapshots = SnapshotService::new(repo.clone(), store.clone(), clock.clone(), ids.clone());
    let restorer = RestoreService::new(repo.clone(), store, clock, ids);

    let report = AutoBackupService::new(SnapshotService::new(
        snapshots.repo.clone(),
        snapshots.store.clone(),
        snapshots.clock.clone(),
        snapshots.ids.clone(),
    ))
    .run_once()
    .unwrap();
    assert_eq!(report.skipped_game_ids, vec!["pending"]);
    assert!(report.failures.is_empty());
    assert_eq!(
        snapshots.create_snapshot("pending", None, Reason::Manual),
        Err(SaveLinkError::SaveSourcesNotConfigured)
    );
    assert_eq!(
        restorer.restore_snapshot("pending", "missing", &|_| {}),
        Err(SaveLinkError::SaveSourcesNotConfigured)
    );
    assert_eq!(
        restorer.restore_with_choice(
            "pending",
            "missing",
            MissingDirChoice::CreateAndRestore,
            &|_| {},
        ),
        Err(SaveLinkError::SaveSourcesNotConfigured)
    );

    snapshots.delete_game("pending").unwrap();
    assert!(repo.get_game("pending").unwrap().is_none());
    assert!(install_dir.is_dir(), "移除待配置游戏不得删除安装目录");
    assert!(executable.is_file(), "移除待配置游戏不得删除 EXE");
}
