//! 快照时间线显示区域与十分钟维护整理规则。

mod common;

use common::*;
use savelink_core::model::{CreateOutcome, Reason, SnapshotDisplayZone};
use savelink_core::service::AutoBackupService;

fn create_changed_snapshot(h: &Harness, marker: u8) -> savelink_core::model::Snapshot {
    h.set_save_dir(&[("save.dat", &[marker])]);
    match h
        .snapshots()
        .create_snapshot(&h.game_id, None, Reason::Manual)
        .unwrap()
    {
        CreateOutcome::Created(snapshot) => snapshot,
        CreateOutcome::NoChange => panic!("测试数据应产生新快照"),
    }
}

#[test]
fn lock_and_unlock_keep_the_old_zone_until_maintenance() {
    let h = Harness::new(&[("save.dat", &[0])]);
    let snapshot = create_changed_snapshot(&h, 1);

    h.snapshots()
        .update_meta(&snapshot.id, None, Some(true))
        .unwrap();
    let pending_lock = h.repo.get_snapshot(&snapshot.id).unwrap().unwrap();
    assert_eq!(pending_lock.display_zone, SnapshotDisplayZone::Normal);
    assert!(pending_lock.display_zone.is_pending(pending_lock.locked));

    h.snapshots()
        .organize_snapshot_layout(&h.game_id, 10)
        .unwrap();
    let organized_lock = h.repo.get_snapshot(&snapshot.id).unwrap().unwrap();
    assert_eq!(organized_lock.display_zone, SnapshotDisplayZone::Locked);
    assert!(!organized_lock
        .display_zone
        .is_pending(organized_lock.locked));

    h.snapshots()
        .update_meta(&snapshot.id, None, Some(false))
        .unwrap();
    let pending_unlock = h.repo.get_snapshot(&snapshot.id).unwrap().unwrap();
    assert_eq!(pending_unlock.display_zone, SnapshotDisplayZone::Locked);
    assert!(pending_unlock
        .display_zone
        .is_pending(pending_unlock.locked));

    h.snapshots()
        .organize_snapshot_layout(&h.game_id, 10)
        .unwrap();
    let organized_unlock = h.repo.get_snapshot(&snapshot.id).unwrap().unwrap();
    assert_eq!(organized_unlock.display_zone, SnapshotDisplayZone::Normal);
    assert!(!organized_unlock
        .display_zone
        .is_pending(organized_unlock.locked));
}

#[test]
fn unlocked_snapshot_outside_retention_stays_pending_until_delete_succeeds() {
    let h = Harness::new(&[("save.dat", &[0])]);
    let mut snapshots = Vec::new();
    for marker in 1..=4 {
        snapshots.push(create_changed_snapshot(&h, marker));
    }
    let oldest = snapshots.first().unwrap().clone();

    h.snapshots()
        .update_meta(&oldest.id, None, Some(true))
        .unwrap();
    h.snapshots()
        .organize_snapshot_layout(&h.game_id, 2)
        .unwrap();
    assert_eq!(
        h.repo
            .get_snapshot(&oldest.id)
            .unwrap()
            .unwrap()
            .display_zone,
        SnapshotDisplayZone::Locked
    );

    h.snapshots()
        .update_meta(&oldest.id, None, Some(false))
        .unwrap();
    h.snapshots()
        .organize_snapshot_layout(&h.game_id, 2)
        .unwrap();
    let pending = h.repo.get_snapshot(&oldest.id).unwrap().unwrap();
    assert_eq!(pending.display_zone, SnapshotDisplayZone::Locked);
    assert!(pending.display_zone.is_pending(pending.locked));

    let retention = AutoBackupService::new(h.snapshots())
        .unlocked_retention_candidates(&h.game_id, 2)
        .unwrap()
        .into_iter()
        .map(|snapshot| snapshot.id)
        .collect::<Vec<_>>();
    assert!(retention.contains(&oldest.id));

    h.snapshots().delete_snapshot(&oldest.id).unwrap();
    assert!(h.repo.get_snapshot(&oldest.id).unwrap().is_none());
}
