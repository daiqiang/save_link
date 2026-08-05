mod common;

use common::Harness;
use savelink_core::model::{CreateOutcome, Reason};
use savelink_core::service::AutoBackupService;

#[test]
fn m1_auto_check_creates_once_then_reports_no_change() {
    let h = Harness::new(&[("save.dat", b"v1")]);
    let service = AutoBackupService::new(h.snapshots());

    let first = service.run_once().expect("首次自动检查应成功");
    assert_eq!(first.checked_game_ids, vec![h.game_id.clone()]);
    assert_eq!(first.created_snapshots.len(), 1);
    assert_eq!(first.created_snapshots[0].reason, Reason::Auto);
    assert!(first.unchanged_game_ids.is_empty());
    assert!(first.failures.is_empty());

    let second = service.run_once().expect("第二次自动检查应成功");
    assert!(second.created_snapshots.is_empty());
    assert_eq!(second.unchanged_game_ids, vec![h.game_id.clone()]);
    assert_eq!(h.timeline().len(), 1, "内容不变不得重复创建快照");
}

#[test]
fn m2_one_game_failure_does_not_leave_partial_snapshot() {
    let h = Harness::new(&[("save.dat", b"v1")]);
    std::fs::remove_dir_all(&h.save_dir).unwrap();

    let report = AutoBackupService::new(h.snapshots())
        .run_once()
        .expect("单个游戏读取失败不应让整轮任务崩溃");

    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].game_id, h.game_id);
    assert!(h.timeline().is_empty());
}

#[test]
fn m3_retention_counts_all_unlocked_sources_and_excludes_locked() {
    let h = Harness::new(&[("save.dat", b"v0")]);
    let snapshots = h.snapshots();

    for index in 0..32 {
        let value = format!("v{index}");
        h.set_save_dir(&[("save.dat", value.as_bytes())]);
        let reason = if index % 2 == 0 { Reason::Manual } else { Reason::Auto };
        let outcome = snapshots
            .create_snapshot(&h.game_id, None, reason)
            .expect("构造快照应成功");
        assert!(matches!(outcome, CreateOutcome::Created(_)));
    }

    let oldest = h.timeline().last().expect("应存在最旧快照").clone();
    snapshots
        .update_meta(&oldest.id, None, Some(true))
        .expect("锁定最旧快照应成功");

    let candidates = AutoBackupService::new(h.snapshots())
        .unlocked_retention_candidates(&h.game_id, 30)
        .expect("计算清理候选应成功");

    assert_eq!(candidates.len(), 1, "32 条中锁定 1 条，只应清理 1 条未锁定记录");
    assert_ne!(candidates[0].id, oldest.id, "锁定记录不得成为清理候选");
    assert!(!candidates[0].locked);
    assert_eq!(h.timeline().len(), 32, "候选计算本身不得删除任何数据");
}
