//! C 组：删除与锁定（安全规则 4 / 3）。对应 spec C1–C4。

mod common;

use common::*;
use savelink_core::error::SaveLinkError;
use savelink_core::model::{CreateOutcome, Reason, Snapshot};
use savelink_core::testkit::{FailKind, FailOp, FailingStore};
use savelink_core::service::SnapshotService;
use savelink_core::store::FsStore;
use std::sync::Arc;

fn create_one(h: &Harness, note: &str) -> Snapshot {
    match h.snapshots().create_snapshot(&h.game_id, Some(note.into()), Reason::Manual).unwrap() {
        CreateOutcome::Created(s) => s,
        _ => panic!("create_one"),
    }
}

#[test]
fn c1_locked_snapshot_cannot_be_deleted() {
    let h = Harness::new(&[("s", b"x")]);
    let snap = create_one(&h, "锁定的");
    h.snapshots().update_meta(&snap.id, None, Some(true)).unwrap();

    let res = h.snapshots().delete_snapshot(&snap.id);

    assert!(matches!(res, Err(SaveLinkError::SnapshotLocked)), "C1: 锁定快照删除应返回 SnapshotLocked");
    assert!(h.repo.get_snapshot(&snap.id).unwrap().is_some(), "C1: 记录应保留");
}

#[test]
fn c2_unlock_then_delete_succeeds() {
    let h = Harness::new(&[("s", b"x")]);
    let snap = create_one(&h, "先锁后删");
    h.snapshots().update_meta(&snap.id, None, Some(true)).unwrap();
    h.snapshots().update_meta(&snap.id, None, Some(false)).unwrap();

    h.snapshots().delete_snapshot(&snap.id).expect("C2: 取消锁定后应可删除");

    assert!(h.repo.get_snapshot(&snap.id).unwrap().is_none(), "C2: 记录应被删除");
    assert_eq!(h.timeline().len(), 0);
}

#[test]
fn c3_delete_failure_rolls_back_no_dangling() {
    // store.delete 注入失败：不应留下"有记录无文件"或"有文件无记录"的悬挂。
    let h = Harness::with_store(&[("s", b"x")], |repo_root| {
        let inner = Arc::new(FsStore::new(repo_root));
        Arc::new(FailingStore::new(inner).fail(FailOp::Delete, FailKind::Error))
    });
    let snap = match SnapshotService::new(h.repo.clone(), h.store.clone(), h.clock.clone(), h.ids.clone())
        .create_snapshot(&h.game_id, Some("待删".into()), Reason::Manual)
        .unwrap()
    {
        CreateOutcome::Created(s) => s,
        _ => panic!(),
    };

    let res = h.snapshots().delete_snapshot(&snap.id);

    assert!(res.is_err(), "C3: 删文件失败应报错");
    assert!(h.repo.get_snapshot(&snap.id).unwrap().is_some(),
        "C3: 删文件失败时记录应保留（避免有记录无文件的反向悬挂）");
    assert_eq!(
        h.repo.get_snapshot(&snap.id).unwrap().unwrap().status,
        savelink_core::model::SnapshotStatus::Complete,
        "C3: 可恢复失败不应把快照永久留在 Deleting"
    );
}

#[test]
fn c4_metadata_mutable_content_immutable() {
    let h = Harness::new(&[("s", b"x")]);
    let snap = create_one(&h, "原备注");

    h.snapshots().update_meta(&snap.id, Some("新备注".into()), Some(true)).unwrap();
    let after = h.repo.get_snapshot(&snap.id).unwrap().unwrap();

    assert_eq!(after.note.as_deref(), Some("新备注"), "C4: note 应可改");
    assert!(after.locked, "C4: locked 应可改");
    // 内容相关字段不可变：
    assert_eq!(after.content_hash, snap.content_hash, "C4: content_hash 不可变");
    assert_eq!(after.created_at, snap.created_at, "C4: created_at 不可变");
    assert_eq!(after.file_count, snap.file_count, "C4: file_count 不可变");
    assert_eq!(after.total_size, snap.total_size, "C4: total_size 不可变");
}

#[test]
fn c5_delete_game_removes_metadata_and_keeps_real_save() {
    let h = Harness::new(&[("s", b"x")]);
    let snap = create_one(&h, "待随游戏移除");

    h.snapshots().delete_game(&h.game_id).expect("C5: 游戏应可移除");

    assert!(h.repo.get_game(&h.game_id).unwrap().is_none(), "C5: 游戏记录应被删除");
    assert!(h.repo.get_snapshot(&snap.id).unwrap().is_none(), "C5: 快照记录应被删除");
    assert!(h.save_dir.join("s").exists(), "C5: 真实存档目录和文件不得被删除");
}

#[test]
fn c6_delete_game_stops_if_snapshot_file_delete_fails() {
    let h = Harness::with_store(&[("s", b"x")], |repo_root| {
        let inner = Arc::new(FsStore::new(repo_root));
        Arc::new(FailingStore::new(inner).fail(FailOp::Delete, FailKind::Error))
    });
    let snap = match SnapshotService::new(h.repo.clone(), h.store.clone(), h.clock.clone(), h.ids.clone())
        .create_snapshot(&h.game_id, Some("待删".into()), Reason::Manual)
        .unwrap()
    {
        CreateOutcome::Created(s) => s,
        _ => panic!(),
    };

    let res = h.snapshots().delete_game(&h.game_id);

    assert!(res.is_err(), "C6: 快照文件删除失败时应报错");
    assert!(h.repo.get_game(&h.game_id).unwrap().is_some(), "C6: 游戏记录应保留");
    assert!(h.repo.get_snapshot(&snap.id).unwrap().is_some(), "C6: 快照记录应保留");
}
