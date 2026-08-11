use rusqlite::{params, Connection};
use savelink_core::steam_discovery::{SteamDiscoveryError, SteamDiscoveryService};
use savelink_core::testkit::TempDir;
use std::fs;
use std::path::Path;

#[test]
fn n1_scans_multiple_libraries_and_groups_save_directories() {
    let temp = TempDir::new();
    let steam_root = temp.child("Steam");
    let second_library = temp.child("SteamLibrary");
    prepare_library(&steam_root);
    prepare_library(&second_library);
    write_library_folders(&steam_root, &[&steam_root, &second_library]);
    write_app_manifest(&steam_root, 646570, "Slay the Spire", "SlayTheSpire");
    write_app_manifest(&second_library, 900001, "Extra ID Game", "ExtraGame");

    let slay_base = steam_root.join("steamapps/common/SlayTheSpire");
    for directory in ["preferences", "runs", "saves", "config"] {
        fs::create_dir_all(slay_base.join(directory)).unwrap();
    }
    fs::write(slay_base.join("saves/slot.dat"), b"save").unwrap();
    let extra_base = second_library.join("steamapps/common/ExtraGame");
    fs::create_dir_all(extra_base.join("userdata")).unwrap();

    let database = temp.path().join("manifest.db");
    build_manifest_database(&database);
    let report = SteamDiscoveryService::new(&database)
        .scan(Some(&steam_root))
        .unwrap();

    assert_eq!(report.library_count, 2);
    assert_eq!(report.registered_app_count, 2);
    assert_eq!(report.manifest_match_count, 2);
    assert_eq!(report.games.len(), 2);

    let slay = report
        .games
        .iter()
        .find(|game| game.app_id == 646570)
        .unwrap();
    assert_eq!(slay.name, "Slay the Spire");
    assert_eq!(
        slay.save_paths,
        vec![
            slay_base.join("preferences"),
            slay_base.join("runs"),
            slay_base.join("saves"),
        ]
    );
    assert_eq!(slay.config_paths, vec![slay_base.join("config")]);
    assert_eq!(slay.current_system_unresolved_rules, 0);
    assert_eq!(slay.other_environment_rules, 1);

    let extra = report
        .games
        .iter()
        .find(|game| game.app_id == 900001)
        .unwrap();
    assert_eq!(extra.name, "Extra ID Game");
    assert_eq!(extra.save_paths, vec![extra_base.join("userdata")]);
}

#[test]
fn n2_exact_file_rule_is_normalized_to_its_parent_directory() {
    let temp = TempDir::new();
    let steam_root = temp.child("Steam");
    prepare_library(&steam_root);
    write_library_folders(&steam_root, &[&steam_root]);
    write_app_manifest(&steam_root, 646570, "Slay the Spire", "SlayTheSpire");
    let base = steam_root.join("steamapps/common/SlayTheSpire");
    fs::create_dir_all(base.join("saves")).unwrap();
    fs::write(base.join("saves/slot.dat"), b"save").unwrap();

    let database = temp.path().join("manifest.db");
    build_manifest_database(&database);
    let report = SteamDiscoveryService::new(&database)
        .scan(Some(&steam_root))
        .unwrap();
    let game = report
        .games
        .iter()
        .find(|game| game.app_id == 646570)
        .unwrap();

    assert!(game.save_paths.contains(&base.join("saves")));
    assert!(!game.save_paths.iter().any(|path| path.is_file()));
}

#[test]
fn n3_missing_manifest_database_has_a_clear_error() {
    let temp = TempDir::new();
    let missing = temp.path().join("missing.db");
    let error = SteamDiscoveryService::new(&missing).scan(None).unwrap_err();
    assert_eq!(error, SteamDiscoveryError::ManifestDatabaseMissing(missing));
}

#[test]
fn n4_store_user_id_rule_does_not_capture_sibling_config_file() {
    let temp = TempDir::new();
    let steam_root = temp.child("Steam");
    prepare_library(&steam_root);
    write_library_folders(&steam_root, &[&steam_root]);
    write_app_manifest(&steam_root, 2778580, "Elden Ring", "EldenRing");

    let base = steam_root.join("steamapps/common/EldenRing");
    let user_dir = base.join("saves/76561198820991451");
    fs::create_dir_all(&user_dir).unwrap();
    fs::write(user_dir.join("ER0000.sl2"), b"save").unwrap();
    fs::write(base.join("saves/GraphicsConfig.xml"), b"config").unwrap();

    let database = temp.path().join("manifest.db");
    build_manifest_database(&database);
    let report = SteamDiscoveryService::new(&database)
        .scan(Some(&steam_root))
        .unwrap();
    let game = report
        .games
        .iter()
        .find(|game| game.app_id == 2778580)
        .unwrap();

    assert_eq!(game.save_paths, vec![user_dir]);
    assert_eq!(game.config_paths, vec![base.join("saves")]);
}

#[test]
fn n5_discovery_collapses_nested_save_rule_matches() {
    let temp = TempDir::new();
    let steam_root = temp.child("Steam");
    prepare_library(&steam_root);
    write_library_folders(&steam_root, &[&steam_root]);
    write_app_manifest(&steam_root, 999999, "Nested Save Game", "NestedSaveGame");

    let base = steam_root.join("steamapps/common/NestedSaveGame");
    fs::create_dir_all(base.join("all/nested")).unwrap();
    fs::write(base.join("all/root.dat"), b"root").unwrap();
    fs::write(base.join("all/nested/child.dat"), b"child").unwrap();

    let database = temp.path().join("manifest.db");
    build_manifest_database(&database);
    let report = SteamDiscoveryService::new(&database)
        .scan(Some(&steam_root))
        .unwrap();
    let game = report
        .games
        .iter()
        .find(|game| game.app_id == 999999)
        .unwrap();

    assert_eq!(game.save_paths, vec![base.join("all")]);
}

fn prepare_library(library: &Path) {
    fs::create_dir_all(library.join("steamapps/common")).unwrap();
}

fn write_library_folders(steam_root: &Path, libraries: &[&Path]) {
    let entries = libraries
        .iter()
        .enumerate()
        .map(|(index, path)| format!("\"{index}\" {{ \"path\" \"{}\" }}", path_text(path)))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        steam_root.join("steamapps/libraryfolders.vdf"),
        format!("\"libraryfolders\"\n{{\n{entries}\n}}"),
    )
    .unwrap();
}

fn write_app_manifest(library: &Path, app_id: u32, name: &str, install_dir: &str) {
    fs::write(
        library
            .join("steamapps")
            .join(format!("appmanifest_{app_id}.acf")),
        format!(
            "\"AppState\"\n{{\n\"appid\" \"{app_id}\"\n\"name\" \"{name}\"\n\"installdir\" \"{install_dir}\"\n}}"
        ),
    )
    .unwrap();
}

fn build_manifest_database(path: &Path) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE manifest_games (
                 id INTEGER PRIMARY KEY,
                 name TEXT NOT NULL UNIQUE,
                 alias TEXT
             );
             CREATE TABLE manifest_store_ids (
                 game_id INTEGER NOT NULL,
                 store TEXT NOT NULL,
                 store_game_id TEXT NOT NULL,
                 is_primary INTEGER NOT NULL DEFAULT 0,
                 PRIMARY KEY (game_id, store, store_game_id)
             ) WITHOUT ROWID;
             CREATE TABLE manifest_file_rules (
                 id INTEGER PRIMARY KEY,
                 game_id INTEGER NOT NULL,
                 path_template TEXT NOT NULL,
                 tags TEXT NOT NULL
             );
             CREATE TABLE manifest_file_constraints (
                 file_rule_id INTEGER NOT NULL,
                 ordinal INTEGER NOT NULL,
                 os TEXT,
                 store TEXT,
                 PRIMARY KEY (file_rule_id, ordinal)
             ) WITHOUT ROWID;
             PRAGMA user_version = 1;",
        )
        .unwrap();

    connection
        .execute(
            "INSERT INTO manifest_games(id, name, alias) VALUES (1, 'Slay the Spire', NULL)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO manifest_games(id, name, alias) VALUES (2, 'Extra ID Game', NULL)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO manifest_games(id, name, alias) VALUES (3, 'Elden Ring', NULL)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO manifest_games(id, name, alias) VALUES (4, 'Nested Save Game', NULL)",
            [],
        )
        .unwrap();
    for (game_id, store_id, primary) in [
        (1, "646570", 1),
        (2, "123456", 1),
        (2, "900001", 0),
        (3, "2778580", 1),
        (4, "999999", 1),
    ] {
        connection
            .execute(
                "INSERT INTO manifest_store_ids(game_id, store, store_game_id, is_primary)
                 VALUES (?1, 'steam', ?2, ?3)",
                params![game_id, store_id, primary],
            )
            .unwrap();
    }

    let rules = [
        (1, 1, "<base>/preferences", "config,save"),
        (2, 1, "<base>/runs", "save"),
        (3, 1, "<base>/saves/slot.dat", "save"),
        (4, 1, "<base>/config", "config"),
        (5, 1, "<xdgData>/SlayTheSpire", "save"),
        (6, 2, "<base>/userdata", "save"),
        (7, 3, "<base>/saves/<storeUserId>", "save"),
        (8, 3, "<base>/saves/GraphicsConfig.xml", "config"),
        (9, 4, "<base>/all", "save"),
        (10, 4, "<base>/all/nested", "save"),
    ];
    for (id, game_id, template, tags) in rules {
        connection
            .execute(
                "INSERT INTO manifest_file_rules(id, game_id, path_template, tags)
                 VALUES (?1, ?2, ?3, ?4)",
                params![id, game_id, template, tags],
            )
            .unwrap();
    }
    for rule_id in [1, 2, 3, 4, 6, 7, 8, 9, 10] {
        connection
            .execute(
                "INSERT INTO manifest_file_constraints(file_rule_id, ordinal, os, store)
                 VALUES (?1, 0, 'windows', 'steam')",
                params![rule_id],
            )
            .unwrap();
    }
    connection
        .execute(
            "INSERT INTO manifest_file_constraints(file_rule_id, ordinal, os, store)
             VALUES (5, 0, 'linux', 'steam')",
            [],
        )
        .unwrap();
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
