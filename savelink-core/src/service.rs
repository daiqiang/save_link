//! 已实现的业务服务层。
//!
//! 服务不感知 Tauri。依赖通过构造注入：Repository + SnapshotStore + Clock + IdGen。
//! 每个方法的契约注释直接对应 `doc/SaveLink恢复与存储测试规格.md` 的用例编号。

use crate::error::{Result, SaveLinkError};
use crate::model::{CreateOutcome, RestoreOutcome, RestoreStep, SaveSource};
use crate::model::{Snapshot, SnapshotDisplayZone, SnapshotStatus};
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
        Self {
            repo,
            store,
            clock,
            ids,
        }
    }

    /// 创建快照。
    ///
    /// 契约（对应 A 组）：
    /// - A1：成功时返回 Created，file_count/total_size/内容指纹正确，reason=Manual。
    /// - A2：扫描 content_hash 与上一快照相同 → 返回 NoChange，不写库不写文件。
    /// - A3：任一文件变化即视为有变化。
    /// - A4：先以 status=Writing 落库 → store.create → verify → 置 Complete。
    ///   任一步失败：删半成品文件 + 删记录，不留 Writing 悬挂；不碰真实存档。
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

    /// 只读比较当前真实存档与指定快照，不创建记录也不写入任何存档文件。
    pub fn current_save_matches(&self, snapshot_id: &str) -> Result<bool> {
        let target = self
            .repo
            .get_snapshot(snapshot_id)?
            .ok_or_else(|| SaveLinkError::Io(format!("snapshot not found: {snapshot_id}")))?;
        if target.status != SnapshotStatus::Complete {
            return Err(SaveLinkError::SnapshotCorrupt);
        }
        let game = self
            .repo
            .get_game(&target.game_id)?
            .ok_or_else(|| SaveLinkError::Io(format!("game not found: {}", target.game_id)))?;
        if !game.is_configured() {
            return Err(SaveLinkError::SaveSourcesNotConfigured);
        }

        let configured_sources = game.effective_save_sources();
        let save_sources = snapshot_save_sources(&configured_sources, target.source_count)?;
        let current = scan::scan_save_sources(&save_sources)?;
        Ok(snapshot_matches_scan(&target, &current))
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
        if !game.is_configured() {
            return Err(SaveLinkError::SaveSourcesNotConfigured);
        }

        let save_sources = game.effective_save_sources();
        scan::validate_save_sources(&save_sources)?;

        // A6：扫描，目录不存在/不可读直接返回错误，不写任何状态。
        let ctx = scan::scan_save_sources(&save_sources)?;

        // A2/A3：与最近一个快照内容比对，一致则不创建。
        let latest = self.repo.list_snapshots(game_id)?.into_iter().next();
        if let Some(prev) = &latest {
            if prev.content_hash == ctx.content_hash {
                return Ok(CreateOutcome::NoChange);
            }
        }

        // A4：先以 Writing 落库，再写文件；任一步失败则回滚记录 + 清半成品。
        let id = self.ids.new_id("snap");
        let created_at = self.clock.now_stamp();
        let pending = Snapshot {
            id: id.clone(),
            game_id: game_id.to_string(),
            created_at: created_at.clone(),
            note,
            note_updated_at: created_at.clone(),
            reason,
            locked: false,
            locked_updated_at: created_at,
            display_zone: crate::model::SnapshotDisplayZone::Normal,
            file_count: ctx.file_count,
            total_size: ctx.total_size,
            source_count: save_sources.len().max(1) as u32,
            content_hash: ctx.content_hash.clone(),
            storage_key: id.clone(),
            status: SnapshotStatus::Writing,
        };
        self.repo.insert_snapshot(pending.clone())?;

        let stored = match self.store.create_save_sources(&id, &save_sources, &ctx) {
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
        let note_changed = note
            .as_ref()
            .is_some_and(|value| snap.note.as_ref() != Some(value));
        let locked_changed = locked.is_some_and(|value| snap.locked != value);
        let changed_at = (note_changed || locked_changed).then(|| self.clock.now_stamp());
        if let Some(n) = note.filter(|_| note_changed) {
            snap.note = Some(n);
            snap.note_updated_at = changed_at.clone().expect("已确认名称发生变化");
        }
        if let Some(l) = locked.filter(|_| locked_changed) {
            snap.locked = l;
            snap.locked_updated_at = changed_at.expect("已确认锁定状态发生变化");
        }
        // content_hash / created_at / file_count / total_size 一律不动。
        self.repo.update_snapshot(snap)
    }

    /// 在维护周期内整理快照显示区域。
    ///
    /// `locked` 只表示保护状态，用户点击锁定/解锁时立即生效；本方法才会把快照
    /// 移入与状态相符的显示区域。未锁定且超出保留数量的快照保持原区域，交由上层
    /// 完成云端感知删除后再移除，这样删除失败时仍能显示“待整理”状态。
    pub fn organize_snapshot_layout(&self, game_id: &str, max_unlocked: usize) -> Result<usize> {
        let snapshots = self.repo.list_snapshots(game_id)?;
        let retained_unlocked: std::collections::HashSet<String> = snapshots
            .iter()
            .filter(|snapshot| !snapshot.locked && snapshot.status == SnapshotStatus::Complete)
            .take(max_unlocked)
            .map(|snapshot| snapshot.id.clone())
            .collect();

        let mut changed = 0;
        for mut snapshot in snapshots {
            let desired_zone = if snapshot.locked {
                SnapshotDisplayZone::Locked
            } else if retained_unlocked.contains(&snapshot.id) {
                SnapshotDisplayZone::Normal
            } else {
                // 超出保留范围的快照要等删除流程成功后消失，不能提前伪装成已整理。
                continue;
            };
            if snapshot.display_zone != desired_zone {
                snapshot.display_zone = desired_zone;
                self.repo.update_snapshot(snapshot)?;
                changed += 1;
            }
        }
        Ok(changed)
    }

    /// 删除快照。
    ///
    /// 契约（C 组）：
    /// - C1：locked → SnapshotLocked，记录与文件均保留。
    /// - C3：先标记 Deleting，再删文件和记录；删文件失败恢复原状态。
    ///   进程中断留下的 Deleting 由启动自检幂等续做。
    pub fn delete_snapshot(&self, snapshot_id: &str) -> Result<()> {
        let mut snap = self
            .repo
            .get_snapshot(snapshot_id)?
            .ok_or_else(|| SaveLinkError::Io(format!("snapshot not found: {snapshot_id}")))?;
        if snap.locked {
            return Err(SaveLinkError::SnapshotLocked); // C1
        }
        let previous_status = snap.status;
        snap.status = SnapshotStatus::Deleting;
        self.repo.update_snapshot(snap.clone())?;
        if let Err(error) = self.store.delete(&snap.storage_key) {
            snap.status = previous_status;
            let _ = self.repo.update_snapshot(snap);
            return Err(error);
        }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoBackupFailure {
    pub game_id: String,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AutoBackupReport {
    pub checked_game_ids: Vec<String>,
    pub created_snapshots: Vec<Snapshot>,
    pub unchanged_game_ids: Vec<String>,
    pub skipped_game_ids: Vec<String>,
    pub failures: Vec<AutoBackupFailure>,
}

/// 单次自动备份检查。调度间隔和全局开关属于桌面壳职责，核心层只处理一次检查。
pub struct AutoBackupService {
    snapshots: SnapshotService,
}

impl AutoBackupService {
    pub fn new(snapshots: SnapshotService) -> Self {
        Self { snapshots }
    }

    /// 执行一次本机时间线区域整理，不创建快照也不执行删除。
    pub fn organize_snapshot_layout(&self, game_id: &str, max_unlocked: usize) -> Result<usize> {
        self.snapshots
            .organize_snapshot_layout(game_id, max_unlocked)
    }

    /// 检查所有游戏；单个游戏失败不会阻断其他游戏。
    pub fn run_once(&self) -> Result<AutoBackupReport> {
        let games = self.snapshots.repo.list_games()?;
        let mut report = AutoBackupReport::default();

        for game in games {
            if game.effective_save_sources().is_empty() {
                report.skipped_game_ids.push(game.id);
                continue;
            }

            report.checked_game_ids.push(game.id.clone());
            match self
                .snapshots
                .create_snapshot(&game.id, None, crate::model::Reason::Auto)
            {
                Ok(CreateOutcome::Created(snapshot)) => {
                    report.created_snapshots.push(snapshot);
                }
                Ok(CreateOutcome::NoChange) => {
                    report.unchanged_game_ids.push(game.id);
                }
                Err(error) => report.failures.push(AutoBackupFailure {
                    game_id: game.id,
                    error: error.to_string(),
                }),
            }
        }

        Ok(report)
    }

    /// 返回超过上限、需要进入“先删云端，再删本地”流程的快照。
    ///
    /// 这里故意不执行删除：生产删除必须由云端感知的保留策略协调器完成。
    /// 所有来源的完整未锁定快照都计入上限，锁定快照永远不计入。
    pub fn unlocked_retention_candidates(
        &self,
        game_id: &str,
        max_unlocked: usize,
    ) -> Result<Vec<Snapshot>> {
        let mut candidates: Vec<Snapshot> = self
            .snapshots
            .repo
            .list_snapshots(game_id)?
            .into_iter()
            .filter(|snapshot| !snapshot.locked && snapshot.status == SnapshotStatus::Complete)
            .skip(max_unlocked)
            .collect();
        candidates.reverse();
        Ok(candidates)
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
        if !game.is_configured() {
            return Err(SaveLinkError::SaveSourcesNotConfigured);
        }
        let target = self
            .repo
            .get_snapshot(snapshot_id)?
            .ok_or_else(|| SaveLinkError::Io(format!("snapshot not found: {snapshot_id}")))?;
        let configured_sources = game.effective_save_sources();
        scan::validate_save_sources(&configured_sources)?;
        let save_sources = snapshot_save_sources(&configured_sources, target.source_count)?;

        // 步骤 1（前置）：目标快照必须完好（B4）。
        if !self.store.verify(&target.storage_key)? {
            return Err(SaveLinkError::SnapshotCorrupt);
        }

        // 步骤 2：真实存档目录不存在 → 交还用户决策，未确认前不写入（B7）。
        if save_sources.iter().any(|source| !source.root().exists()) {
            return Err(SaveLinkError::SaveDirMissingNeedsChoice);
        }

        self.do_restore(&target, &save_sources, progress)
    }

    /// 真正执行恢复的内部流程。前置检查已在调用方完成。
    fn do_restore(
        &self,
        target: &Snapshot,
        save_sources: &[SaveSource],
        progress: &ProgressSink<'_>,
    ) -> Result<RestoreOutcome> {
        if save_sources.iter().all(SaveSource::is_directory) {
            let save_dirs = save_sources
                .iter()
                .map(|source| source.root().to_path_buf())
                .collect::<Vec<_>>();
            return self.do_restore_directories(target, &save_dirs, progress);
        }
        if save_sources.iter().all(|source| !source.is_directory()) {
            return self.do_restore_selected_files(target, save_sources, progress);
        }
        Err(SaveLinkError::Io(
            "暂不支持整目录与精确文件来源混合恢复".into(),
        ))
    }

    fn do_restore_directories(
        &self,
        target: &Snapshot,
        save_dirs: &[std::path::PathBuf],
        progress: &ProgressSink<'_>,
    ) -> Result<RestoreOutcome> {
        let current = scan::scan(save_dirs).map_err(|_| SaveLinkError::SaveDirUnreadable)?;

        // 已经是目标版本：不覆盖，也不发出虚假的恢复进度。
        if snapshot_matches_scan(target, &current) {
            return Ok(RestoreOutcome {
                target_id: target.id.clone(),
                restored: false,
            });
        }

        // 覆盖恢复采用"旁路 + 同盘 rename 替换"。
        progress(RestoreStep::RestoreTarget);
        let stamp = self.clock.now_stamp().replace([' ', ':'], "");
        let mut tmp_dirs = Vec::with_capacity(save_dirs.len());
        let mut old_dirs = Vec::with_capacity(save_dirs.len());
        for (index, save_dir) in save_dirs.iter().enumerate() {
            let parent = save_dir
                .parent()
                .ok_or_else(|| SaveLinkError::Io("save dir has no parent".into()))?;
            tmp_dirs.push(parent.join(format!(".savelink_tmp_{stamp}_{index}")));
            old_dirs.push(parent.join(format!(".savelink_old_{stamp}_{index}")));
        }

        // 4a. 解包目标到同盘临时目录 + 校验。失败 → 真实存档未动，直接返回（已回滚=未开始覆盖）。
        cleanup_dirs(&tmp_dirs);
        cleanup_dirs(&old_dirs);
        if let Err(e) = self.store.restore_sources(&target.storage_key, &tmp_dirs) {
            cleanup_dirs(&tmp_dirs);
            return Err(restore_failed(e, true)); // 尚未触碰真实目录 → 视为已回滚
        }
        // 校验全部临时目录的聚合内容与目标快照一致。
        match scan::scan(&tmp_dirs) {
            Ok(r)
                if r.content_hash == target.content_hash
                    && r.file_count == target.file_count
                    && r.total_size == target.total_size => {}
            _ => {
                cleanup_dirs(&tmp_dirs);
                return Err(SaveLinkError::RestoreFailed { rolled_back: true });
            }
        }

        // 4b. 每个根目录都在各自磁盘上旁路替换；中途失败会把已经替换的目录全部回滚。
        if let Err((error, rolled_back)) = replace_save_dirs(save_dirs, &tmp_dirs, &old_dirs) {
            cleanup_dirs(&tmp_dirs);
            return Err(restore_failed_io(error, rolled_back));
        }
        // 最终校验通过前保留 .old；失败时立即恢复原目录。
        progress(RestoreStep::Verify);
        if !matches!(scan::scan(save_dirs), Ok(ref r) if snapshot_matches_scan(target, r)) {
            let rolled_back = rollback_save_dirs(save_dirs, &old_dirs, &current);
            return Err(SaveLinkError::RestoreFailed { rolled_back });
        }

        cleanup_dirs(&old_dirs);

        Ok(RestoreOutcome {
            target_id: target.id.clone(),
            restored: true,
        })
    }

    fn do_restore_selected_files(
        &self,
        target: &Snapshot,
        save_sources: &[SaveSource],
        progress: &ProgressSink<'_>,
    ) -> Result<RestoreOutcome> {
        let current =
            scan::scan_save_sources(save_sources).map_err(|_| SaveLinkError::SaveDirUnreadable)?;
        if snapshot_matches_scan(target, &current) {
            return Ok(RestoreOutcome {
                target_id: target.id.clone(),
                restored: false,
            });
        }

        progress(RestoreStep::RestoreTarget);
        let stamp = restore_stamp(&self.clock.now_stamp());
        let tmp_dirs = save_sources
            .iter()
            .enumerate()
            .map(|(index, source)| {
                source
                    .root()
                    .parent()
                    .ok_or_else(|| SaveLinkError::Io("save dir has no parent".into()))
                    .map(|parent| parent.join(format!(".savelink_selected_{stamp}_{index}")))
            })
            .collect::<Result<Vec<_>>>()?;
        cleanup_dirs(&tmp_dirs);
        if let Err(error) = self.store.restore_sources(&target.storage_key, &tmp_dirs) {
            cleanup_dirs(&tmp_dirs);
            return Err(restore_failed(error, true));
        }
        match scan::scan(&tmp_dirs) {
            Ok(scan)
                if scan.content_hash == target.content_hash
                    && scan.file_count == target.file_count
                    && scan.total_size == target.total_size => {}
            _ => {
                cleanup_dirs(&tmp_dirs);
                return Err(SaveLinkError::RestoreFailed { rolled_back: true });
            }
        }

        let replacements = match prepare_selected_replacements(save_sources, &tmp_dirs, &stamp) {
            Ok(replacements) => replacements,
            Err(error) => {
                cleanup_dirs(&tmp_dirs);
                return Err(restore_failed(error, true));
            }
        };
        if let Err((error, rolled_back)) = apply_selected_replacements(&replacements) {
            cleanup_selected_replacements(&replacements);
            cleanup_dirs(&tmp_dirs);
            return Err(restore_failed_io(error, rolled_back));
        }

        progress(RestoreStep::Verify);
        if !matches!(scan::scan_save_sources(save_sources), Ok(ref scan) if snapshot_matches_scan(target, scan))
        {
            let rolled_back = rollback_selected_replacements(&replacements);
            cleanup_selected_replacements(&replacements);
            cleanup_dirs(&tmp_dirs);
            return Err(SaveLinkError::RestoreFailed { rolled_back });
        }

        cleanup_selected_replacements(&replacements);
        cleanup_dirs(&tmp_dirs);
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
        if !game.is_configured() {
            return Err(SaveLinkError::SaveSourcesNotConfigured);
        }
        let target = self
            .repo
            .get_snapshot(snapshot_id)?
            .ok_or_else(|| SaveLinkError::Io(format!("snapshot not found: {snapshot_id}")))?;
        let configured_sources = game.effective_save_sources();
        scan::validate_save_sources(&configured_sources)?;
        let save_sources = snapshot_save_sources(&configured_sources, target.source_count)?;

        if !self.store.verify(&target.storage_key)? {
            return Err(SaveLinkError::SnapshotCorrupt);
        }

        match choice {
            MissingDirChoice::Cancel | MissingDirChoice::Reselect => {
                // 不写入任何文件，交还上层（B7）。
                Err(SaveLinkError::SaveDirMissingNeedsChoice)
            }
            MissingDirChoice::CreateAndRestore => {
                let mut created = Vec::new();
                for save_dir in save_sources.iter().map(SaveSource::root) {
                    if save_dir.exists() {
                        continue;
                    }
                    if let Err(error) = fs::create_dir_all(save_dir) {
                        cleanup_dirs(&created);
                        return Err(SaveLinkError::Io(error.to_string()));
                    }
                    created.push(save_dir.to_path_buf());
                }
                self.do_restore(&target, &save_sources, progress)
            }
        }
    }
}

fn snapshot_save_sources(configured: &[SaveSource], source_count: u32) -> Result<Vec<SaveSource>> {
    let expected = source_count.max(1) as usize;
    if expected == 1 {
        return configured
            .first()
            .cloned()
            .map(|source| vec![source])
            .ok_or_else(|| SaveLinkError::Io("game has no save path".into()));
    }
    if configured.len() != expected {
        return Err(SaveLinkError::Io(format!(
            "该快照包含 {expected} 个存档目录，请先为这台电脑绑定全部目录"
        )));
    }
    Ok(configured.to_vec())
}

#[derive(Debug)]
struct SelectedFileReplacement {
    target: std::path::PathBuf,
    incoming: Option<std::path::PathBuf>,
    backup: std::path::PathBuf,
    had_original: bool,
}

fn restore_stamp(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect()
}

fn prepare_selected_replacements(
    save_sources: &[SaveSource],
    payload_dirs: &[std::path::PathBuf],
    stamp: &str,
) -> Result<Vec<SelectedFileReplacement>> {
    if save_sources.len() != payload_dirs.len() {
        return Err(SaveLinkError::SnapshotCorrupt);
    }
    let mut replacements = Vec::new();
    for (source_index, (source, payload_dir)) in save_sources.iter().zip(payload_dirs).enumerate() {
        let SaveSource::Files { root, files } = source else {
            return Err(SaveLinkError::Io("精确文件恢复收到整目录来源".into()));
        };
        for (file_index, mapping) in files.iter().enumerate() {
            let target = root.join(&mapping.local_relative_path);
            let parent = target
                .parent()
                .ok_or_else(|| SaveLinkError::Io("存档文件没有父目录".into()))?;
            fs::create_dir_all(parent).map_err(|error| SaveLinkError::Io(error.to_string()))?;
            let suffix = format!("{stamp}_{source_index}_{file_index}");
            let incoming_path = parent.join(format!(".savelink_incoming_{suffix}"));
            let backup = parent.join(format!(".savelink_backup_{suffix}"));
            let _ = fs::remove_file(&incoming_path);
            let _ = fs::remove_file(&backup);
            let payload = payload_dir.join(&mapping.snapshot_relative_path);
            let incoming = if payload.exists() {
                if !payload.is_file() {
                    return Err(SaveLinkError::SnapshotCorrupt);
                }
                fs::copy(&payload, &incoming_path)
                    .map_err(|error| SaveLinkError::Io(error.to_string()))?;
                Some(incoming_path)
            } else {
                None
            };
            replacements.push(SelectedFileReplacement {
                had_original: target.exists(),
                target,
                incoming,
                backup,
            });
        }
    }
    Ok(replacements)
}

fn apply_selected_replacements(
    replacements: &[SelectedFileReplacement],
) -> std::result::Result<(), (std::io::Error, bool)> {
    for (index, replacement) in replacements.iter().enumerate() {
        if replacement.had_original {
            if let Err(error) = fs::rename(&replacement.target, &replacement.backup) {
                let rolled_back = rollback_moved_selected_replacements(replacements, index);
                return Err((error, rolled_back));
            }
        }
    }
    for replacement in replacements {
        if let Some(incoming) = &replacement.incoming {
            if let Err(error) = fs::rename(incoming, &replacement.target) {
                let rolled_back = rollback_selected_replacements(replacements);
                return Err((error, rolled_back));
            }
        }
    }
    Ok(())
}

fn rollback_moved_selected_replacements(
    replacements: &[SelectedFileReplacement],
    processed: usize,
) -> bool {
    let mut rolled_back = true;
    for replacement in replacements[..processed].iter().rev() {
        if replacement.had_original
            && replacement.backup.exists()
            && fs::rename(&replacement.backup, &replacement.target).is_err()
        {
            rolled_back = false;
        }
    }
    rolled_back
}

fn rollback_selected_replacements(replacements: &[SelectedFileReplacement]) -> bool {
    let mut rolled_back = true;
    for replacement in replacements.iter().rev() {
        if replacement.target.exists() && fs::remove_file(&replacement.target).is_err() {
            rolled_back = false;
            continue;
        }
        if replacement.had_original
            && replacement.backup.exists()
            && fs::rename(&replacement.backup, &replacement.target).is_err()
        {
            rolled_back = false;
        }
    }
    rolled_back
}

fn cleanup_selected_replacements(replacements: &[SelectedFileReplacement]) {
    for replacement in replacements {
        if let Some(incoming) = &replacement.incoming {
            let _ = fs::remove_file(incoming);
        }
        let _ = fs::remove_file(&replacement.backup);
    }
}

fn snapshot_matches_scan(snapshot: &Snapshot, scan: &crate::model::ScanResult) -> bool {
    snapshot.content_hash == scan.content_hash
        && snapshot.file_count == scan.file_count
        && snapshot.total_size == scan.total_size
}

fn cleanup_dirs(paths: &[std::path::PathBuf]) {
    for path in paths {
        let _ = fs::remove_dir_all(path);
    }
}

fn replace_save_dirs(
    save_dirs: &[std::path::PathBuf],
    tmp_dirs: &[std::path::PathBuf],
    old_dirs: &[std::path::PathBuf],
) -> std::result::Result<(), (std::io::Error, bool)> {
    replace_save_dirs_with(save_dirs, tmp_dirs, old_dirs, &mut |from, to| {
        fs::rename(from, to)
    })
}

fn replace_save_dirs_with(
    save_dirs: &[std::path::PathBuf],
    tmp_dirs: &[std::path::PathBuf],
    old_dirs: &[std::path::PathBuf],
    rename: &mut impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> std::result::Result<(), (std::io::Error, bool)> {
    if save_dirs.len() != tmp_dirs.len() || save_dirs.len() != old_dirs.len() {
        return Err((
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "restore directory lists have different lengths",
            ),
            true,
        ));
    }

    for (moved_old, (save_dir, old_dir)) in save_dirs.iter().zip(old_dirs).enumerate() {
        if let Err(error) = rename(save_dir, old_dir) {
            let rolled_back = rollback_moved_old(save_dirs, old_dirs, moved_old, rename);
            return Err((error, rolled_back));
        }
    }

    for (tmp_dir, save_dir) in tmp_dirs.iter().zip(save_dirs) {
        if let Err(error) = rename(tmp_dir, save_dir) {
            let rolled_back = rollback_save_dirs_with(save_dirs, old_dirs, rename);
            return Err((error, rolled_back));
        }
    }
    Ok(())
}

fn rollback_moved_old(
    save_dirs: &[std::path::PathBuf],
    old_dirs: &[std::path::PathBuf],
    moved: usize,
    rename: &mut impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> bool {
    let mut rolled_back = true;
    for index in (0..moved).rev() {
        if rename(&old_dirs[index], &save_dirs[index]).is_err() {
            rolled_back = false;
        }
    }
    rolled_back
}

fn rollback_save_dirs(
    save_dirs: &[std::path::PathBuf],
    old_dirs: &[std::path::PathBuf],
    expected: &crate::model::ScanResult,
) -> bool {
    let restored =
        rollback_save_dirs_with(save_dirs, old_dirs, &mut |from, to| fs::rename(from, to));
    restored && matches!(scan::scan(save_dirs), Ok(ref result) if result == expected)
}

fn rollback_save_dirs_with(
    save_dirs: &[std::path::PathBuf],
    old_dirs: &[std::path::PathBuf],
    rename: &mut impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> bool {
    let mut rolled_back = true;
    for index in (0..save_dirs.len()).rev() {
        if save_dirs[index].exists() && fs::remove_dir_all(&save_dirs[index]).is_err() {
            rolled_back = false;
            continue;
        }
        if rename(&old_dirs[index], &save_dirs[index]).is_err() {
            rolled_back = false;
        }
    }
    rolled_back
}

/// 把任意错误归一为 RestoreFailed，携带回滚语义（B6）。
fn restore_failed(_e: SaveLinkError, rolled_back: bool) -> SaveLinkError {
    SaveLinkError::RestoreFailed { rolled_back }
}
fn restore_failed_io(_e: std::io::Error, rolled_back: bool) -> SaveLinkError {
    SaveLinkError::RestoreFailed { rolled_back }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::replace_save_dirs_with;
    use crate::testkit::{write_files, TempDir};
    use std::fs;
    use std::io;

    #[test]
    fn partial_multi_source_replace_rolls_back_every_original_directory() {
        let temp = TempDir::new();
        let save_dirs = vec![temp.child("save-0"), temp.child("save-1")];
        let tmp_dirs = vec![temp.child("tmp-0"), temp.child("tmp-1")];
        let old_dirs = vec![temp.path().join("old-0"), temp.path().join("old-1")];
        write_files(&save_dirs[0], &[("slot.sav", b"OLD-0")]);
        write_files(&save_dirs[1], &[("profile.dat", b"OLD-1")]);
        write_files(&tmp_dirs[0], &[("slot.sav", b"NEW-0")]);
        write_files(&tmp_dirs[1], &[("profile.dat", b"NEW-1")]);

        let mut rename_calls = 0usize;
        let result = replace_save_dirs_with(&save_dirs, &tmp_dirs, &old_dirs, &mut |from, to| {
            rename_calls += 1;
            if rename_calls == 4 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected second-target failure",
                ));
            }
            fs::rename(from, to)
        });

        assert!(matches!(result, Err((_, true))));
        assert_eq!(fs::read(save_dirs[0].join("slot.sav")).unwrap(), b"OLD-0");
        assert_eq!(
            fs::read(save_dirs[1].join("profile.dat")).unwrap(),
            b"OLD-1"
        );
        assert!(!old_dirs[0].exists());
        assert!(!old_dirs[1].exists());
    }
}

/// 启动自检（E 组）。
///
/// 契约：
/// - E1：清理 status==Writing 的残留记录（标记 Corrupt 或删除），并清除对应半成品文件；
///   清理后它们不得出现在正常时间线。
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
    // 云端已删除但本机删除中断时，幂等续做本地物理删除。
    for deleting in repo.list_deleting()? {
        if store.delete(&deleting.storage_key).is_ok() {
            repo.delete_snapshot(&deleting.id)?;
        }
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
    let mut cur = if p.exists() {
        Some(p.to_path_buf())
    } else {
        p.parent().map(|x| x.to_path_buf())
    };
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
