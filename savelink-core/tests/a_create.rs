//! A 组：创建快照。对应 doc/SaveLink恢复与存储测试规格.md A1-A6。
//!
//! 当前全部红灯（service 为 todo!）。实现 SnapshotService::create_snapshot 后转绿。

mod common;

use common::*;
use savelink_core::error::SaveLinkError;
use savelink_core::model::{CreateOutcome, Reason};
use savelink_core::testkit::{dir_fingerprint, FailKind, FailOp, FailingStore};
use savelink_core::store::FsStore;
use std::sync::Arc;

#[test]
fn a1_normal_create_records_correct_stats_and_content() {
    let h = Harness::new(&[
        ("save.dat", b"hello world"),
        ("config/opt.ini", b"a=1"),
        ("slot/0.sav", b"\x00\x01\x02\x03"),
    ]);
    let before_fp = dir_fingerprint(&h.save_dir);

    let outcome = h
        .snapshots()
        .create_snapshot(&h.game_id, Some("Boss 前".into()), Reason::Manual)
        .expect("create should succeed");

    match outcome {
        CreateOutcome::Created(s) => {
            assert_eq!(s.file_count, 3, "A1: 文件数应为 3");
            assert_eq!(s.reason, Reason::Manual);
            assert_eq!(s.content_hash, before_fp, "A1: 快照指纹应等于存档目录指纹");
        }
        CreateOutcome::NoChange => panic!("A1: 首次创建不应是 NoChange"),
    }
    assert_eq!(h.timeline().len(), 1, "A1: 时间线应新增 1 条");
}

#[test]
fn a2_unchanged_save_does_not_create_duplicate() {
    let h = Harness::new(&[("save.dat", b"hello")]);
    h.snapshots()
        .create_snapshot(&h.game_id, None, Reason::Manual)
        .unwrap();

    let again = h
        .snapshots()
        .create_snapshot(&h.game_id, None, Reason::Manual)
        .unwrap();

    assert_eq!(again, CreateOutcome::NoChange, "A2: 内容未变应返回 NoChange");
    assert_eq!(h.timeline().len(), 1, "A2: 时间线数量不应增加");
}

#[test]
fn a3_modify_file_is_detected_as_change() {
    let h = Harness::new(&[("save.dat", b"v1")]);
    let first = match h
        .snapshots()
        .create_snapshot(&h.game_id, None, Reason::Manual)
        .unwrap()
    {
        CreateOutcome::Created(s) => s,
        _ => panic!("first create"),
    };

    h.set_save_dir(&[("save.dat", b"v2")]); // 改 1 个字节
    let second = match h
        .snapshots()
        .create_snapshot(&h.game_id, None, Reason::Manual)
        .unwrap()
    {
        CreateOutcome::Created(s) => s,
        _ => panic!("A3: 内容变化应创建新快照"),
    };

    assert_ne!(first.content_hash, second.content_hash, "A3: content_hash 应不同");
    assert_eq!(h.timeline().len(), 2);
}

#[test]
fn a3b_add_file_is_detected_as_change() {
    let h = Harness::new(&[("a.sav", b"x")]);
    h.snapshots().create_snapshot(&h.game_id, None, Reason::Manual).unwrap();
    h.set_save_dir(&[("a.sav", b"x"), ("b.sav", b"y")]); // 新增文件
    let r = h.snapshots().create_snapshot(&h.game_id, None, Reason::Manual).unwrap();
    assert!(matches!(r, CreateOutcome::Created(_)), "A3: 新增文件应视为变化");
}

#[test]
fn a3c_delete_file_is_detected_as_change() {
    let h = Harness::new(&[("a.sav", b"x"), ("b.sav", b"y")]);
    h.snapshots().create_snapshot(&h.game_id, None, Reason::Manual).unwrap();
    h.set_save_dir(&[("a.sav", b"x")]); // 删除一个
    let r = h.snapshots().create_snapshot(&h.game_id, None, Reason::Manual).unwrap();
    assert!(matches!(r, CreateOutcome::Created(_)), "A3: 删除文件应视为变化");
}

#[test]
fn a4_failed_create_leaves_no_half_product() {
    // store.create 注入失败，验证事务不留半成品、不留 Writing 悬挂。
    let h = Harness::with_store(&[("save.dat", b"data")], |repo_root| {
        let inner = Arc::new(FsStore::new(repo_root));
        Arc::new(FailingStore::new(inner).fail(FailOp::Create, FailKind::Error))
    });
    let save_fp_before = dir_fingerprint(&h.save_dir);

    let res = h.snapshots().create_snapshot(&h.game_id, None, Reason::Manual);

    assert!(res.is_err(), "A4: 注入失败时创建应报错");
    assert_eq!(h.timeline().len(), 0, "A4: 不应留下快照记录");
    assert!(!any_writing(&h.repo.list_snapshots(&h.game_id).unwrap()),
        "A4: 不应留下 status=Writing 的悬挂记录");
    assert_eq!(dir_fingerprint(&h.save_dir), save_fp_before, "A4: 真实存档不应被触碰");
}

#[test]
fn a5_empty_dir_can_create() {
    let h = Harness::new(&[]); // 空目录
    let outcome = h
        .snapshots()
        .create_snapshot(&h.game_id, None, Reason::Manual)
        .expect("A5: 空目录应允许创建");
    match outcome {
        CreateOutcome::Created(s) => {
            assert_eq!(s.file_count, 0, "A5: 空目录 file_count=0");
            assert_eq!(s.total_size, 0, "A5: 空目录 total_size=0");
        }
        CreateOutcome::NoChange => panic!("A5: 首次创建不应 NoChange"),
    }
}

#[test]
fn a6_unreadable_dir_writes_no_state() {
    // 存档目录指向不存在的位置。
    let h = Harness::new(&[("save.dat", b"x")]);
    let _ = std::fs::remove_dir_all(&h.save_dir); // 删掉，使其不存在

    let res = h.snapshots().create_snapshot(&h.game_id, None, Reason::Manual);

    assert!(
        matches!(res, Err(SaveLinkError::SaveDirMissing) | Err(SaveLinkError::SaveDirUnreadable)),
        "A6: 目录不可读应返回 SaveDirMissing/SaveDirUnreadable，实际: {res:?}"
    );
    assert_eq!(h.timeline().len(), 0, "A6: 不应写入任何记录");
}
