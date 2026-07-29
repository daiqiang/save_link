//! B 组：恢复（产品生命线）。对应 doc/SaveLink恢复与存储测试规格.md B1-B14。
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
fn b1_restore_does_not_create_an_automatic_snapshot() {
    let h = Harness::new(&[]);
    let target_id = setup_target_and_current(&h, &[("save.dat", b"TARGET")], &[("save.dat", b"CURRENT")]);
    let before_count = h.timeline().len();

    let out = h.restore().restore_snapshot(&h.game_id, &target_id, &no_progress()).expect("restore ok");

    assert!(out.restored, "B1: 当前与目标不同时应实际执行恢复");
    assert_eq!(h.timeline().len(), before_count, "B1: 恢复不应自动创建快照");
    assert!(!h.timeline().iter().any(|s| s.reason == Reason::BeforeRestore));
}

#[test]
fn b2_restore_preparation_failure_leaves_save_untouched() {
    let h = Harness::new(&[]);
    let target_id = setup_target_and_current(&h, &[("s", b"TARGET")], &[("s", b"CURRENT")]);
    let current_fp = dir_fingerprint(&h.save_dir);

    let res = h
        .restore_failing(FailOp::Restore, FailKind::Error, 0)
        .restore_snapshot(&h.game_id, &target_id, &no_progress());

    assert!(matches!(res, Err(SaveLinkError::RestoreFailed { rolled_back: true })), "B2: 应返回已回滚的恢复失败，实际 {res:?}");
    assert_eq!(dir_fingerprint(&h.save_dir), current_fp, "B2: 真实存档应一字节未动");
    assert_eq!(h.timeline().len(), 1, "B2: 失败不应创建快照");
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
    assert_eq!(h.timeline().len(), 1, "B5: 失败不应创建额外快照");
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
            assert!(rolled_back, "B6: 尚未替换真实目录的失败必须报告已回滚");
            assert_eq!(dir_fingerprint(&h.save_dir), current_fp,
                "B6: rolled_back=true 时真实存档应等于操作前");
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

    let out = h
        .restore()
        .restore_with_choice(
            &h.game_id,
            &target_id,
            MissingDirChoice::CreateAndRestore,
            &no_progress(),
        )
        .expect("B7: 用户确认后应创建目录并恢复");
    assert!(out.restored);
    assert!(!h.timeline().iter().any(|s| s.reason == Reason::BeforeRestore));
}

#[test]
fn b8_can_round_trip_between_two_existing_snapshots_without_creating_more() {
    let h = Harness::new(&[]);
    h.set_save_dir(&[("s", b"TARGET")]);
    let target_id = match h.snapshots().create_snapshot(&h.game_id, Some("目标".into()), Reason::Manual).unwrap() {
        savelink_core::model::CreateOutcome::Created(s) => s.id,
        _ => panic!("B8: 应创建目标快照"),
    };
    h.set_save_dir(&[("s", b"ORIGINAL")]);
    let original_id = match h.snapshots().create_snapshot(&h.game_id, Some("原状态".into()), Reason::Manual).unwrap() {
        savelink_core::model::CreateOutcome::Created(s) => s.id,
        _ => panic!("B8: 应创建原状态快照"),
    };
    let original_fp = dir_fingerprint(&h.save_dir);
    let timeline_count = h.timeline().len();

    h.restore().restore_snapshot(&h.game_id, &target_id, &no_progress()).expect("restore target");
    h.restore().restore_snapshot(&h.game_id, &original_id, &no_progress()).expect("restore original");

    assert_eq!(dir_fingerprint(&h.save_dir), original_fp, "B8: 可恢复已有快照回到原状态");
    assert_eq!(h.timeline().len(), timeline_count, "B8: 往返恢复不应制造重复快照");
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
        vec![RestoreStep::RestoreTarget, RestoreStep::Verify],
        "B9: 进度事件顺序应为 恢复→校验"
    );
}

#[test]
fn b10_restore_from_empty_directory_does_not_create_a_snapshot() {
    let h = Harness::new(&[]);
    let target_id = setup_target_and_current(&h, &[("s", b"TARGET")], &[]);
    let before_count = h.timeline().len();

    let out = h.restore().restore_snapshot(&h.game_id, &target_id, &no_progress()).expect("restore ok");

    assert!(out.restored);
    assert_eq!(h.timeline().len(), before_count, "B10: 空目录恢复不应创建快照");
}

#[test]
fn b11_current_already_equals_target_is_a_noop() {
    let h = Harness::new(&[("s", b"TARGET")]);
    let target = match h
        .snapshots()
        .create_snapshot(&h.game_id, Some("当前版本".into()), Reason::Manual)
        .unwrap()
    {
        savelink_core::model::CreateOutcome::Created(snapshot) => snapshot,
        _ => panic!("B11: 应创建目标快照"),
    };
    let steps: Arc<Mutex<Vec<RestoreStep>>> = Arc::new(Mutex::new(Vec::new()));
    let steps_c = steps.clone();
    let sink = move |step: RestoreStep| steps_c.lock().unwrap().push(step);

    let out = h
        .restore_failing(FailOp::Restore, FailKind::Error, 0)
        .restore_snapshot(&h.game_id, &target.id, &sink)
        .expect("B11: 已是目标版本时不应调用 store.restore");

    assert!(!out.restored);
    assert!(steps.lock().unwrap().is_empty(), "B11: 无操作恢复不应发出虚假进度");
    assert_eq!(h.timeline().len(), 1, "B11: 不应创建重复快照");
}

#[test]
fn b12_legacy_before_restore_snapshot_remains_restorable() {
    let h = Harness::new(&[]);
    h.set_save_dir(&[("s", b"LEGACY")]);
    let legacy = match h
        .snapshots()
        .create_snapshot(&h.game_id, Some("旧版恢复前自动备份".into()), Reason::BeforeRestore)
        .unwrap()
    {
        savelink_core::model::CreateOutcome::Created(snapshot) => snapshot,
        _ => panic!("B12: 应创建旧版快照"),
    };
    h.set_save_dir(&[("s", b"CURRENT")]);

    h.restore().restore_snapshot(&h.game_id, &legacy.id, &no_progress()).expect("restore legacy");

    assert_eq!(std::fs::read(h.save_dir.join("s")).unwrap(), b"LEGACY");
    assert_eq!(h.timeline().len(), 1, "B12: 恢复历史快照不应创建新快照");
}

#[test]
fn b13_final_verification_failure_rolls_back_original_directory() {
    let h = Harness::new(&[]);
    let target_id = setup_target_and_current(&h, &[("s", b"TARGET")], &[("s", b"CURRENT")]);
    let current_fp = dir_fingerprint(&h.save_dir);
    let save_dir = h.save_dir.clone();
    let sink = move |step: RestoreStep| {
        if step == RestoreStep::Verify {
            std::fs::write(save_dir.join("tampered-after-replace"), b"BROKEN").unwrap();
        }
    };

    let res = h.restore().restore_snapshot(&h.game_id, &target_id, &sink);

    assert!(matches!(res, Err(SaveLinkError::RestoreFailed { rolled_back: true })));
    assert_eq!(dir_fingerprint(&h.save_dir), current_fp, "B13: 最终校验失败后必须恢复原目录");
    assert_eq!(h.timeline().len(), 1, "B13: 回滚过程不应创建快照");
}
