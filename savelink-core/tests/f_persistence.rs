//! F 组：SQLite 持久化验证（第 2 步新增）。
//!
//! 证明真数据库的核心价值：数据落盘、关机不丢。
//! 与 A–E 组互补——那些验证逻辑正确，这里验证持久性。

mod common;

use common::*;
use savelink_core::model::Game;
use savelink_core::model::{CreateOutcome, Reason};
use savelink_core::repo::Repository;
use savelink_core::sqlite_repo::SqliteRepo;
use savelink_core::testkit::TempDir;
use std::path::PathBuf;
use std::sync::Arc;

#[test]
fn f1_data_survives_reopen() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("savelink.db");

    // 第一次：开库、装配服务、创建一个快照，然后丢弃连接（模拟关机）。
    let snap_id;
    {
        let repo: Arc<dyn Repository> = Arc::new(SqliteRepo::open(&db_path).unwrap());
        let h = Harness::with_repo(&[("save.dat", b"hello")], repo);
        let s = match h
            .snapshots()
            .create_snapshot(&h.game_id, Some("持久化测试".into()), Reason::Manual)
            .unwrap()
        {
            CreateOutcome::Created(s) => s,
            _ => panic!("create"),
        };
        snap_id = s.id;
    } // repo / 连接在此释放——相当于程序关闭

    // 第二次：重新打开同一个 .db 文件，数据应仍在。
    {
        let repo = SqliteRepo::open(&db_path).unwrap();
        let got = repo.get_snapshot(&snap_id).unwrap();
        assert!(got.is_some(), "F1: 重新打开数据库后，快照记录应仍然存在（数据已落盘）");
        let snap = got.unwrap();
        assert_eq!(snap.note.as_deref(), Some("持久化测试"), "F1: 备注应原样保留");
        assert_eq!(snap.reason, Reason::Manual, "F1: reason 应正确反序列化");

        let games = repo.list_games().unwrap();
        assert_eq!(games.len(), 1, "F1: 游戏记录也应持久化");
    }
}

#[test]
fn f2_enum_roundtrip_through_sql() {
    // 各种 reason / locked 经 SQL 存取后语义不变。
    let tmp = TempDir::new();
    let db_path = tmp.path().join("db2.db");
    let repo: Arc<dyn Repository> = Arc::new(SqliteRepo::open(&db_path).unwrap());
    let h = Harness::with_repo(&[("s", b"x")], repo);

    let s = match h.snapshots().create_snapshot(&h.game_id, None, Reason::BeforeRestore).unwrap() {
        CreateOutcome::Created(s) => s,
        _ => panic!(),
    };
    h.snapshots().update_meta(&s.id, None, Some(true)).unwrap();

    let got = h.repo.get_snapshot(&s.id).unwrap().unwrap();
    assert_eq!(got.reason, Reason::BeforeRestore, "F2: before_restore 经 SQL 往返应不变");
    assert!(got.locked, "F2: locked 经 SQL 往返应不变");
}

#[test]
fn f3_game_update_survives_reopen() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("db3.db");
    let new_save = tmp.child("new-save");

    {
        let repo = SqliteRepo::open(&db_path).unwrap();
        let game = Game {
            id: "g_edit".into(),
            name: "旧名称".into(),
            icon: None,
            repo_path: tmp.path().join("repo"),
            save_paths: vec![tmp.child("old-save")],
            created_at: "2026-06-23 00:00".into(),
            updated_at: "2026-06-23 00:00".into(),
        };
        repo.insert_game(game.clone()).unwrap();

        let mut edited = game;
        edited.name = "新名称".into();
        edited.save_paths = vec![new_save.clone()];
        edited.updated_at = "2026-06-23 00:01".into();
        repo.update_game(edited).unwrap();
    }

    {
        let repo = SqliteRepo::open(&db_path).unwrap();
        let got = repo.get_game("g_edit").unwrap().unwrap();
        assert_eq!(got.name, "新名称", "F3: 游戏名称修改应持久化");
        assert_eq!(got.save_paths, vec![PathBuf::from(new_save)], "F3: 存档目录修改应持久化");
        assert_eq!(got.updated_at, "2026-06-23 00:01", "F3: 更新时间应持久化");
    }
}
