use savelink_core::desmume_discovery::{inspect_rom, DesmumeDiscoveryService};
use savelink_core::testkit::TempDir;
use std::fs;
use std::path::Path;

#[test]
fn q1_stale_ini_path_requires_an_explicit_rom_directory() {
    let temp = TempDir::new();
    let emulator = prepare_emulator(temp.path());
    let stale = temp.path().join("missing-roms");
    fs::write(
        emulator.join("desmume.ini"),
        format!("[PathSettings]\nRoms={}\n", stale.display()),
    )
    .unwrap();

    let report = DesmumeDiscoveryService::scan(&emulator, None).unwrap();
    assert_eq!(report.configured_rom_root, Some(stale));
    assert!(report.configured_rom_root_missing);
    assert!(report.rom_root.is_none());
    assert!(report.games.is_empty());
}

#[test]
fn q2_discovers_exact_dsv_and_ignores_similarly_named_history_files() {
    let temp = TempDir::new();
    let emulator = prepare_emulator(temp.path());
    let roms = temp.child("roms");
    write_nds(&roms.join("zzjb2r ver0.99.nds"), b"METALMAX2R", b"TMXJ");
    write_nds(&roms.join("other.nds"), b"OTHER GAME", b"ABCE");
    let battery = emulator.join("Battery");
    fs::create_dir_all(&battery).unwrap();
    fs::write(battery.join("zzjb2r ver0.99.dsv"), b"current").unwrap();
    fs::write(battery.join("other.dsv-01"), b"history-only").unwrap();

    let report = DesmumeDiscoveryService::scan(&emulator, Some(&roms)).unwrap();
    assert_eq!(report.games.len(), 2);
    let metal = report
        .games
        .iter()
        .find(|game| game.name == "zzjb2r ver0.99")
        .unwrap();
    assert!(metal.has_save);
    assert_eq!(metal.identity.rom.header_title, "METALMAX2R");
    assert_eq!(metal.identity.rom.game_code, "TMXJ");
    assert_eq!(metal.identity.rom.sha256.len(), 64);
    assert!(metal
        .identity
        .rom
        .sha256
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));

    let other = report
        .games
        .iter()
        .find(|game| game.name == "other")
        .unwrap();
    assert!(!other.has_save, ".dsv-01 不能冒充当前 .dsv 存档");
}

#[test]
fn q3_renaming_the_same_rom_keeps_its_strong_identity() {
    let temp = TempDir::new();
    let emulator = prepare_emulator(temp.path());
    let first = temp.path().join("zzjb2r ver0.99.nds");
    let renamed = temp.path().join("重装机兵2r.nds");
    write_nds(&first, b"METALMAX2R", b"TMXJ");
    fs::copy(&first, &renamed).unwrap();

    let (first_identity, _) = inspect_rom(&emulator, &first).unwrap();
    let (renamed_identity, _) = inspect_rom(&emulator, &renamed).unwrap();
    assert_eq!(first_identity.rom.sha256, renamed_identity.rom.sha256);
    assert_eq!(first_identity.rom.game_code, renamed_identity.rom.game_code);
    assert_ne!(first_identity.rom.file_name, renamed_identity.rom.file_name);
}

#[cfg(windows)]
#[test]
fn q4_directory_link_loop_does_not_overflow_the_scan_stack() {
    let temp = TempDir::new();
    let emulator = prepare_emulator(temp.path());
    let roms = temp.child("roms");
    write_nds(&roms.join("game.nds"), b"TEST GAME", b"ABCD");

    // Creating a directory link requires Developer Mode or elevated test privileges on
    // some Windows machines. The scan behavior is still covered wherever the link can be made.
    let loop_dir = roms.join("loop");
    if std::os::windows::fs::symlink_dir(&roms, &loop_dir).is_err() {
        return;
    }

    let report = DesmumeDiscoveryService::scan(&emulator, Some(&roms)).unwrap();
    assert_eq!(report.games.len(), 1);
    assert_eq!(report.games[0].name, "game");
}

#[test]
#[ignore = "requires SAVELINK_DESMUME_ROOT and SAVELINK_DESMUME_ROM_ROOT"]
fn q5_live_desmume_copy_discovers_a_real_save() {
    let emulator = std::env::var_os("SAVELINK_DESMUME_ROOT")
        .map(std::path::PathBuf::from)
        .expect("SAVELINK_DESMUME_ROOT is required");
    let roms = std::env::var_os("SAVELINK_DESMUME_ROM_ROOT")
        .map(std::path::PathBuf::from)
        .expect("SAVELINK_DESMUME_ROM_ROOT is required");
    let report = DesmumeDiscoveryService::scan(&emulator, Some(&roms)).unwrap();
    let game = report
        .games
        .iter()
        .find(|game| game.identity.rom.game_code == "TMXJ")
        .expect("real Metal Max 2 Reloaded ROM should be discovered");
    assert!(game.has_save);
    assert_eq!(game.identity.rom.header_title, "METALMAX2R");
    assert_eq!(
        game.save_path.extension().and_then(|value| value.to_str()),
        Some("dsv")
    );
}

#[test]
fn q6_rom_hashing_does_not_require_a_one_megabyte_worker_stack() {
    let temp = TempDir::new();
    let emulator = prepare_emulator(temp.path());
    let rom = temp.path().join("small-stack.nds");
    write_nds(&rom, b"SMALL STACK", b"STKE");

    let identity = std::thread::Builder::new()
        .name("small-rom-scan-worker".into())
        .stack_size(512 * 1024)
        .spawn(move || inspect_rom(&emulator, &rom).unwrap().0)
        .unwrap()
        .join()
        .unwrap();

    assert_eq!(identity.rom.header_title, "SMALL STACK");
    assert_eq!(identity.rom.game_code, "STKE");
    assert_eq!(identity.rom.sha256.len(), 64);
}

fn prepare_emulator(parent: &Path) -> std::path::PathBuf {
    let emulator = parent.join("desmume-0.9.13-win64");
    fs::create_dir_all(&emulator).unwrap();
    fs::write(emulator.join("DeSmuME_0.9.13_x64.exe"), b"test executable").unwrap();
    emulator
}

fn write_nds(path: &Path, title: &[u8], game_code: &[u8]) {
    let mut bytes = vec![0u8; 4096];
    bytes[..title.len().min(12)].copy_from_slice(&title[..title.len().min(12)]);
    bytes[12..16].copy_from_slice(game_code);
    bytes[16..18].copy_from_slice(b"QC");
    fs::write(path, bytes).unwrap();
}
