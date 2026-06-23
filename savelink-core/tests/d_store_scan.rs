//! D 组：存储层与扫描。对应 spec D1–D5。
//!
//! 注意：D3（content_hash 稳定性）测的是**已实现**的 scan 模块，应直接为绿——
//! 它是裁判工具的自检，确保 oracle 本身可靠。其余 D 项依赖 FsStore（todo，红）。

mod common;

use savelink_core::model::ScanResult;
use savelink_core::scan;
use savelink_core::store::{FsStore, SnapshotStore};
use savelink_core::testkit::{corrupt_dir, dir_fingerprint, write_files, TempDir};
use std::path::PathBuf;

fn scan_ctx(dir: &std::path::Path) -> ScanResult {
    scan::fingerprint_dir(dir).unwrap()
}

#[test]
fn d1_create_restore_roundtrip_is_lossless() {
    let tmp = TempDir::new();
    let src = tmp.child("src");
    write_files(&src, &[
        ("save.dat", b"main save"),
        ("sub/dir/deep.bin", &[0u8, 1, 2, 3, 255]),
        ("中文名.sav", "存档内容".as_bytes()),
        ("empty.flag", b""),
    ]);
    let src_fp = dir_fingerprint(&src);

    let store = FsStore::new(tmp.child("repo"));
    let ctx = scan_ctx(&src);
    let stored = store.create("snap_1", &[src.clone()], &ctx).expect("D1: create");

    let out = tmp.child("restored");
    store.restore(&stored.storage_key, &out).expect("D1: restore");

    assert_eq!(dir_fingerprint(&out), src_fp,
        "D1: 往返后内容指纹应一致（含子目录、空文件、中文名、二进制）");
}

#[test]
fn d2_verify_detects_intact_and_corrupt() {
    let tmp = TempDir::new();
    let src = tmp.child("src");
    write_files(&src, &[("a.sav", b"data")]);

    let store = FsStore::new(tmp.child("repo"));
    let ctx = scan_ctx(&src);
    let stored = store.create("snap_v", &[src.clone()], &ctx).expect("create");

    assert!(store.verify(&stored.storage_key).expect("verify ok"), "D2: 完好快照应 verify==true");

    // 破坏物理内容后应 verify==false。
    let snap_dir = tmp.path().join("repo").join("snapshots").join("snap_v");
    if snap_dir.exists() {
        corrupt_dir(&snap_dir);
    }
    assert!(!store.verify(&stored.storage_key).expect("verify ok2"), "D2: 损坏快照应 verify==false");
}

#[test]
fn d3_content_hash_is_order_stable() {
    // 这是 oracle 自检：同一组文件、不同写入顺序，content_hash 必须相同。
    // 该用例测的是已实现的 scan 模块，应直接为绿。
    let tmp = TempDir::new();
    let a = tmp.child("a");
    write_files(&a, &[("x.sav", b"1"), ("y.sav", b"2"), ("z/w.sav", b"3")]);
    let b = tmp.child("b");
    write_files(&b, &[("z/w.sav", b"3"), ("y.sav", b"2"), ("x.sav", b"1")]); // 不同顺序

    assert_eq!(
        scan::fingerprint_dir(&a).unwrap().content_hash,
        scan::fingerprint_dir(&b).unwrap().content_hash,
        "D3: content_hash 必须与文件枚举顺序无关"
    );
}

#[test]
fn d3b_content_hash_changes_with_any_diff() {
    let tmp = TempDir::new();
    let a = tmp.child("a");
    write_files(&a, &[("x.sav", b"same")]);
    let h1 = scan::fingerprint_dir(&a).unwrap().content_hash;

    // 改名（内容相同）→ 指纹应变（路径计入哈希）。
    let b = tmp.child("b");
    write_files(&b, &[("renamed.sav", b"same")]);
    let h2 = scan::fingerprint_dir(&b).unwrap().content_hash;

    assert_ne!(h1, h2, "D3: 仅改文件名也应改变指纹");
}

#[test]
fn d4_storage_key_is_opaque_to_upper_layers() {
    // storage_key 由 store 决定，上层只原样回传。
    // 本用例通过"不同 snapshot_id 命名规则不影响往返正确性"间接保证解耦。
    let tmp = TempDir::new();
    let src = tmp.child("src");
    write_files(&src, &[("s", b"x")]);
    let store = FsStore::new(tmp.child("repo"));
    let ctx = scan_ctx(&src);

    let stored = store.create("任意-不规则_KEY.123", &[src.clone()], &ctx).expect("create");
    let out = tmp.child("out");
    store.restore(&stored.storage_key, &out).expect("restore by opaque key");

    assert_eq!(dir_fingerprint(&out), dir_fingerprint(&src),
        "D4: 上层用不透明 key 即可往返，无需理解其结构");
}

#[test]
fn d5_cross_platform_paths_and_readable_after_restore() {
    let tmp = TempDir::new();
    let src = tmp.child("src");
    // 含多层子目录，验证路径分隔处理。
    write_files(&src, &[("a/b/c/deep.sav", b"deep"), ("top.sav", b"top")]);

    let store = FsStore::new(tmp.child("repo"));
    let ctx = scan_ctx(&src);
    let stored = store.create("snap_p", &[src.clone()], &ctx).expect("create");
    let out = tmp.child("out");
    store.restore(&stored.storage_key, &out).expect("restore");

    // 恢复后文件应可读（权限未破坏）。
    let deep = out.join("a").join("b").join("c").join("deep.sav");
    let content = std::fs::read(&deep).expect("D5: 恢复后文件应可读");
    assert_eq!(content, b"deep");
}

// 让 PathBuf 导入被用到（保持文件自洽）。
#[allow(dead_code)]
fn _types(_p: PathBuf) {}
