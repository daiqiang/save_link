//! C 组补充：指定快照与当前真实存档的一致性只读比较。

mod common;

use common::Harness;
use savelink_core::error::SaveLinkError;
use savelink_core::model::{CreateOutcome, Reason};

fn create_snapshot(harness: &Harness) -> savelink_core::model::Snapshot {
    match harness
        .snapshots()
        .create_snapshot(&harness.game_id, None, Reason::Manual)
        .expect("快照应创建成功")
    {
        CreateOutcome::Created(snapshot) => snapshot,
        CreateOutcome::NoChange => panic!("测试必须创建一条新快照"),
    }
}

#[test]
fn current_save_matches_a_new_snapshot_without_writing_state() {
    let harness = Harness::new(&[("save.dat", b"same")]);
    let snapshot = create_snapshot(&harness);
    let before = harness.timeline();

    assert!(harness
        .snapshots()
        .current_save_matches(&snapshot.id)
        .expect("只读比较应成功"));
    assert_eq!(harness.timeline(), before, "比较不能修改快照记录");
}

#[test]
fn current_save_reports_different_after_content_changes() {
    let harness = Harness::new(&[("save.dat", b"before")]);
    let snapshot = create_snapshot(&harness);
    harness.set_save_dir(&[("save.dat", b"after")]);

    assert!(!harness
        .snapshots()
        .current_save_matches(&snapshot.id)
        .expect("只读比较应成功"));
}

#[test]
fn current_save_can_match_an_older_snapshot() {
    let harness = Harness::new(&[("save.dat", b"version-1")]);
    let first = create_snapshot(&harness);
    harness.set_save_dir(&[("save.dat", b"version-2")]);
    let second = create_snapshot(&harness);
    harness.set_save_dir(&[("save.dat", b"version-1")]);

    assert!(harness
        .snapshots()
        .current_save_matches(&first.id)
        .expect("旧快照应可独立比较"));
    assert!(!harness
        .snapshots()
        .current_save_matches(&second.id)
        .expect("另一条快照应可独立比较"));
}

#[test]
fn missing_current_save_returns_a_clear_error() {
    let harness = Harness::new(&[("save.dat", b"content")]);
    let snapshot = create_snapshot(&harness);
    std::fs::remove_dir_all(&harness.save_dir).expect("测试目录应可删除");

    assert!(matches!(
        harness.snapshots().current_save_matches(&snapshot.id),
        Err(SaveLinkError::SaveDirMissing)
    ));
}
