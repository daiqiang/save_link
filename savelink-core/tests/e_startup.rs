//! E 组：中断恢复与启动自检。对应 spec E1–E2。

mod common;

use common::*;
use savelink_core::model::{Reason, Snapshot, SnapshotStatus};
use savelink_core::service::{same_volume, startup_self_check};
use savelink_core::store::SnapshotStore;
use std::sync::Arc;

#[test]
fn e1_startup_clears_dangling_writing_snapshots() {
    let h = Harness::new(&[("s", b"x")]);

    // 手动塞入一条 status=Writing 的残留记录（模拟上次创建中崩溃）。
    let dangling = Snapshot {
        id: "snap_dangling".into(),
        game_id: h.game_id.clone(),
        created_at: h.clock.now_stamp(),
        note: Some("半成品".into()),
        reason: Reason::Manual,
        locked: false,
        file_count: 0,
        total_size: 0,
        source_count: 1,
        content_hash: "deadbeef".into(),
        storage_key: "snap_dangling".into(),
        status: SnapshotStatus::Writing,
    };
    h.repo.insert_snapshot(dangling).unwrap();

    startup_self_check(&h.repo, &h.store).expect("E1: 启动自检应成功");

    // 清理后：不得作为正常（Complete）快照出现在时间线。
    let visible: Vec<_> = h
        .timeline()
        .into_iter()
        .filter(|s| s.status == SnapshotStatus::Complete)
        .collect();
    assert!(
        !visible.iter().any(|s| s.id == "snap_dangling"),
        "E1: 残留 Writing 快照不得出现在正常时间线"
    );
    // 也不应仍处于 Writing 悬挂态。
    assert!(
        !any_writing(&h.repo.list_snapshots(&h.game_id).unwrap()),
        "E1: 自检后不应再有 Writing 悬挂记录"
    );
}

#[test]
fn e2_same_volume_detection_drives_atomic_strategy() {
    // 同一临时目录下两路径必然同卷 → Some(true)。
    let h = Harness::new(&[]);
    let a = h.tmp.child("vol_a");
    let b = h.tmp.child("vol_b");

    match same_volume(&a, &b) {
        Some(true) => { /* 期望：同卷可用原子 rename */ }
        Some(false) => panic!("E2: 同一临时根下的两目录应判定为同卷"),
        None => panic!("E2: same_volume 不应对本地同卷目录返回 None（无法判定）"),
    }
}

#[test]
fn e3_startup_finishes_interrupted_local_deletion() {
    let h = Harness::new(&[("s", b"x")]);
    let created = match h
        .snapshots()
        .create_snapshot(&h.game_id, None, Reason::Manual)
        .unwrap()
    {
        savelink_core::model::CreateOutcome::Created(snapshot) => snapshot,
        _ => panic!("应创建测试快照"),
    };
    let mut deleting = created.clone();
    deleting.status = SnapshotStatus::Deleting;
    h.repo.update_snapshot(deleting).unwrap();

    startup_self_check(&h.repo, &h.store).expect("启动自检应续做删除");

    assert!(h.repo.get_snapshot(&created.id).unwrap().is_none());
    assert!(!h.store.verify(&created.storage_key).unwrap());
}

// 保持 Arc 导入被使用。
#[allow(dead_code)]
fn _types(_s: Arc<dyn SnapshotStore>) {}
