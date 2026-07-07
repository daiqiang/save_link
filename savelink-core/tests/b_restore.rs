//! B 组：恢复（产品生命线）。对应 savelink-restore-test-spec.md B1–B9。
//!
//! 每条都以「真实存档目录的内容指纹」为最终裁判。全部红灯，实现 RestoreService 后转绿。

mod common;

use common::*;
use savelink_core::error::SaveLinkError;
use savelink_core::model::{MissingDirChoice, Reason, RestoreStep};
use savelink_core::testkit::{corrupt_dir, dir_fingerprint, FailKind, FailOp};
use std::sync::{Arc, Mutex};

/// 造两个快照：T(旧目标) 与 当前状态 S。返回目标快照 id。
/// 步骤：写 T 内容 → 建快照T → 改存档为 S（不建快照，模拟"玩到了新进度"）。
fn setup_target_and_current(h: &Harness, target: &[(&str, &[u8])], current: &[(&str, &[u8])]) -> String {
    h.set_save_dir(target);
    let t = match h.snapshots().create_snapshot(&h.game_id, Some("目标版本".into()), Reason::Manual).unwrap() {
        savelink_core::model::CreateOutcome::Created(s) => s.id,
        _ => panic!("setup: target create"),
    };
    h.set_save_dir(current);
    t
}

#[test]
fn b1_backup_before_restore_captures_current_state() {
    let h = Harness::new(&[]);
    let target_id = setup_target_and_current(&h, &[("save.dat", b"TARGET")], &[("save.dat", b"CURRENT")]);
    let current_fp = dir_fingerprint(&h.save_dir);

    let out = h.restore().restore_snapshot(&h.game_id, &target_id, &no_progress()).expect("restore ok");

    let timeline = h.timeline();
    let backup = timeline.iter().find(|s| s.id == out.backup_id).expect("B1: 应有 before_restore 备份");
    assert_eq!(backup.reason, Reason::BeforeRestore, "B1: 备份 reason 应为 BeforeRestore");
    assert_eq!(backup.content_hash, current_fp, "B1: 备份内容应等于恢复前的真实存档（不是目标）");
}

#[test]
fn b2_backup_failure_aborts_and_leaves_save_untouched() {
    let h = Harness::new(&[]);
    let target_id = setup_target_and_current(&h, &[("s", b"TARGET")], &[("s", b"CURRENT")]);
    let current_fp = dir_fingerprint(&h.save_dir);

    // before_restore 备份 = 一次 create。注入 create 失败（每次命中都失败）。
    let res = h
        .restore_failing(FailOp::Create, FailKind::Error, 0)
        .restore_snapshot(&h.game_id, &target_id, &no_progress());

    assert!(matches!(res, Err(SaveLinkError::BackupFailed)), "B2: 备份失败应返回 BackupFailed，实际 {res:?}");
    assert_eq!(dir_fingerprint(&h.save_dir), current_fp, "B2: 真实存档应一字节未动");
}

#[test]
fn b3_restore_makes_save_equal_target_and_removes_old_files() {
    let h = Harness::new(&[]);
    // 目标含 a,b；当前含 a(改),c —— 恢复后应只剩目标的 a,b，且 c 不残留。
    let target_id = setup_target_and_current(
        &h,
        &[("a.sav", b"A"), ("b.sav", b"B")],
        &[("a.sav", b"A-modified"), ("c.sav", b"C")],
    );
    // 计算目标指纹：临时还原一份目标内容来取指纹。
    let probe = h.tmp.child("probe_target");
    savelink_core::testkit::write_files(&probe, &[("a.sav", b"A"), ("b.sav", b"B")]);
    let target_fp = dir_fingerprint(&probe);

    h.restore().restore_snapshot(&h.game_id, &target_id, &no_progress()).expect("restore ok");

    assert_eq!(dir_fingerprint(&h.save_dir), target_fp, "B3: 恢复后存档应逐字节等于目标");
    assert!(!h.save_dir.join("c.sav").exists(), "B3: 不应残留只属于当前状态的 c.sav（覆盖须是替换而非合并）");
}

#[test]
fn b3b_restore_small_file_snapshot_keeps_files_in_chinese_path() {
    let h = Harness::new(&[]);
    let chinese_save_dir = h.tmp.child("模拟存档").join("艾尔登法环");
    std::fs::create_dir_all(&chinese_save_dir).unwrap();
    let game = savelink_core::model::Game {
        id: h.game_id.clone(),
        name: "EldenRing".into(),
        icon: None,
        repo_path: h.tmp.path().join("repo"),
        save_paths: vec![chinese_save_dir.clone()],
        created_at: h.clock.now_stamp(),
        updated_at: h.clock.now_stamp(),
    };
    h.repo.update_game(game).unwrap();

    savelink_core::testkit::write_files(&chinese_save_dir, &[("1.txt", "2026-06-23 第一行数据".as_bytes())]);
    let target = match h.snapshots().create_snapshot(&h.game_id, Some("第一个存档快照".into()), Reason::Manual).unwrap() {
        savelink_core::model::CreateOutcome::Created(s) => s,
        _ => panic!("setup: target create"),
    };
    assert_eq!(target.file_count, 1, "B3b: 小文件快照也应记录 1 个文件");
    assert!(target.total_size > 0, "B3b: 小文件快照大小应大于 0 字节");

    let _ = std::fs::remove_dir_all(&chinese_save_dir);
    savelink_core::testkit::write_files(
        &chinese_save_dir,
        &[("1.txt", "2026-06-23 第一行数据\r\n2026-07-06 第二行数据".as_bytes())],
    );

    h.restore().restore_snapshot(&h.game_id, &target.id, &no_progress()).expect("restore ok");

    let restored = chinese_save_dir.join("1.txt");
    assert!(restored.exists(), "B3b: 恢复后真实目录必须保留目标快照文件，不能变成空目录");
    assert_eq!(
        std::fs::read_to_string(restored).unwrap(),
        "2026-06-23 第一行数据",
        "B3b: 恢复后文件内容应等于目标小文件快照"
    );
    let restored_scan = savelink_core::scan::fingerprint_dir(&chinese_save_dir).unwrap();
    assert_eq!(restored_scan.file_count, 1, "B3b: 恢复后真实目录文件数不能为 0");
    assert_eq!(restored_scan.total_size, target.total_size, "B3b: 恢复后真实目录大小应等于目标快照");
    assert_eq!(restored_scan.content_hash, target.content_hash, "B3b: 恢复后真实目录指纹应等于目标快照");
}

#[test]
fn b4_corrupt_target_does_not_touch_save() {
    let h = Harness::new(&[]);
    let target_id = setup_target_and_current(&h, &[("s", b"TARGET")], &[("s", b"CURRENT")]);
    let current_fp = dir_fingerprint(&h.save_dir);

    // 破坏目标快照物理内容（FsStore 布局：repo/snapshots/{id}/...）。
    let snap_dir = h.tmp.path().join("repo").join("snapshots").join(&target_id);
    if snap_dir.exists() {
        corrupt_dir(&snap_dir);
    }

    let res = h.restore().restore_snapshot(&h.game_id, &target_id, &no_progress());

    assert!(matches!(res, Err(SaveLinkError::SnapshotCorrupt)), "B4: 损坏目标应返回 SnapshotCorrupt，实际 {res:?}");
    assert_eq!(dir_fingerprint(&h.save_dir), current_fp, "B4: 真实存档不应被触碰");
}

#[test]
fn b5_crash_during_overwrite_leaves_consistent_state() {
    // 注入覆盖阶段（store.restore）失败/中断，断言不出现"一半一半"。
    let h = Harness::new(&[]);
    let target_id = setup_target_and_current(&h, &[("s", b"TARGET")], &[("s", b"CURRENT")]);
    let current_fp = dir_fingerprint(&h.save_dir);

    let probe = h.tmp.child("probe_b5");
    savelink_core::testkit::write_files(&probe, &[("s", b"TARGET")]);
    let target_fp = dir_fingerprint(&probe);

    let res = h
        .restore_failing(FailOp::Restore, FailKind::PartialThenError, 0)
        .restore_snapshot(&h.game_id, &target_id, &no_progress());

    assert!(res.is_err(), "B5: 覆盖阶段失败应报错");
    let fp = dir_fingerprint(&h.save_dir);
    assert!(
        fp == current_fp || fp == target_fp,
        "B5: 崩溃后存档必须是完整旧态或完整新态，不允许混合态"
    );
    let has_backup = h.timeline().iter().any(|s| s.reason == Reason::BeforeRestore);
    assert!(has_backup, "B5: 必须存在 before_restore 备份，用户始终有回退点");
}

#[test]
fn b6_restore_failure_carries_rolled_back_semantics() {
    let h = Harness::new(&[]);
    let target_id = setup_target_and_current(&h, &[("s", b"TARGET")], &[("s", b"CURRENT")]);
    let current_fp = dir_fingerprint(&h.save_dir);

    let res = h
        .restore_failing(FailOp::Restore, FailKind::Error, 0)
        .restore_snapshot(&h.game_id, &target_id, &no_progress());

    match res {
        Err(SaveLinkError::RestoreFailed { rolled_back }) => {
            if rolled_back {
                assert_eq!(dir_fingerprint(&h.save_dir), current_fp,
                    "B6: rolled_back=true 时真实存档应等于操作前");
            } else {
                assert!(h.timeline().iter().any(|s| s.reason == Reason::BeforeRestore),
                    "B6: rolled_back=false 时必须保留 before_restore 备份");
            }
        }
        other => panic!("B6: 应返回 RestoreFailed{{rolled_back}}，实际 {other:?}"),
    }
}

#[test]
fn b7_missing_save_dir_needs_user_choice() {
    let h = Harness::new(&[]);
    let target_id = setup_target_and_current(&h, &[("s", b"TARGET")], &[("s", b"CURRENT")]);
    let _ = std::fs::remove_dir_all(&h.save_dir); // 存档目录被删

    let res = h.restore().restore_snapshot(&h.game_id, &target_id, &no_progress());

    assert!(
        matches!(res, Err(SaveLinkError::SaveDirMissingNeedsChoice)),
        "B7: 目录不存在应要求用户决策，而非静默创建，实际 {res:?}"
    );
    assert!(!h.save_dir.exists(), "B7: 用户未确认前不应在该路径写入");

    // 用户选择"取消"——仍不写入。
    let res2 = h.restore().restore_with_choice(&h.game_id, &target_id, MissingDirChoice::Cancel, &no_progress());
    assert!(res2.is_err());
    assert!(!h.save_dir.exists(), "B7: 取消后仍不写入");
}

#[test]
fn b8_can_restore_backup_to_return_to_original() {
    let h = Harness::new(&[]);
    let target_id = setup_target_and_current(&h, &[("s", b"TARGET")], &[("s", b"ORIGINAL")]);
    let original_fp = dir_fingerprint(&h.save_dir);

    // 第一次恢复到目标。
    let out = h.restore().restore_snapshot(&h.game_id, &target_id, &no_progress()).expect("restore 1");
    // 现在再恢复刚生成的 before_restore（内容 == ORIGINAL），应回到原状。
    let out2 = h.restore().restore_snapshot(&h.game_id, &out.backup_id, &no_progress()).expect("restore 2");

    assert_eq!(dir_fingerprint(&h.save_dir), original_fp, "B8: 恢复 before_restore 应回到原始状态");
    assert_ne!(out2.backup_id, out.backup_id, "B8: 第二次恢复应再生成新的 before_restore");
}

#[test]
fn b9_progress_events_in_order() {
    let h = Harness::new(&[]);
    let target_id = setup_target_and_current(&h, &[("s", b"TARGET")], &[("s", b"CURRENT")]);

    let steps: Arc<Mutex<Vec<RestoreStep>>> = Arc::new(Mutex::new(Vec::new()));
    let steps_c = steps.clone();
    let sink = move |step: RestoreStep| steps_c.lock().unwrap().push(step);

    h.restore().restore_snapshot(&h.game_id, &target_id, &sink).expect("restore ok");

    let got = steps.lock().unwrap().clone();
    assert_eq!(
        got,
        vec![RestoreStep::BackupCurrent, RestoreStep::RestoreTarget, RestoreStep::Verify],
        "B9: 进度事件顺序应为 备份→恢复→校验"
    );
}
