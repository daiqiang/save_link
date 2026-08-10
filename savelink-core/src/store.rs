//! 存储层抽象。架构成败手：上层只认 `storage_key`，不关心底层是 zip 还是去重仓库。
//!
//! - MVP 实现：`FsStore`（零依赖的目录复制存储，作为 ZipStore 的等价替身）。
//!   实现者可直接把它换成真正的 zip 实现，trait 不变。
//! - 未来：`ResticStore`，`storage_key` 存 restic snapshot id。
//!
//! 当前 `FsStore` 的方法体是 `todo!()` —— 红灯。实现它以让 D 组用例转绿。

use crate::error::{Result, SaveLinkError};
use crate::model::{ScanResult, StoredSnapshot};
use crate::scan;
use std::fs;
use std::path::{Path, PathBuf};

/// 快照文件的存与取。
pub trait SnapshotStore: Send + Sync {
    /// 把若干源目录打包成一个快照，返回 storage_key 与统计信息。
    ///
    /// 约束：写入必须可识别中断（半成品不得被当作完好快照）。
    fn create(&self, snapshot_id: &str, sources: &[PathBuf], ctx: &ScanResult)
        -> Result<StoredSnapshot>;

    /// 把指定快照解出到目标目录（恢复用）。必须是覆盖式、内容无损。
    fn restore(&self, key: &str, target: &Path) -> Result<()>;

    /// 把快照中的各个独立存档根恢复到对应目标目录。
    /// 单目录实现默认退化为原有 `restore`，保持测试替身与旧存储实现兼容。
    fn restore_sources(&self, key: &str, targets: &[PathBuf]) -> Result<()> {
        let target = targets
            .first()
            .ok_or_else(|| SaveLinkError::Io("restore has no target".into()))?;
        if targets.len() != 1 {
            return Err(SaveLinkError::Io("snapshot store does not support multiple sources".into()));
        }
        self.restore(key, target)
    }

    /// 校验快照完整性（恢复前 / 创建后调用）。
    fn verify(&self, key: &str) -> Result<bool>;

    /// 删除快照物理文件。
    fn delete(&self, key: &str) -> Result<()>;
}

/// 零依赖存储实现：把每个快照存为仓库下的一个目录树副本。
///
/// 它是 ZipStore 的行为等价替身——同一套 trait 语义，便于先把服务层逻辑
/// 与测试跑通；实现者可无缝替换为 zip。
///
/// 仓库布局（与架构文档一致，zip 换成目录即可）：
/// `{repo}/snapshots/{snapshot_id}/...`  +  `{repo}/snapshots/{snapshot_id}.ok`（完成标记）
pub struct FsStore {
    repo_root: PathBuf,
}

impl FsStore {
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self { repo_root: repo_root.into() }
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    fn snap_dir(&self, key: &str) -> PathBuf {
        self.repo_root.join("snapshots").join(key)
    }

    /// 完成标记文件，内容为 content_hash。存在即表示该快照写入完整。
    fn ok_marker(&self, key: &str) -> PathBuf {
        self.repo_root.join("snapshots").join(format!("{key}.ok"))
    }

    /// 多目录布局标记，内容为根目录数量。旧单目录快照没有该文件。
    fn layout_marker(&self, key: &str) -> PathBuf {
        self.repo_root
            .join("snapshots")
            .join(format!("{key}.layout"))
    }

    fn stored_source_count(&self, key: &str) -> Result<u32> {
        let marker = self.layout_marker(key);
        if !marker.exists() {
            return Ok(1);
        }
        let value = fs::read_to_string(marker).map_err(|e| SaveLinkError::Io(e.to_string()))?;
        let count = value
            .trim()
            .parse::<u32>()
            .map_err(|_| SaveLinkError::SnapshotCorrupt)?;
        if count < 2 {
            return Err(SaveLinkError::SnapshotCorrupt);
        }
        Ok(count)
    }
}

/// 递归把 `src` 目录下的内容复制进 `dst`（不含 src 本身这一层）。
fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).map_err(|e| SaveLinkError::Io(e.to_string()))?;
    for entry in fs::read_dir(src).map_err(|e| SaveLinkError::Io(e.to_string()))? {
        let entry = entry.map_err(|e| SaveLinkError::Io(e.to_string()))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).map_err(|e| SaveLinkError::Io(e.to_string()))?;
            }
            fs::copy(&from, &to).map_err(|e| SaveLinkError::Io(e.to_string()))?;
        }
    }
    Ok(())
}

impl SnapshotStore for FsStore {
    fn create(
        &self,
        snapshot_id: &str,
        sources: &[PathBuf],
        ctx: &ScanResult,
    ) -> Result<StoredSnapshot> {
        let dir = self.snap_dir(snapshot_id);
        // 重入清理：若有同名残留先清掉，保证幂等。
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_file(self.ok_marker(snapshot_id));
        let _ = fs::remove_file(self.layout_marker(snapshot_id));
        fs::create_dir_all(&dir).map_err(|e| SaveLinkError::Io(e.to_string()))?;

        if sources.len() > 1 {
            for (index, source) in sources.iter().enumerate() {
                copy_tree(source, &dir.join("sources").join(index.to_string()))?;
            }
            fs::write(self.layout_marker(snapshot_id), sources.len().to_string())
                .map_err(|e| SaveLinkError::Io(e.to_string()))?;
        } else if let Some(source) = sources.first() {
            copy_tree(source, &dir)?;
        }

        // 全部内容写完后，最后写 .ok 标记（含指纹）。中断时不会有标记 → verify 为 false。
        fs::write(self.ok_marker(snapshot_id), ctx.content_hash.as_bytes())
            .map_err(|e| SaveLinkError::Io(e.to_string()))?;

        Ok(StoredSnapshot {
            storage_key: snapshot_id.to_string(),
            file_count: ctx.file_count,
            total_size: ctx.total_size,
        })
    }

    fn restore(&self, key: &str, target: &Path) -> Result<()> {
        let dir = self.snap_dir(key);
        if !dir.exists() {
            return Err(SaveLinkError::SnapshotCorrupt);
        }
        copy_tree(&dir, target)
    }

    fn restore_sources(&self, key: &str, targets: &[PathBuf]) -> Result<()> {
        let dir = self.snap_dir(key);
        if !dir.exists() {
            return Err(SaveLinkError::SnapshotCorrupt);
        }
        let source_count = self.stored_source_count(key)? as usize;
        if targets.len() != source_count {
            return Err(SaveLinkError::SnapshotCorrupt);
        }
        if source_count == 1 {
            return copy_tree(&dir, &targets[0]);
        }
        for (index, target) in targets.iter().enumerate() {
            let source = dir.join("sources").join(index.to_string());
            if !source.is_dir() {
                return Err(SaveLinkError::SnapshotCorrupt);
            }
            copy_tree(&source, target)?;
        }
        Ok(())
    }

    fn verify(&self, key: &str) -> Result<bool> {
        let dir = self.snap_dir(key);
        let marker = self.ok_marker(key);
        if !dir.exists() || !marker.exists() {
            return Ok(false);
        }
        let expected = fs::read_to_string(&marker).map_err(|e| SaveLinkError::Io(e.to_string()))?;
        let source_count = self.stored_source_count(key)?;
        let actual = scan::fingerprint_snapshot_payload(&dir, source_count)?.content_hash;
        Ok(actual == expected.trim())
    }

    fn delete(&self, key: &str) -> Result<()> {
        let dir = self.snap_dir(key);
        if dir.exists() {
            fs::remove_dir_all(&dir).map_err(|e| SaveLinkError::Io(e.to_string()))?;
        }
        let marker = self.ok_marker(key);
        if marker.exists() {
            fs::remove_file(&marker).map_err(|e| SaveLinkError::Io(e.to_string()))?;
        }
        let layout = self.layout_marker(key);
        if layout.exists() {
            fs::remove_file(&layout).map_err(|e| SaveLinkError::Io(e.to_string()))?;
        }
        Ok(())
    }
}
