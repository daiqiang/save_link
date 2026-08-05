mod common;

use common::Harness;
use chrono::{Duration, Local, TimeZone};
use savelink_core::model::{CreateOutcome, Reason};
use savelink_core::repo::{InMemoryRepo, Repository};
use savelink_core::service::AutoBackupService;
use std::sync::Arc;

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

#[test]
fn m4_mixed_time_formats_use_the_actual_latest_snapshot_for_deduplication() {
    let repo: Arc<dyn Repository> = Arc::new(InMemoryRepo::new());
    let h = Harness::with_repo(&[("save.dat", b"v1")], repo);
    let snapshots = h.snapshots();

    let mut older = match snapshots
        .create_snapshot(&h.game_id, None, Reason::Manual)
        .unwrap()
    {
        CreateOutcome::Created(snapshot) => snapshot,
        _ => panic!("第一条快照应创建"),
    };
    h.set_save_dir(&[("save.dat", b"v2")]);
    let mut latest = match snapshots
        .create_snapshot(&h.game_id, None, Reason::Manual)
        .unwrap()
    {
        CreateOutcome::Created(snapshot) => snapshot,
        _ => panic!("第二条快照应创建"),
    };
    let latest_time = Local
        .with_ymd_and_hms(2026, 8, 5, 22, 0, 0)
        .earliest()
        .unwrap();
    older.created_at = (latest_time - Duration::minutes(35)).to_rfc3339();
    latest.created_at = latest_time.format("%Y-%m-%d %H:%M").to_string();
    h.repo.update_snapshot(older).unwrap();
    h.repo.update_snapshot(latest.clone()).unwrap();

    let report = AutoBackupService::new(h.snapshots()).run_once().unwrap();
    assert!(report.created_snapshots.is_empty());
    assert_eq!(report.unchanged_game_ids, vec![h.game_id.clone()]);
    assert_eq!(h.timeline()[0].id, latest.id, "必须选择真实的最近快照做去重");
    assert_eq!(h.timeline().len(), 2, "内容未变化不能生成重复快照");
}

#[test]
fn m5_retention_removes_the_actual_oldest_snapshot_with_mixed_time_formats() {
    let repo: Arc<dyn Repository> = Arc::new(InMemoryRepo::new());
    let h = Harness::with_repo(&[("save.dat", b"v0")], repo);
    let snapshots = h.snapshots();
    let base = Local
        .with_ymd_and_hms(2026, 8, 5, 20, 0, 0)
        .earliest()
        .unwrap();
    let mut created = Vec::new();

    for index in 0..31 {
        let value = format!("v{index}");
        h.set_save_dir(&[("save.dat", value.as_bytes())]);
        let mut snapshot = match snapshots
            .create_snapshot(&h.game_id, None, Reason::Auto)
            .unwrap()
        {
            CreateOutcome::Created(snapshot) => snapshot,
            _ => panic!("构造快照应成功"),
        };
        let timestamp = base + Duration::minutes(index);
        snapshot.created_at = if index % 2 == 0 {
            timestamp.to_rfc3339()
        } else {
            timestamp.format("%Y-%m-%d %H:%M").to_string()
        };
        h.repo.update_snapshot(snapshot.clone()).unwrap();
        created.push(snapshot);
    }

    let candidates = AutoBackupService::new(h.snapshots())
        .unlocked_retention_candidates(&h.game_id, 30)
        .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0].id, created[0].id,
        "31 条混合来源快照只能淘汰真实时间最旧的一条"
    );
}
