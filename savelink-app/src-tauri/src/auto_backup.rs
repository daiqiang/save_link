use crate::commands::{
    acquire_cloud_sync_guard, baidu_connection_status, AppState, BaiduCloudRuntime,
    BAIDU_ACCOUNT_ID,
};
use savelink_core::cloud_model::CloudSyncStatus;
use savelink_core::cloud_repo::CloudStateRepository;
use savelink_core::model::{Reason, SnapshotStatus};
use savelink_core::service::{AutoBackupReport, AutoBackupService};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub const AUTO_BACKUP_ENABLED_SETTING: &str = "auto_backup_enabled";
pub const RETENTION_LIMIT_SETTING: &str = "max_unlocked_snapshots_per_game";
pub const RETENTION_POLICY_CONFIRMED_SETTING: &str = "retention_policy_confirmed";
pub const AUTO_BACKUP_INTERVAL_MINUTES: u64 = 10;
pub const AUTO_BACKUP_CHANGED_EVENT: &str = "auto-backup-changed";
pub const DEFAULT_RETENTION_LIMIT: usize = 10;
pub const MIN_RETENTION_LIMIT: usize = 1;
pub const MAX_RETENTION_LIMIT: usize = 100;

pub fn ensure_default(
    repo: &Arc<dyn CloudStateRepository>,
    has_existing_games: bool,
) -> Result<(), String> {
    if repo
        .get_setting(AUTO_BACKUP_ENABLED_SETTING)
        .map_err(|error| error.to_string())?
        .is_none()
    {
        repo.set_setting(AUTO_BACKUP_ENABLED_SETTING, "true")
            .map_err(|error| error.to_string())?;
    }
    let retention_limit = repo
        .get_setting(RETENTION_LIMIT_SETTING)
        .map_err(|error| error.to_string())?;
    if retention_limit
        .as_deref()
        .and_then(|value| value.parse::<usize>().ok())
        .is_none_or(|value| !is_valid_retention_limit(value))
    {
        repo.set_setting(
            RETENTION_LIMIT_SETTING,
            &DEFAULT_RETENTION_LIMIT.to_string(),
        )
        .map_err(|error| error.to_string())?;
    }
    if repo
        .get_setting(RETENTION_POLICY_CONFIRMED_SETTING)
        .map_err(|error| error.to_string())?
        .is_none()
    {
        // 已有 v0.2 数据库先保留旧快照，等用户明确确认新上限后再清理。
        repo.set_setting(
            RETENTION_POLICY_CONFIRMED_SETTING,
            if has_existing_games { "false" } else { "true" },
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn enabled(repo: &Arc<dyn CloudStateRepository>) -> Result<bool, String> {
    Ok(repo
        .get_setting(AUTO_BACKUP_ENABLED_SETTING)
        .map_err(|error| error.to_string())?
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(true))
}

pub fn set_enabled(repo: &Arc<dyn CloudStateRepository>, enabled: bool) -> Result<(), String> {
    repo.set_setting(
        AUTO_BACKUP_ENABLED_SETTING,
        if enabled { "true" } else { "false" },
    )
    .map_err(|error| error.to_string())
}

pub fn retention_limit(repo: &Arc<dyn CloudStateRepository>) -> Result<usize, String> {
    Ok(repo
        .get_setting(RETENTION_LIMIT_SETTING)
        .map_err(|error| error.to_string())?
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| is_valid_retention_limit(*value))
        .unwrap_or(DEFAULT_RETENTION_LIMIT))
}

pub fn retention_policy_confirmed(repo: &Arc<dyn CloudStateRepository>) -> Result<bool, String> {
    Ok(repo
        .get_setting(RETENTION_POLICY_CONFIRMED_SETTING)
        .map_err(|error| error.to_string())?
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(true))
}

pub fn set_retention_limit(
    repo: &Arc<dyn CloudStateRepository>,
    limit: usize,
) -> Result<(), String> {
    if !is_valid_retention_limit(limit) {
        return Err(format!(
            "快照保留数量必须是 {} 到 {} 之间的整数",
            MIN_RETENTION_LIMIT, MAX_RETENTION_LIMIT
        ));
    }
    repo.set_setting(RETENTION_LIMIT_SETTING, &limit.to_string())
        .map_err(|error| error.to_string())?;
    repo.set_setting(RETENTION_POLICY_CONFIRMED_SETTING, "true")
        .map_err(|error| error.to_string())
}

fn is_valid_retention_limit(limit: usize) -> bool {
    (MIN_RETENTION_LIMIT..=MAX_RETENTION_LIMIT).contains(&limit)
}

pub fn start(app: AppHandle) {
    let _ = std::thread::Builder::new()
        .name("savelink-auto-backup".into())
        .spawn(move || loop {
            if let Err(error) = run_once_if_enabled(&app) {
                eprintln!("自动备份检查失败: {error}");
            }
            std::thread::sleep(Duration::from_secs(AUTO_BACKUP_INTERVAL_MINUTES * 60));
        });
}

/// 用户从关闭切换为开启时立即补做一次检查，不必等待下一个十分钟周期。
pub fn trigger(app: AppHandle) {
    let _ = std::thread::Builder::new()
        .name("savelink-auto-backup-trigger".into())
        .spawn(move || {
            if let Err(error) = run_once_if_enabled(&app) {
                eprintln!("自动备份即时检查失败: {error}");
            }
        });
}

/// 已有自动快照产生后，只续做云同步与保留策略，不扫描其他游戏创建快照。
pub fn trigger_sync(app: AppHandle) {
    let _ = std::thread::Builder::new()
        .name("savelink-auto-sync-trigger".into())
        .spawn(move || {
            let state = app.state::<AppState>();
            match sync_pending_and_prune(&state) {
                Ok(true) => {
                    let _ = app.emit(AUTO_BACKUP_CHANGED_EVENT, ());
                }
                Ok(false) => {}
                Err(error) => eprintln!("自动云同步或快照清理失败: {error}"),
            }
        });
}

fn run_once_if_enabled(app: &AppHandle) -> Result<Option<AutoBackupReport>, String> {
    let state = app.state::<AppState>();
    let report = if enabled(&state.cloud_repo)? {
        let report = {
            let _operation = state
                .snapshot_operation_lock
                .lock()
                .map_err(|_| "快照操作锁已损坏".to_string())?;
            AutoBackupService::new(state.snapshots())
                .run_once()
                .map_err(|error| error.to_string())?
        };

        for failure in &report.failures {
            eprintln!("游戏 {} 自动备份失败: {}", failure.game_id, failure.error);
        }
        if !report.created_snapshots.is_empty() {
            let _ = app.emit(AUTO_BACKUP_CHANGED_EVENT, ());
        }
        Some(report)
    } else {
        // 自动备份关闭时仍运行维护周期：整理区域、清理过期快照和续做云端收尾。
        None
    };
    match sync_pending_and_prune(&state) {
        Ok(true) => {
            let _ = app.emit(AUTO_BACKUP_CHANGED_EVENT, ());
        }
        Ok(false) => {}
        Err(error) => eprintln!("自动云同步或快照清理失败: {error}"),
    }
    Ok(report)
}

fn sync_pending_and_prune(state: &AppState) -> Result<bool, String> {
    let retention_limit = retention_limit(&state.cloud_repo)?;
    let retention_policy_confirmed = retention_policy_confirmed(&state.cloud_repo)?;
    let layout_changed = {
        let _operation = state
            .snapshot_operation_lock
            .lock()
            .map_err(|_| "快照操作锁已损坏".to_string())?;
        let games = state.repo.list_games().map_err(|error| error.to_string())?;
        let max_unlocked = if retention_policy_confirmed {
            retention_limit
        } else {
            usize::MAX
        };
        let snapshots = AutoBackupService::new(state.snapshots());
        let mut changed = false;
        for game in games {
            changed |= snapshots
                .organize_snapshot_layout(&game.id, max_unlocked)
                .map_err(|error| error.to_string())?
                > 0;
        }
        changed
    };

    let _cloud_guard = match acquire_cloud_sync_guard(state) {
        Ok(guard) => guard,
        Err(_) => return Ok(layout_changed),
    };

    let games = state.repo.list_games().map_err(|error| error.to_string())?;
    let snapshots = AutoBackupService::new(state.snapshots());
    let mut retention_candidates = Vec::new();
    let mut retention_ids = HashSet::new();
    if retention_policy_confirmed {
        for game in &games {
            for snapshot in snapshots
                .unlocked_retention_candidates(&game.id, retention_limit)
                .map_err(|error| error.to_string())?
            {
                retention_ids.insert(snapshot.id.clone());
                retention_candidates.push(snapshot);
            }
        }
    }

    // 删除一旦进入生命周期，即使当前总数后来降到上限以内，也必须续做收尾。
    for status in [
        CloudSyncStatus::DeletePending,
        CloudSyncStatus::Deleting,
        CloudSyncStatus::DeleteFailed,
        CloudSyncStatus::RemoteDeleted,
    ] {
        for record in state
            .cloud_repo
            .list_cloud_snapshots_by_status(BAIDU_ACCOUNT_ID, status)
            .map_err(|error| error.to_string())?
        {
            if retention_ids.insert(record.snapshot_id.clone()) {
                if let Some(snapshot) = state
                    .repo
                    .get_snapshot(&record.snapshot_id)
                    .map_err(|error| error.to_string())?
                {
                    retention_candidates.push(snapshot);
                }
            }
        }
    }

    let connected = baidu_connection_status(&state.cloud_repo, &state.baidu_token_store)?.connected;
    let service = if connected || !retention_candidates.is_empty() {
        Some(BaiduCloudRuntime::from_state(state).build()?)
    } else {
        None
    };

    let mut cloud_changed = layout_changed;
    if connected {
        let service = service.as_ref().expect("已连接百度网盘时必须构造云服务");
        let mut stop_uploading = false;
        for game in &games {
            for snapshot in state
                .repo
                .list_snapshots(&game.id)
                .map_err(|error| error.to_string())?
            {
                if retention_ids.contains(&snapshot.id)
                    || snapshot.status != SnapshotStatus::Complete
                    || snapshot.reason != Reason::Auto
                {
                    continue;
                }
                let cloud_status = state
                    .cloud_repo
                    .get_cloud_snapshot(BAIDU_ACCOUNT_ID, &snapshot.id)
                    .map_err(|error| error.to_string())?
                    .map(|record| record.sync_status);
                if !needs_upload(cloud_status) {
                    continue;
                }

                let _operation = state
                    .snapshot_operation_lock
                    .lock()
                    .map_err(|_| "快照操作锁已损坏".to_string())?;
                match service.upload_snapshot(&game.id, &snapshot.id) {
                    Ok(_) => cloud_changed = true,
                    Err(error) => {
                        cloud_changed = true;
                        eprintln!("快照 {} 自动上传失败: {}", snapshot.id, error);
                        if matches!(
                            error.code(),
                            "auth_required" | "network_unavailable" | "rate_limited"
                        ) {
                            if error.code() == "auth_required" {
                                let _ = state.baidu_token_store.clear();
                            }
                            stop_uploading = true;
                            break;
                        }
                    }
                }
            }
            if stop_uploading {
                break;
            }
        }
    }

    for candidate in retention_candidates {
        let service = service.as_ref().expect("存在待清理快照时必须构造云服务");
        let _operation = state
            .snapshot_operation_lock
            .lock()
            .map_err(|_| "快照操作锁已损坏".to_string())?;
        match service.delete_snapshot_everywhere(&candidate.id) {
            Ok(()) => cloud_changed = true,
            Err(error) => {
                cloud_changed = true;
                eprintln!("快照 {} 自动清理失败: {}", candidate.id, error);
            }
        }
    }

    // 处理“云端已删、应用在本机删除阶段退出”的收尾记录。
    for record in state
        .cloud_repo
        .list_cloud_snapshots_by_status(BAIDU_ACCOUNT_ID, CloudSyncStatus::RemoteDeleted)
        .map_err(|error| error.to_string())?
    {
        if state
            .repo
            .get_snapshot(&record.snapshot_id)
            .map_err(|error| error.to_string())?
            .is_none()
        {
            state
                .cloud_repo
                .delete_cloud_snapshot(BAIDU_ACCOUNT_ID, &record.snapshot_id)
                .map_err(|error| error.to_string())?;
            cloud_changed = true;
        }
    }
    Ok(cloud_changed)
}

fn needs_upload(status: Option<CloudSyncStatus>) -> bool {
    matches!(
        status,
        None | Some(CloudSyncStatus::Uploading) | Some(CloudSyncStatus::Error)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_or_failed_uploads_are_retried() {
        assert!(needs_upload(None));
        assert!(needs_upload(Some(CloudSyncStatus::Uploading)));
        assert!(needs_upload(Some(CloudSyncStatus::Error)));
    }

    #[test]
    fn published_downloaded_or_deleting_snapshots_are_not_reuploaded() {
        for status in [
            CloudSyncStatus::Uploaded,
            CloudSyncStatus::Downloaded,
            CloudSyncStatus::DeletePending,
            CloudSyncStatus::Deleting,
            CloudSyncStatus::DeleteFailed,
            CloudSyncStatus::RemoteDeleted,
        ] {
            assert!(!needs_upload(Some(status)), "unexpected retry: {status:?}");
        }
    }

    #[test]
    fn retention_limit_defaults_to_ten_and_rejects_values_outside_range() {
        use savelink_core::cloud_repo::CloudStateRepository;
        use savelink_core::sqlite_repo::SqliteRepo;

        let repo: Arc<dyn CloudStateRepository> =
            Arc::new(SqliteRepo::open_in_memory().expect("内存数据库应初始化"));
        ensure_default(&repo, false).expect("默认设置应写入");
        assert_eq!(retention_limit(&repo).unwrap(), DEFAULT_RETENTION_LIMIT);
        assert!(retention_policy_confirmed(&repo).unwrap());

        set_retention_limit(&repo, 1).expect("最小值应可保存");
        assert_eq!(retention_limit(&repo).unwrap(), 1);
        set_retention_limit(&repo, 100).expect("最大值应可保存");
        assert_eq!(retention_limit(&repo).unwrap(), 100);
        assert!(set_retention_limit(&repo, 0).is_err());
        assert!(set_retention_limit(&repo, 101).is_err());
    }

    #[test]
    fn existing_database_must_confirm_the_new_default_before_pruning() {
        use savelink_core::cloud_repo::CloudStateRepository;
        use savelink_core::sqlite_repo::SqliteRepo;

        let repo: Arc<dyn CloudStateRepository> =
            Arc::new(SqliteRepo::open_in_memory().expect("内存数据库应初始化"));
        repo.set_setting(AUTO_BACKUP_ENABLED_SETTING, "true")
            .expect("旧版自动备份设置应可写入");
        ensure_default(&repo, true).expect("旧数据库默认设置应初始化");

        assert_eq!(retention_limit(&repo).unwrap(), DEFAULT_RETENTION_LIMIT);
        assert!(!retention_policy_confirmed(&repo).unwrap());
        set_retention_limit(&repo, DEFAULT_RETENTION_LIMIT).expect("确认默认上限应成功");
        assert!(retention_policy_confirmed(&repo).unwrap());
    }
}
