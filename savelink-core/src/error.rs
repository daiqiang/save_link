//! 统一错误类型。前端按 kind 渲染对应状态（对应视觉文档「状态设计」清单）。

use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveLinkError {
    /// 存档目录不存在。
    SaveDirMissing,
    /// 存档目录无法访问（权限/IO）。
    SaveDirUnreadable,
    /// 恢复失败。`rolled_back` 告诉前端真实存档是否已被安全回滚，
    /// 直接对应视觉文档「恢复失败必须说清是否已开始覆盖」。
    RestoreFailed { rolled_back: bool },
    /// 快照校验不通过（损坏）。
    SnapshotCorrupt,
    /// 锁定快照不可删（安全规则 4）。
    SnapshotLocked,
    /// 真实存档目录不存在，需用户决策（安全规则 5）。
    SaveDirMissingNeedsChoice,
    /// 一个游戏的多个存档根目录相同或相互嵌套，无法安全地作为独立来源处理。
    OverlappingSavePaths { first: PathBuf, second: PathBuf },
    /// 其它 IO 错误。
    Io(String),
}

impl fmt::Display for SaveLinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveLinkError::SaveDirMissing => write!(f, "存档目录不存在"),
            SaveLinkError::SaveDirUnreadable => write!(f, "无法访问该存档目录"),
            SaveLinkError::RestoreFailed { rolled_back } => {
                write!(f, "恢复失败（已回滚: {rolled_back}）")
            }
            SaveLinkError::SnapshotCorrupt => write!(f, "快照已损坏"),
            SaveLinkError::SnapshotLocked => write!(f, "锁定快照不能删除，请先取消锁定"),
            SaveLinkError::SaveDirMissingNeedsChoice => {
                write!(f, "存档目录不存在，请选择如何处理")
            }
            SaveLinkError::OverlappingSavePaths { first, second } => write!(
                f,
                "存档目录不能相同或相互嵌套：{} 与 {}",
                first.display(),
                second.display()
            ),
            SaveLinkError::Io(msg) => write!(f, "IO 错误: {msg}"),
        }
    }
}

impl std::error::Error for SaveLinkError {}

pub type Result<T> = std::result::Result<T, SaveLinkError>;
