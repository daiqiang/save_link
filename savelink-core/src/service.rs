//! 已实现的业务服务层。
//!
//! 服务不感知 Tauri。依赖通过构造注入：Repository + SnapshotStore + Clock + IdGen。
//! 每个方法的契约注释直接对应 `savelink-restore-test-spec.md` 的用例编号。

use crate::error::{Result, SaveLinkError};
use crate::model::{CreateOutcome, RestoreOutcome, RestoreStep};
use crate::model::{Snapshot, SnapshotStatus};
use crate::repo::{Clock, IdGen, Repository};
use crate::scan;
use crate::store::SnapshotStore;
use std::fs;
use std::path::Path;
use std::sync::Arc;

/// 进度回调（恢复/创建的步骤事件）。生产接 Tauri emit，测试用闭包收集。
pub type ProgressSink<'a> = dyn Fn(RestoreStep) + Send + Sync + 'a;

pub struct SnapshotService {
    pub repo: Arc<dyn Repository>,
    pub store: Arc<dyn SnapshotStore>,
    pub clock: Arc<dyn Clock>,
    pub ids: Arc<dyn IdGen>,
}

impl SnapshotService {
    pub fn new(
        repo: Arc<dyn Repository>,
        store: Arc<dyn SnapshotStore>,
        clock: Arc<dyn Clock>,
        ids: Arc<dyn IdGen>,
    ) -> Self {
        Self { repo, store, clock, ids }
    }

    /// 创建快照。
    ///
    /// 契约（对应 A 组）：
    /// - A1：成功时返回 Created，file_count/total_size/内容指纹正确，reason=Manual。
    /// - A2：扫描 content_hash 与上一快照相同 → 返回 NoChange，不写库不写文件。
    /// - A3：任一文件变化即视为有变化。
    /// - A4：先以 status=Writing 落库 → store.create → verify → 置 Complete。
    ///       任一步失败：删半成品文件 + 删记录，不留 Writing 悬挂；不碰真实存档。
    /// - A5：空目录允许创建（file_count=0）。
    /// - A6：目录不可读 → SaveDirMissing/SaveDirUnreadable，不写任何状态。
    pub fn create_snapshot(
        &self,
        game_id: &str,
        note: Option<String>,
        reason: crate::model::Reason,
    ) -> Result<CreateOutcome> {
        self.create_internal(game_id, note, reason)
    }

    fn create_internal(
        &self,
        game_id: &str,
        note: Option<String>,
        reason: crate::model::Reason,
    ) -> Result<CreateOutcome> {
        let game = self
            .repo
            .get_game(game_id)?
            .ok_or_else(|| SaveLinkError::Io(format!("game not found: {game_id}")))?;

        // A6：扫描，目录不存在/不可读直接返回错误，不写任何状态。
        let ctx = scan::scan(&game.save_paths)?;

        // A2/A3：与最近一个快照内容比对，一致则不创建。
        let latest = self.repo.list_snapshots(game_id)?.into_iter().next();
        if let Some(prev) = &latest {
            if prev.content_hash == ctx.content_hash {
                return Ok(CreateOutcome::NoChange);
            }
        }

        // A4：先以 Writing 落库，再写文件；任一步失败则回滚记录 + 清半成品。
        let id = self.ids.new_id("snap");
        let pending = Snapshot {
            id: id.clone(),
            game_id: game_id.to_string(),
            created_at: self.clock.now_stamp(),
            note,
            reason,
            locked: false,
            file_count: ctx.file_count,
            total_size: ctx.total_size,
            content_hash: ctx.content_hash.clone(),
            storage_key: id.clone(),
            status: SnapshotStatus::Writing,
        };
        self.repo.insert_snapshot(pending.clone())?;

        let stored = match self.store.create(&id, &game.save_paths, &ctx) {
            Ok(s) => s,
            Err(e) => {
                let _ = self.store.delete(&id); // 清理可能的半成品
                let _ = self.repo.delete_snapshot(&id); // 回滚记录
                return Err(e);
            }
        };

        // 写入后校验；不通过同样回滚。
        match self.store.verify(&id) {
            Ok(true) => {}
            Ok(false) => {
                let _ = self.store.delete(&id);
                let _ = self.repo.delete_snapshot(&id);
                return Err(SaveLinkError::SnapshotCorrupt);
            }
            Err(e) => {
                let _ = self.store.delete(&id);
                let _ = self.repo.delete_snapshot(&id);
                return Err(e);
            }
        }

        let complete = Snapshot {
            file_count: stored.file_count,
            total_size: stored.total_size,
            storage_key: stored.storage_key,
            status: SnapshotStatus::Complete,
            ..pending
        };
        self.repo.update_snapshot(complete.clone())?;
        Ok(CreateOutcome::Created(complete))
    }

    /// 修改快照元数据（仅 note / locked 可变，安全规则 3 / C4）。
    pub fn update_meta(
        &self,
        snapshot_id: &str,
        note: Option<String>,
        locked: Option<bool>,
    ) -> Result<()> {
        let mut snap = self
            .repo
            .get_snapshot(snapshot_id)?
            .ok_or_else(|| SaveLinkError::Io(format!("snapshot not found: {snapshot_id}")))?;
        if let Some(n) = note {
            snap.note = Some(n);
        }
        if let Some(l) = locked {
            snap.locked = l;
        }
        // content_hash / created_at / file_count / total_size 一律不动。
        self.repo.update_snapshot(snap)
    }

    /// 删除快照。
    ///
    /// 契约（C 组）：
    /// - C1：locked → SnapshotLocked，记录与文件均保留。
    /// - C3：先删文件再删记录；删文件失败则回滚，不留悬挂（无"有记录无文件"）。
    pub fn delete_snapshot(&self, snapshot_id: &str) -> Result<()> {
        let snap = self
            .repo
            .get_snapshot(snapshot_id)?
            .ok_or_else(|| SaveLinkError::Io(format!("snapshot not found: {snapshot_id}")))?;
        if snap.locked {
            return Err(SaveLinkError::SnapshotLocked); // C1
        }
        // C3：先删文件，失败则不动记录（避免有记录无文件的反向悬挂）。
        self.store.delete(&snap.storage_key)?;
        self.repo.delete_snapshot(snapshot_id)?;
        Ok(())
    }

    /// 删除游戏及其在 SaveLink 仓库中的全部快照。
    ///
    /// 不删除真实存档目录。每个快照都先删物理文件，再删对应记录；
    /// 任一物理删除失败则中止，避免数据库指向已不存在的快照文件。
    pub fn delete_game(&self, game_id: &str) -> Result<()> {
        self.repo
            .get_game(game_id)?
            .ok_or_else(|| SaveLinkError::Io(format!("game not found: {game_id}")))?;
        let snaps = self.repo.list_snapshots(game_id)?;
        for snap in &snaps {
            self.store.delete(&snap.storage_key)?;
            self.repo.delete_snapshot(&snap.id)?;
        }
        self.repo.delete_game(game_id)?;
        Ok(())
    }
}

pub struct RestoreService {
    pub repo: Arc<dyn Repository>,
    pub store: Arc<dyn SnapshotStore>,
    clock: Arc<dyn Clock>,
}

impl RestoreService {
    pub fn new(
        repo: Arc<dyn Repository>,
        store: Arc<dyn SnapshotStore>,
        clock: Arc<dyn Clock>,
        _ids: Arc<dyn IdGen>,
    ) -> Self {
        Self { repo, store, clock }
    }

    /// 恢复快照——产品生命线（对应 B 组全部用例）。
    ///
    /// 必守顺序与契约：
    /// 1. 前置：目标快照存在且 `store.verify` 通过；否则 SnapshotCorrupt，不动真实存档（B4）。
    /// 2. 真实存档目录不存在 → 返回 SaveDirMissingNeedsChoice，未确认前不写入（B7）。
    /// 3. 当前已经等于目标时直接返回，不执行覆盖（B11）。
    /// 4. 覆盖恢复使用"旁路 + 同盘 rename 替换"：目标先恢复到临时目录并校验，
    ///    当前目录暂存为 .old，目标目录通过最终校验后才删除 .old（B3/B5）。
    /// 5. 替换或最终校验失败时立即换回 .old，准确返回 RestoreFailed{rolled_back}（B6/B13）。
    /// 6. 当前版本不会自动创建恢复前快照；成功恢复后如需回到原状态，必须事先手动创建快照（B1）。
    pub fn restore_snapshot(
        &self,
        game_id: &str,
        snapshot_id: &str,
        progress: &ProgressSink<'_>,
    ) -> Result<RestoreOutcome> {
        let game = self
            .repo
            .get_game(game_id)?
            .ok_or_else(|| SaveLinkError::Io(format!("game not found: {game_id}")))?;
        let target = self
            .repo
            .get_snapshot(snapshot_id)?
            .ok_or_else(|| SaveLinkError::Io(format!("snapshot not found: {snapshot_id}")))?;
        let save_dir = game
            .save_paths
            .first()
            .ok_or_else(|| SaveLinkError::Io("game has no save path".into()))?
            .clone();

        // 步骤 1（前置）：目标快照必须完好（B4）。
        if !self.store.verify(&target.storage_key)? {
            return Err(SaveLinkError::SnapshotCorrupt);
        }

        // 步骤 2：真实存档目录不存在 → 交还用户决策，未确认前不写入（B7）。
        if !save_dir.exists() {
            return Err(SaveLinkError::SaveDirMissingNeedsChoice);
        }

        self.do_restore(&target, &save_dir, progress)
    }

    /// 真正执行恢复的内部流程。前置检查已在调用方完成。
    fn do_restore(
        &self,
        target: &Snapshot,
        save_dir: &Path,
        progress: &ProgressSink<'_>,
    ) -> Result<RestoreOutcome> {
        let current = scan::fingerprint_dir(save_dir).map_err(|_| SaveLinkError::SaveDirUnreadable)?;

        // 已经是目标版本：不覆盖，也不发出虚假的恢复进度。
        if snapshot_matches_scan(target, &current) {
            return Ok(RestoreOutcome {
                target_id: target.id.clone(),
                restored: false,
            });
        }

        // 覆盖恢复采用"旁路 + 同盘 rename 替换"。
        progress(RestoreStep::RestoreTarget);
        let parent = save_dir
            .parent()
            .ok_or_else(|| SaveLinkError::Io("save dir has no parent".into()))?;
        let stamp = self.clock.now_stamp().replace([' ', ':'], "");
        let tmp_dir = parent.join(format!(".savelink_tmp_{stamp}"));
        let old_dir = parent.join(format!(".savelink_old_{stamp}"));

        // 4a. 解包目标到同盘临时目录 + 校验。失败 → 真实存档未动，直接返回（已回滚=未开始覆盖）。
        let _ = fs::remove_dir_all(&tmp_dir);
        if let Err(e) = self.store.restore(&target.storage_key, &tmp_dir) {
            let _ = fs::remove_dir_all(&tmp_dir);
            return Err(restore_failed(e, true)); // 尚未触碰真实目录 → 视为已回滚
        }
        // 校验临时目录内容与目标快照一致。
        match scan::fingerprint_dir(&tmp_dir) {
            Ok(r)
                if r.content_hash == target.content_hash
                    && r.file_count == target.file_count
                    && r.total_size == target.total_size => {}
            _ => {
                let _ = fs::remove_dir_all(&tmp_dir);
                return Err(SaveLinkError::RestoreFailed { rolled_back: true });
            }
        }

        // 4b. 原子替换：真实目录 → .old，临时目录 → 真实目录。
        if let Err(e) = fs::rename(save_dir, &old_dir) {
            let _ = fs::remove_dir_all(&tmp_dir);
            return Err(restore_failed_io(e, true)); // 真实目录仍完整 → 已回滚
        }
        if let Err(e) = fs::rename(&tmp_dir, save_dir) {
            // 第二个 rename 失败：把 .old 换回去，恢复原状。
            let rolled_back = rollback_old_dir(save_dir, &old_dir, &current);
            let _ = fs::remove_dir_all(&tmp_dir);
            return Err(restore_failed_io(e, rolled_back));
        }
        // 最终校验通过前保留 .old；失败时立即恢复原目录。
        progress(RestoreStep::Verify);
        if !matches!(scan::fingerprint_dir(save_dir), Ok(ref r) if snapshot_matches_scan(target, r)) {
            let rolled_back = rollback_old_dir(save_dir, &old_dir, &current);
            return Err(SaveLinkError::RestoreFailed { rolled_back });
        }

        // 新目录已经确认完整，旧目录到这里才可以清理。
        let _ = fs::remove_dir_all(&old_dir);

        Ok(RestoreOutcome {
            target_id: target.id.clone(),
            restored: true,
        })
    }

    /// 恢复时真实存档目录不存在，用户已做出选择后的续走入口（B7）。
    pub fn restore_with_choice(
        &self,
        game_id: &str,
        snapshot_id: &str,
        choice: crate::model::MissingDirChoice,
        progress: &ProgressSink<'_>,
    ) -> Result<RestoreOutcome> {
        use crate::model::MissingDirChoice;
        let game = self
            .repo
            .get_game(game_id)?
            .ok_or_else(|| SaveLinkError::Io(format!("game not found: {game_id}")))?;
        let target = self
            .repo
            .get_snapshot(snapshot_id)?
            .ok_or_else(|| SaveLinkError::Io(format!("snapshot not found: {snapshot_id}")))?;
        let save_dir = game
            .save_paths
            .first()
            .ok_or_else(|| SaveLinkError::Io("game has no save path".into()))?
            .clone();

        if !self.store.verify(&target.storage_key)? {
            return Err(SaveLinkError::SnapshotCorrupt);
        }

        match choice {
            MissingDirChoice::Cancel | MissingDirChoice::Reselect => {
                // 不写入任何文件，交还上层（B7）。
                Err(SaveLinkError::SaveDirMissingNeedsChoice)
            }
            MissingDirChoice::CreateAndRestore => {
                fs::create_dir_all(&save_dir).map_err(|e| SaveLinkError::Io(e.to_string()))?;
                self.do_restore(&target, &save_dir, progress)
            }
        }
    }
}

fn snapshot_matches_scan(snapshot: &Snapshot, scan: &crate::model::ScanResult) -> bool {
    snapshot.content_hash == scan.content_hash
        && snapshot.file_count == scan.file_count
        && snapshot.total_size == scan.total_size
}

fn rollback_old_dir(save_dir: &Path, old_dir: &Path, expected: &crate::model::ScanResult) -> bool {
    if (save_dir.exists() && fs::remove_dir_all(save_dir).is_err())
        || fs::rename(old_dir, save_dir).is_err()
    {
        return false;
    }
    matches!(scan::fingerprint_dir(save_dir), Ok(ref r) if r == expected)
}

/// 把任意错误归一为 RestoreFailed，携带回滚语义（B6）。
fn restore_failed(_e: SaveLinkError, rolled_back: bool) -> SaveLinkError {
    SaveLinkError::RestoreFailed { rolled_back }
}
fn restore_failed_io(_e: std::io::Error, rolled_back: bool) -> SaveLinkError {
    SaveLinkError::RestoreFailed { rolled_back }
}

/// 启动自检（E 组）。
///
/// 契约：
/// - E1：清理 status==Writing 的残留记录（标记 Corrupt 或删除），并清除对应半成品文件；
///       清理后它们不得出现在正常时间线。
pub fn startup_self_check(
    repo: &Arc<dyn Repository>,
    store: &Arc<dyn SnapshotStore>,
) -> Result<()> {
    // E1：上次创建中崩溃会留下 status==Writing 的悬挂记录。
    // 清掉对应半成品文件，并删除记录，使其不出现在正常时间线。
    for dangling in repo.list_writing()? {
        let _ = store.delete(&dangling.storage_key); // 半成品文件可能不存在，忽略错误
        repo.delete_snapshot(&dangling.id)?;
    }
    Ok(())
}

/// 同盘检测（E2）。判断两路径是否在同一卷——决定能否用原子 rename。
///
/// 跨卷时 RestoreService 必须走安全降级路径。返回 None 表示无法判定，
/// 调用方应保守按"跨卷"处理。
pub fn same_volume(a: &Path, b: &Path) -> Option<bool> {
    // 取各自存在的祖先目录（路径本身可能尚不存在）。
    let ra = existing_ancestor(a)?;
    let rb = existing_ancestor(b)?;
    volume_id(&ra).and_then(|va| volume_id(&rb).map(|vb| va == vb))
}

/// 向上找到第一个真实存在的祖先目录。
fn existing_ancestor(p: &Path) -> Option<std::path::PathBuf> {
    let mut cur = if p.exists() { Some(p.to_path_buf()) } else { p.parent().map(|x| x.to_path_buf()) };
    while let Some(c) = cur {
        if c.exists() {
            return Some(c);
        }
        cur = c.parent().map(|x| x.to_path_buf());
    }
    None
}

#[cfg(windows)]
fn volume_id(p: &Path) -> Option<String> {
    // Windows：用盘符（如 C:）作为卷标识。同盘符即同卷，rename 原子。
    use std::path::Component;
    p.canonicalize().ok().and_then(|abs| {
        abs.components().find_map(|c| match c {
            Component::Prefix(pre) => Some(format!("{:?}", pre.kind())),
            _ => None,
        })
    })
}

#[cfg(unix)]
fn volume_id(p: &Path) -> Option<String> {
    // Unix：用 st_dev（设备号）作为卷标识。
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).ok().map(|m| m.dev().to_string())
}
