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
pub const AUTO_BACKUP_INTERVAL_MINUTES: u64 = 10;
pub const AUTO_BACKUP_CHANGED_EVENT: &str = "auto-backup-changed";
pub const MAX_UNLOCKED_SNAPSHOTS_PER_GAME: usize = 30;

pub fn ensure_default(repo: &Arc<dyn CloudStateRepository>) -> Result<(), String> {
    if repo
        .get_setting(AUTO_BACKUP_ENABLED_SETTING)
        .map_err(|error| error.to_string())?
        .is_none()
    {
        repo.set_setting(AUTO_BACKUP_ENABLED_SETTING, "true")
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

fn run_once_if_enabled(app: &AppHandle) -> Result<Option<AutoBackupReport>, String> {
    let state = app.state::<AppState>();
    if !enabled(&state.cloud_repo)? {
        return Ok(None);
    }

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
    match sync_pending_and_prune(&state) {
        Ok(true) => {
            let _ = app.emit(AUTO_BACKUP_CHANGED_EVENT, ());
        }
        Ok(false) => {}
        Err(error) => eprintln!("自动云同步或快照清理失败: {error}"),
    }
    Ok(Some(report))
}

fn sync_pending_and_prune(state: &AppState) -> Result<bool, String> {
    let _cloud_guard = match acquire_cloud_sync_guard(state) {
        Ok(guard) => guard,
        Err(_) => return Ok(false),
    };

    let games = state.repo.list_games().map_err(|error| error.to_string())?;
    let snapshots = AutoBackupService::new(state.snapshots());
    let mut retention_candidates = Vec::new();
    let mut retention_ids = HashSet::new();
    for game in &games {
        for snapshot in snapshots
            .unlocked_retention_candidates(&game.id, MAX_UNLOCKED_SNAPSHOTS_PER_GAME)
            .map_err(|error| error.to_string())?
        {
            retention_ids.insert(snapshot.id.clone());
            retention_candidates.push(snapshot);
        }
    }

    // 删除一旦进入生命周期，即使当前总数后来降到 30 条以内，也必须续做收尾。
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
    let service = BaiduCloudRuntime::from_state(state).build()?;

    let mut cloud_changed = false;
    if connected {
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
}
