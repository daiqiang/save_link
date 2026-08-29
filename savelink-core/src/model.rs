//! SaveLink 核心领域类型。
//!
//! 字段与 `doc/SaveLink技术架构.md` 的数据模型、以及前端原型 mock 一致。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 快照内的逻辑文件与本机真实文件之间的映射。
///
/// 模拟器通常让多个游戏共用一个存档目录，并按 ROM 文件名区分存档。快照使用
/// `snapshot_relative_path` 作为稳定名称，本机恢复时再写入 `local_relative_path`，
/// 因而 ROM 改名不会让跨设备恢复失效。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveFileMapping {
    pub local_relative_path: PathBuf,
    pub snapshot_relative_path: PathBuf,
}

/// 一个游戏需要保护的本机来源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SaveSource {
    /// 传统游戏：保护整个目录，保持 v0.1-v0.3 行为不变。
    Directory { path: PathBuf },
    /// 模拟器游戏：只保护共享目录中明确列出的文件。
    Files {
        root: PathBuf,
        files: Vec<SaveFileMapping>,
    },
}

impl SaveSource {
    pub fn root(&self) -> &Path {
        match self {
            Self::Directory { path } => path,
            Self::Files { root, .. } => root,
        }
    }

    pub fn is_directory(&self) -> bool {
        matches!(self, Self::Directory { .. })
    }
}

/// ROM 的稳定身份。这里只保存可计算的元数据，不保存或上传 ROM 内容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RomIdentity {
    pub file_name: String,
    pub sha256: String,
    pub header_title: String,
    pub game_code: String,
}

/// 可跨设备同步的模拟器游戏身份。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmulatorGameIdentity {
    pub emulator: String,
    pub rom: RomIdentity,
}

/// 当前设备上的模拟器绑定。路径仅保存在本机，不进入云协议。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmulatorLocalBinding {
    pub emulator_root: PathBuf,
    pub rom_path: PathBuf,
    pub rom_size: u64,
    pub rom_modified_unix_ms: Option<u64>,
    pub local_rom: RomIdentity,
}

/// 当前设备上的普通 PC 游戏启动绑定。路径和启动参数仅保存在本机，不进入云协议。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameLaunchBinding {
    pub executable_path: PathBuf,
    pub install_dir: PathBuf,
    #[serde(default)]
    pub launch_arguments: Vec<String>,
    #[serde(default)]
    pub steam_app_id: Option<u32>,
}

impl GameLaunchBinding {
    pub fn executable(executable_path: PathBuf, install_dir: PathBuf) -> Self {
        Self {
            executable_path,
            install_dir,
            launch_arguments: Vec::new(),
            steam_app_id: None,
        }
    }

    pub fn steam(steam_executable_path: PathBuf, install_dir: PathBuf, app_id: u32) -> Self {
        Self {
            executable_path: steam_executable_path,
            install_dir,
            launch_arguments: vec!["-applaunch".into(), app_id.to_string()],
            steam_app_id: Some(app_id),
        }
    }
}

/// 游戏在当前设备上的存档配置状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameConfigurationState {
    Configured,
    PendingDiscovery,
    PendingBinding,
}

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
    /// 云端已安全清理，正在删除本机物理快照；启动自检会续做未完成删除。
    Deleting,
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
    /// 非空时覆盖 `save_paths` 的整目录语义，用于共享目录中的精确文件保护。
    pub save_sources: Vec<SaveSource>,
    /// 跨设备同步的模拟器/ROM 身份，不包含本机绝对路径。
    pub emulator_identity: Option<EmulatorGameIdentity>,
    /// 当前设备的 ROM 绑定；云端下载后在用户完成本机绑定前为空。
    pub emulator_binding: Option<EmulatorLocalBinding>,
    /// 当前设备的普通 PC 游戏启动绑定；用于启动游戏和动态发现存档。
    pub launch_binding: Option<GameLaunchBinding>,
    pub created_at: String,
    pub updated_at: String,
}

impl Game {
    /// 返回当前设备真正参与扫描、快照和恢复的来源。
    pub fn effective_save_sources(&self) -> Vec<SaveSource> {
        if !self.save_sources.is_empty() {
            return self.save_sources.clone();
        }
        self.save_paths
            .iter()
            .cloned()
            .map(|path| SaveSource::Directory { path })
            .collect()
    }

    pub fn configuration_state(&self) -> GameConfigurationState {
        if !self.effective_save_sources().is_empty() {
            GameConfigurationState::Configured
        } else if self.launch_binding.is_some() {
            GameConfigurationState::PendingDiscovery
        } else {
            GameConfigurationState::PendingBinding
        }
    }

    pub fn is_configured(&self) -> bool {
        self.configuration_state() == GameConfigurationState::Configured
    }
}

/// 快照元数据。内容不可变，仅 `note` / `locked` 可改（安全规则 3）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub id: String,
    pub game_id: String,
    pub created_at: String,
    pub note: Option<String>,
    /// 用户可变名称最后修改时间；自动展示名称不写入该字段。
    pub note_updated_at: String,
    pub reason: Reason,
    /// 是否受到自动清理保护。该字段是安全语义，也会同步到云端。
    pub locked: bool,
    /// 锁定状态最后修改时间，用于跨设备按字段合并。
    pub locked_updated_at: String,
    /// 当前本机时间线所属的显示区域。与 `locked` 分离，支持十分钟维护周期内的待整理状态。
    pub display_zone: SnapshotDisplayZone,
    pub file_count: u64,
    pub total_size: u64,
    /// 快照包含的独立存档根目录数量。旧快照固定为 1。
    pub source_count: u32,
    /// 整快照内容指纹，支撑「存档未变化」判断。
    pub content_hash: String,
    /// 不透明存储键：上层不得解析其结构（解耦未来 ResticStore）。
    pub storage_key: String,
    pub status: SnapshotStatus,
}

/// 快照在本机时间线中的显示区域。
///
/// 这是本机展示元数据，不进入云端协议。锁定/解锁时只立即改变 `Snapshot::locked`，
/// 下一次维护周期再把快照移动到与保护状态对应的区域。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotDisplayZone {
    #[default]
    Normal,
    Locked,
}

impl SnapshotDisplayZone {
    pub fn for_locked(locked: bool) -> Self {
        if locked {
            Self::Locked
        } else {
            Self::Normal
        }
    }

    pub fn is_pending(self, locked: bool) -> bool {
        self != Self::for_locked(locked)
    }
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
#[allow(clippy::large_enum_variant)]
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
