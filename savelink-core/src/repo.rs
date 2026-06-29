//! 元数据持久化抽象 + 可注入的时钟/ID 生成。
//!
//! - `Repository` trait：生产用 `SqliteRepo`（实现者后续加），测试用 `InMemoryRepo`。
//! - `InMemoryRepo` 是**已实现**的（测试需要它来断言时间线状态）。
//! - `Clock` / `IdGen`：可注入，让 created_at / id 可重复，断言稳定。

use crate::error::Result;
use crate::model::{Game, Snapshot};
use std::sync::Mutex;

/// 元数据存取。仅定义当前测试范围所需的方法。
pub trait Repository: Send + Sync {
    fn insert_game(&self, game: Game) -> Result<()>;
    fn get_game(&self, game_id: &str) -> Result<Option<Game>>;
    fn list_games(&self) -> Result<Vec<Game>>;
    fn update_game(&self, game: Game) -> Result<()>;

    fn insert_snapshot(&self, snap: Snapshot) -> Result<()>;
    fn get_snapshot(&self, snapshot_id: &str) -> Result<Option<Snapshot>>;
    /// 时间线：按 created_at 倒序（新在前）。
    fn list_snapshots(&self, game_id: &str) -> Result<Vec<Snapshot>>;
    fn update_snapshot(&self, snap: Snapshot) -> Result<()>;
    fn delete_snapshot(&self, snapshot_id: &str) -> Result<()>;

    /// 启动自检：返回所有 status==Writing 的残留快照（上次中断的半成品）。
    fn list_writing(&self) -> Result<Vec<Snapshot>>;
}

/// 可注入时钟，便于断言 created_at。
pub trait Clock: Send + Sync {
    fn now_stamp(&self) -> String;
}

/// 可注入 ID 生成。
pub trait IdGen: Send + Sync {
    fn new_id(&self, prefix: &str) -> String;
}

/// 测试用：单调递增的假时钟（每次调用 +1 分钟），输出 "2026-06-23 00:00" 形式。
pub struct FakeClock {
    minute: Mutex<u32>,
}
impl FakeClock {
    pub fn new() -> Self {
        Self { minute: Mutex::new(0) }
    }
}
impl Default for FakeClock {
    fn default() -> Self {
        Self::new()
    }
}
impl Clock for FakeClock {
    fn now_stamp(&self) -> String {
        let mut m = self.minute.lock().unwrap();
        let cur = *m;
        *m += 1;
        let hh = cur / 60;
        let mm = cur % 60;
        format!("2026-06-23 {hh:02}:{mm:02}")
    }
}

/// 测试用：递增计数 ID，可预测。
pub struct SeqIdGen {
    n: Mutex<u64>,
}
impl SeqIdGen {
    pub fn new() -> Self {
        Self { n: Mutex::new(0) }
    }
}
impl Default for SeqIdGen {
    fn default() -> Self {
        Self::new()
    }
}
impl IdGen for SeqIdGen {
    fn new_id(&self, prefix: &str) -> String {
        let mut n = self.n.lock().unwrap();
        *n += 1;
        format!("{prefix}_{:04}", *n)
    }
}

/// 已实现的内存仓库，供测试断言时间线状态。
pub struct InMemoryRepo {
    games: Mutex<Vec<Game>>,
    snaps: Mutex<Vec<Snapshot>>,
}

impl InMemoryRepo {
    pub fn new() -> Self {
        Self { games: Mutex::new(Vec::new()), snaps: Mutex::new(Vec::new()) }
    }
}
impl Default for InMemoryRepo {
    fn default() -> Self {
        Self::new()
    }
}

impl Repository for InMemoryRepo {
    fn insert_game(&self, game: Game) -> Result<()> {
        self.games.lock().unwrap().push(game);
        Ok(())
    }
    fn get_game(&self, game_id: &str) -> Result<Option<Game>> {
        Ok(self.games.lock().unwrap().iter().find(|g| g.id == game_id).cloned())
    }
    fn list_games(&self) -> Result<Vec<Game>> {
        Ok(self.games.lock().unwrap().clone())
    }
    fn update_game(&self, game: Game) -> Result<()> {
        let mut games = self.games.lock().unwrap();
        if let Some(slot) = games.iter_mut().find(|g| g.id == game.id) {
            *slot = game;
        }
        Ok(())
    }
    fn insert_snapshot(&self, snap: Snapshot) -> Result<()> {
        self.snaps.lock().unwrap().push(snap);
        Ok(())
    }
    fn get_snapshot(&self, snapshot_id: &str) -> Result<Option<Snapshot>> {
        Ok(self.snaps.lock().unwrap().iter().find(|s| s.id == snapshot_id).cloned())
    }
    fn list_snapshots(&self, game_id: &str) -> Result<Vec<Snapshot>> {
        let mut v: Vec<Snapshot> = self
            .snaps
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.game_id == game_id)
            .cloned()
            .collect();
        // created_at 倒序（FakeClock 单调递增，字符串序即时间序）。
        v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(v)
    }
    fn update_snapshot(&self, snap: Snapshot) -> Result<()> {
        let mut g = self.snaps.lock().unwrap();
        if let Some(slot) = g.iter_mut().find(|s| s.id == snap.id) {
            *slot = snap;
        }
        Ok(())
    }
    fn delete_snapshot(&self, snapshot_id: &str) -> Result<()> {
        self.snaps.lock().unwrap().retain(|s| s.id != snapshot_id);
        Ok(())
    }
    fn list_writing(&self) -> Result<Vec<Snapshot>> {
        use crate::model::SnapshotStatus;
        Ok(self
            .snaps
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.status == SnapshotStatus::Writing)
            .cloned()
            .collect())
    }
}
