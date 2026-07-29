//! SaveLink 核心领域类型。
//!
//! 字段与 `doc/SaveLink技术架构.md` 的数据模型、以及前端原型 mock 一致。

use std::path::PathBuf;

/// 快照创建原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// 用户手动创建。
    Manual,
    /// 旧版本创建的恢复前备份。当前版本不再主动产生，保留用于兼容历史数据。
    BeforeRestore,
    /// 阶段 2 自动快照（MVP 暂不产生）。
    Auto,
}

/// 快照物理写入状态。
///
/// `Writing` 是中断防护的关键：记录先以 `Writing` 落库，
/// 文件写完并校验通过后才置 `Complete`。启动自检清理残留的 `Writing`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotStatus {
    Writing,
    Complete,
    Corrupt,
}

/// 游戏（用户管理存档的基本单位）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    /// 该游戏的快照仓库根目录。
    pub repo_path: PathBuf,
    /// 真实存档目录（MVP 先支持一个，结构上允许多个）。
    pub save_paths: Vec<PathBuf>,
    pub created_at: String,
    pub updated_at: String,
}

/// 快照元数据。内容不可变，仅 `note` / `locked` 可改（安全规则 3）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub id: String,
    pub game_id: String,
    pub created_at: String,
    pub note: Option<String>,
    pub reason: Reason,
    pub locked: bool,
    pub file_count: u64,
    pub total_size: u64,
    /// 整快照内容指纹，支撑「存档未变化」判断。
    pub content_hash: String,
    /// 不透明存储键：上层不得解析其结构（解耦未来 ResticStore）。
    pub storage_key: String,
    pub status: SnapshotStatus,
}

/// 一次扫描的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanResult {
    pub file_count: u64,
    pub total_size: u64,
    pub content_hash: String,
    pub readable: bool,
}

/// SnapshotStore 写入成功后返回的物理信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSnapshot {
    pub storage_key: String,
    pub file_count: u64,
    pub total_size: u64,
}

/// 创建快照的结果：可能是新建，也可能是「存档未变化」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateOutcome {
    Created(Snapshot),
    /// 与上一快照内容一致，未创建（对应原型「存档未变化」）。
    NoChange,
}

/// 恢复成功结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreOutcome {
    pub target_id: String,
    /// false 表示当前真实存档已经等于目标版本，本次没有执行覆盖。
    pub restored: bool,
}

/// 真实存档目录不存在时，需要用户决策（安全规则 5）。
/// 恢复流程在用户未确认前不得写入。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingDirChoice {
    CreateAndRestore,
    Reselect,
    Cancel,
}

/// 恢复进度步骤。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreStep {
    RestoreTarget,
    Verify,
}
