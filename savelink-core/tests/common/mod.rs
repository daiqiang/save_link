//! 测试共用：组装服务、构造游戏、便捷断言。
//! 各测试文件 `mod common;` 引入。

#![allow(dead_code)]

use savelink_core::model::{Game, Reason, Snapshot, SnapshotStatus};
use savelink_core::repo::{Clock, FakeClock, IdGen, Repository, SeqIdGen};
use savelink_core::service::{RestoreService, SnapshotService};
use savelink_core::store::{FsStore, SnapshotStore};
use savelink_core::testkit::{write_files, TempDir};
use std::path::PathBuf;
use std::sync::Arc;

/// 一套完整的被测装配：临时目录 + 存档目录 + 仓库 + 各服务。
pub struct Harness {
    pub tmp: TempDir,
    pub save_dir: PathBuf,
    pub repo: Arc<dyn Repository>,
    pub store: Arc<dyn SnapshotStore>,
    pub clock: Arc<dyn Clock>,
    pub ids: Arc<dyn IdGen>,
    pub game_id: String,
}

impl Harness {
    /// 构造：存档目录写入给定文件，注册一个游戏。store 默认是真实 FsStore。
    pub fn new(files: &[(&str, &[u8])]) -> Self {
        Self::with_store(files, |repo_root| Arc::new(FsStore::new(repo_root)))
    }

    /// 自定义 store（用于注入 FailingStore 包装）。
    pub fn with_store(
        files: &[(&str, &[u8])],
        make_store: impl FnOnce(PathBuf) -> Arc<dyn SnapshotStore>,
    ) -> Self {
        let tmp = TempDir::new();
        let save_dir = tmp.child("save");
        let repo_root = tmp.child("repo");
        write_files(&save_dir, files);

        // 用真 SQLite（内存模式）跑全部测试：证明换数据库后端，
        // 上层逻辑与这 31 个用例一行不改即可通过。
        let repo: Arc<dyn Repository> =
            Arc::new(savelink_core::sqlite_repo::SqliteRepo::open_in_memory().unwrap());
        let store = make_store(repo_root);
        Self::assemble(tmp, save_dir, repo, store)
    }

    /// 调用方提供 repo（用于 F 组持久化测试：传入基于文件的 SqliteRepo）。
    pub fn with_repo(files: &[(&str, &[u8])], repo: Arc<dyn Repository>) -> Self {
        let tmp = TempDir::new();
        let save_dir = tmp.child("save");
        let repo_root = tmp.child("repo");
        write_files(&save_dir, files);
        let store: Arc<dyn SnapshotStore> = Arc::new(FsStore::new(repo_root));
        Self::assemble(tmp, save_dir, repo, store)
    }

    fn assemble(
        tmp: TempDir,
        save_dir: PathBuf,
        repo: Arc<dyn Repository>,
        store: Arc<dyn SnapshotStore>,
    ) -> Self {
        let clock: Arc<dyn Clock> = Arc::new(FakeClock::new());
        let ids: Arc<dyn IdGen> = Arc::new(SeqIdGen::new());

        let game = Game {
            id: "g_test".into(),
            name: "测试游戏".into(),
            icon: None,
            repo_path: tmp.path().join("repo"),
            save_paths: vec![save_dir.clone()],
            save_sources: Vec::new(),
            emulator_identity: None,
            emulator_binding: None,
            created_at: clock.now_stamp(),
            updated_at: clock.now_stamp(),
        };
        repo.insert_game(game).unwrap();

        Self {
            tmp,
            save_dir,
            repo,
            store,
            clock,
            ids,
            game_id: "g_test".into(),
        }
    }

    pub fn snapshots(&self) -> SnapshotService {
        SnapshotService::new(
            self.repo.clone(),
            self.store.clone(),
            self.clock.clone(),
            self.ids.clone(),
        )
    }

    pub fn restore(&self) -> RestoreService {
        RestoreService::new(
            self.repo.clone(),
            self.store.clone(),
            self.clock.clone(),
            self.ids.clone(),
        )
    }

    /// 返回一个 RestoreService，其 store 在原 store 外再包一层故障注入。
    /// 之前已创建的快照仍可访问（共享同一底层 store / 仓库目录）。
    /// `nth==0` 表示每次命中该 op 都失败。
    pub fn restore_failing(
        &self,
        op: savelink_core::testkit::FailOp,
        kind: savelink_core::testkit::FailKind,
        nth: usize,
    ) -> RestoreService {
        use savelink_core::testkit::FailingStore;
        let wrapped: Arc<dyn SnapshotStore> = if nth == 0 {
            Arc::new(FailingStore::new(self.store.clone()).fail(op, kind))
        } else {
            Arc::new(FailingStore::new(self.store.clone()).fail_on_call(op, kind, nth))
        };
        RestoreService::new(
            self.repo.clone(),
            wrapped,
            self.clock.clone(),
            self.ids.clone(),
        )
    }

    pub fn timeline(&self) -> Vec<Snapshot> {
        self.repo.list_snapshots(&self.game_id).unwrap()
    }

    /// 改写存档目录内容（用于"恢复后内容应与目标一致/不同"等场景）。
    pub fn set_save_dir(&self, files: &[(&str, &[u8])]) {
        let _ = std::fs::remove_dir_all(&self.save_dir);
        std::fs::create_dir_all(&self.save_dir).unwrap();
        write_files(&self.save_dir, files);
    }
}

/// 无操作进度回调。
pub fn no_progress() -> impl Fn(savelink_core::model::RestoreStep) + Send + Sync {
    |_step| {}
}

pub fn count_reason(snaps: &[Snapshot], reason: Reason) -> usize {
    snaps.iter().filter(|s| s.reason == reason).count()
}

pub fn any_writing(snaps: &[Snapshot]) -> bool {
    snaps.iter().any(|s| s.status == SnapshotStatus::Writing)
}
