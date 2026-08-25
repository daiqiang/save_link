use savelink_core::model::{CreateOutcome, Game, Reason, SaveFileMapping, SaveSource};
use savelink_core::repo::{Clock, FakeClock, IdGen, Repository, SeqIdGen};
use savelink_core::service::{RestoreService, SnapshotService};
use savelink_core::sqlite_repo::SqliteRepo;
use savelink_core::store::{FsStore, SnapshotStore};
use savelink_core::testkit::TempDir;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct SelectedHarness {
    _temp: TempDir,
    repo: Arc<dyn Repository>,
    store: Arc<dyn SnapshotStore>,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn IdGen>,
}

impl SelectedHarness {
    fn new(root: &Path, local_name: &str) -> Self {
        let temp = TempDir::new();
        let repo: Arc<dyn Repository> = Arc::new(SqliteRepo::open_in_memory().unwrap());
        let store: Arc<dyn SnapshotStore> = Arc::new(FsStore::new(temp.path().join("repo")));
        let clock: Arc<dyn Clock> = Arc::new(FakeClock::new());
        let ids: Arc<dyn IdGen> = Arc::new(SeqIdGen::new());
        repo.insert_game(selected_game(root, local_name, clock.as_ref()))
            .unwrap();
        Self {
            _temp: temp,
            repo,
            store,
            clock,
            ids,
        }
    }

    fn snapshots(&self) -> SnapshotService {
        SnapshotService::new(
            self.repo.clone(),
            self.store.clone(),
            self.clock.clone(),
            self.ids.clone(),
        )
    }

    fn restore(&self) -> RestoreService {
        RestoreService::new(
            self.repo.clone(),
            self.store.clone(),
            self.clock.clone(),
            self.ids.clone(),
        )
    }
}

fn selected_game(root: &Path, local_name: &str, clock: &dyn Clock) -> Game {
    let now = clock.now_stamp();
    Game {
        id: "desmume_game".into(),
        name: "DeSmuME 游戏".into(),
        icon: None,
        repo_path: PathBuf::new(),
        save_paths: vec![root.to_path_buf()],
        save_sources: vec![SaveSource::Files {
            root: root.to_path_buf(),
            files: vec![SaveFileMapping {
                local_relative_path: PathBuf::from(local_name),
                snapshot_relative_path: PathBuf::from("save.dsv"),
            }],
        }],
        emulator_identity: None,
        emulator_binding: None,
        launch_binding: None,
        created_at: now.clone(),
        updated_at: now,
    }
}

#[test]
fn p1_shared_directory_only_tracks_the_selected_game_file() {
    let outer = TempDir::new();
    let battery = outer.child("Battery");
    fs::write(battery.join("game-a.dsv"), b"A-ONE").unwrap();
    fs::write(battery.join("game-b.dsv"), b"B-ONE").unwrap();
    let harness = SelectedHarness::new(&battery, "game-a.dsv");

    let first = match harness
        .snapshots()
        .create_snapshot("desmume_game", None, Reason::Manual)
        .unwrap()
    {
        CreateOutcome::Created(snapshot) => snapshot,
        CreateOutcome::NoChange => panic!("first selected-file snapshot should be created"),
    };
    assert_eq!(first.file_count, 1);
    assert_eq!(first.total_size, 5);

    fs::write(battery.join("game-b.dsv"), b"B-TWO").unwrap();
    assert!(matches!(
        harness
            .snapshots()
            .create_snapshot("desmume_game", None, Reason::Auto)
            .unwrap(),
        CreateOutcome::NoChange
    ));

    fs::write(battery.join("game-a.dsv"), b"A-TWO").unwrap();
    harness
        .snapshots()
        .create_snapshot("desmume_game", None, Reason::Auto)
        .unwrap();
    harness
        .restore()
        .restore_snapshot("desmume_game", &first.id, &|_| {})
        .unwrap();

    assert_eq!(fs::read(battery.join("game-a.dsv")).unwrap(), b"A-ONE");
    assert_eq!(fs::read(battery.join("game-b.dsv")).unwrap(), b"B-TWO");
}

#[test]
fn p2_restore_maps_the_logical_save_to_a_renamed_rom_without_touching_siblings() {
    let outer = TempDir::new();
    let source_battery = outer.child("source-battery");
    fs::write(source_battery.join("zzjb2r ver0.99.dsv"), b"CLOUD-SAVE").unwrap();
    let harness = SelectedHarness::new(&source_battery, "zzjb2r ver0.99.dsv");
    let snapshot = match harness
        .snapshots()
        .create_snapshot("desmume_game", None, Reason::Manual)
        .unwrap()
    {
        CreateOutcome::Created(snapshot) => snapshot,
        CreateOutcome::NoChange => panic!("first selected-file snapshot should be created"),
    };

    let target_battery = outer.child("target-battery");
    fs::write(target_battery.join("another-game.dsv"), b"KEEP-ME").unwrap();
    let mut game = harness.repo.get_game("desmume_game").unwrap().unwrap();
    game.save_paths = vec![target_battery.clone()];
    game.save_sources =
        selected_game(&target_battery, "重装机兵2r.dsv", harness.clock.as_ref()).save_sources;
    harness.repo.update_game(game).unwrap();

    harness
        .restore()
        .restore_snapshot("desmume_game", &snapshot.id, &|_| {})
        .unwrap();

    assert_eq!(
        fs::read(target_battery.join("重装机兵2r.dsv")).unwrap(),
        b"CLOUD-SAVE"
    );
    assert_eq!(
        fs::read(target_battery.join("another-game.dsv")).unwrap(),
        b"KEEP-ME"
    );
}
