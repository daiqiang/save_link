//! Tauri 命令层（薄壳）。
//!
//! 职责：把 savelink-core 的纯逻辑包成前端可调用的命令，并做 DTO 序列化。
//! 不写业务逻辑——业务在 core 里，已被测试焊死。
//!
//! 对照 Java BS：这一层相当于 Spring Controller，core 相当于 Service 层。

use crate::{auto_backup, oauth_config::baidu_oauth_config};
use savelink_core::baidu_oauth::{
    new_oauth_state, BaiduOAuthClient, FileBaiduTokenStore, OAuthCallbackListener,
    RefreshingBaiduTokenProvider,
};
use savelink_core::baidu_store::BaiduNetdiskStore;
use savelink_core::cloud_archive::ZipCloudArchiveCodec;
use savelink_core::cloud_model::{CloudAccount, CloudSnapshotRecord};
use savelink_core::cloud_repo::CloudStateRepository;
use savelink_core::cloud_service::{
    CloudSnapshotDiscovery, CloudSyncError, CloudSyncService, ReceiveOutcome, UploadOutcome,
};
use savelink_core::model::{CreateOutcome, Game, MissingDirChoice, Reason, Snapshot};
use savelink_core::repo::{Clock, IdGen, Repository};
use savelink_core::service::{RestoreService, SnapshotService};
use savelink_core::sqlite_repo::SqliteRepo;
use savelink_core::steam_discovery::{SteamDiscoveredGame, SteamDiscoveryService};
use savelink_core::store::{FsStore, SnapshotStore};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri_plugin_opener::OpenerExt;

pub(crate) const BAIDU_ACCOUNT_ID: &str = "baidu-netdisk";
const BAIDU_PROVIDER: &str = "baidu_netdisk";
const BAIDU_TOKEN_REF: &str = "credentials/baidu-oauth.json";
const BAIDU_CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);
const DEVICE_ID_SETTING: &str = "device_id";
const REPOSITORY_ID_SETTING: &str = "repository_id";

/// 真实时钟：持久化固定秒精度的 UTC RFC 3339；前端按本地时区展示。
struct SystemClock;
impl Clock for SystemClock {
    fn now_stamp(&self) -> String {
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }
}

/// 真实 ID 生成：时间戳 + 计数，避免碰撞。
struct TimeIdGen {
    counter: std::sync::atomic::AtomicU64,
}
impl IdGen for TimeIdGen {
    fn new_id(&self, prefix: &str) -> String {
        use std::sync::atomic::Ordering;
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("{prefix}_{nanos}_{n}")
    }
}

/// 应用全局状态（类似 Spring 单例 Bean）：共享一份 repo / store / clock / ids。
pub struct AppState {
    pub sqlite_repo: Arc<SqliteRepo>,
    pub repo: Arc<dyn Repository>,
    pub cloud_repo: Arc<dyn CloudStateRepository>,
    pub store: Arc<dyn SnapshotStore>,
    pub baidu_token_store: FileBaiduTokenStore,
    pub baidu_auth_in_progress: Arc<AtomicBool>,
    pub baidu_sync_in_progress: Arc<AtomicBool>,
    pub device_id: String,
    pub cloud_repository_id: String,
    pub profile_label: Option<String>,
    pub data_dir: PathBuf,
    pub repository_dir: PathBuf,
    pub clock: Arc<dyn Clock>,
    pub ids: Arc<dyn IdGen>,
    pub snapshot_operation_lock: Arc<Mutex<()>>,
}

impl AppState {
    /// 在指定数据目录下初始化：savelink.db + repository/ 仓库目录。
    pub fn init(data_dir: &std::path::Path) -> Result<Self, String> {
        Self::init_with_profile(data_dir, None)
    }

    pub fn init_with_profile(
        data_dir: &std::path::Path,
        profile_label: Option<String>,
    ) -> Result<Self, String> {
        std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        let sqlite_repo =
            Arc::new(SqliteRepo::open(data_dir.join("savelink.db")).map_err(|e| e.to_string())?);
        let device_id = ensure_local_cloud_id(&sqlite_repo, DEVICE_ID_SETTING, "device")?;
        let cloud_repository_id =
            ensure_local_cloud_id(&sqlite_repo, REPOSITORY_ID_SETTING, "repo")?;
        let repo: Arc<dyn Repository> = sqlite_repo.clone();
        let cloud_repo: Arc<dyn CloudStateRepository> = sqlite_repo.clone();
        let repository_dir = data_dir.join("repository");
        let store = FsStore::new(repository_dir.clone());
        auto_backup::ensure_default(&cloud_repo)?;
        Ok(Self {
            sqlite_repo,
            repo,
            cloud_repo,
            store: Arc::new(store),
            baidu_token_store: FileBaiduTokenStore::new(data_dir.join(BAIDU_TOKEN_REF)),
            baidu_auth_in_progress: Arc::new(AtomicBool::new(false)),
            baidu_sync_in_progress: Arc::new(AtomicBool::new(false)),
            device_id,
            cloud_repository_id,
            profile_label,
            data_dir: data_dir.to_path_buf(),
            repository_dir,
            clock: Arc::new(SystemClock),
            ids: Arc::new(TimeIdGen {
                counter: std::sync::atomic::AtomicU64::new(0),
            }),
            snapshot_operation_lock: Arc::new(Mutex::new(())),
        })
    }

    pub(crate) fn snapshots(&self) -> SnapshotService {
        SnapshotService::new(
            self.repo.clone(),
            self.store.clone(),
            self.clock.clone(),
            self.ids.clone(),
        )
    }

    fn restorer(&self) -> RestoreService {
        RestoreService::new(
            self.repo.clone(),
            self.store.clone(),
            self.clock.clone(),
            self.ids.clone(),
        )
    }
}

fn acquire_snapshot_operation_guard(state: &AppState) -> Result<MutexGuard<'_, ()>, String> {
    state
        .snapshot_operation_lock
        .lock()
        .map_err(|_| "快照操作锁已损坏".to_string())
}

fn ensure_local_cloud_id(repo: &SqliteRepo, setting: &str, prefix: &str) -> Result<String, String> {
    if let Some(existing) = repo
        .get_setting(setting)
        .map_err(|error| error.to_string())?
    {
        savelink_core::cloud_protocol::validate_id(&existing, setting)
            .map_err(|error| error.to_string())?;
        return Ok(existing);
    }
    let random = new_oauth_state().map_err(|error| error.to_string())?;
    let value = format!("{prefix}_{random}");
    repo.set_setting(setting, &value)
        .map_err(|error| error.to_string())?;
    Ok(value)
}

/* ---------- DTO：前端契约（对应架构文档 lib/types.ts） ---------- */

#[derive(Serialize, Deserialize)]
pub struct GameDto {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub save_paths: Vec<String>,
    pub snapshot_count: usize,
    pub last_snapshot_at: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct SnapshotDto {
    pub id: String,
    pub game_id: String,
    pub created_at: String,
    pub note: Option<String>,
    pub reason: String,
    pub locked: bool,
    pub file_count: u64,
    pub total_size: u64,
    pub source_count: u32,
    pub cloud_status: Option<String>,
    pub cloud_error_code: Option<String>,
}

#[derive(Serialize)]
pub struct AppInfoDto {
    pub version: String,
    pub data_dir: String,
    pub repository_dir: String,
    pub database_path: String,
    pub profile_label: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AutoBackupSettingsDto {
    pub enabled: bool,
    pub interval_minutes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BaiduConnectionDto {
    pub connected: bool,
    pub provider: String,
    pub display_name: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CloudUploadDto {
    pub snapshot_id: String,
    pub outcome: String,
    pub cloud_status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CloudSnapshotDto {
    pub cloud_game_id: String,
    pub game_name: String,
    pub snapshot_id: String,
    pub created_at: String,
    pub note: Option<String>,
    pub reason: String,
    pub locked: bool,
    pub file_count: u64,
    pub total_size: u64,
    pub source_count: u32,
    pub cloud_status: String,
    pub last_error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CloudReceiveDto {
    pub snapshot_id: String,
    pub game_id: String,
    pub outcome: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SteamDiscoveredGameDto {
    pub name: String,
    pub steam_name: String,
    pub app_id: u32,
    pub install_dir: String,
    pub save_paths: Vec<String>,
    pub config_paths: Vec<String>,
    pub current_system_unresolved_rules: usize,
    pub other_environment_rules: usize,
    pub already_added: bool,
    /// 至少命中一个真实存档目录时即可直接添加；多目录会作为一个完整游戏共同保护。
    pub can_add_directly: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SteamDiscoveryReportDto {
    pub steam_root: String,
    pub library_count: usize,
    pub registered_app_count: usize,
    pub manifest_match_count: usize,
    pub games: Vec<SteamDiscoveredGameDto>,
}

fn reason_str(r: Reason) -> String {
    match r {
        Reason::Manual => "manual",
        Reason::BeforeRestore => "before_restore",
        Reason::Auto => "auto",
    }
    .to_string()
}

fn cloud_snapshot_to_dto(discovery: CloudSnapshotDiscovery) -> CloudSnapshotDto {
    let record = discovery.snapshot;
    CloudSnapshotDto {
        cloud_game_id: discovery.cloud_game_id,
        game_name: discovery.game_name,
        snapshot_id: record.snapshot_id,
        created_at: record.created_at,
        note: record.note,
        reason: reason_str(record.reason),
        locked: record.locked,
        file_count: record.file_count,
        total_size: record.total_size,
        source_count: record.source_count,
        cloud_status: record.sync_status.as_str().into(),
        last_error_code: record.last_error_code,
    }
}

fn snapshot_to_dto(s: &Snapshot) -> SnapshotDto {
    snapshot_to_dto_with_cloud(s, None)
}

fn snapshot_to_dto_with_cloud(s: &Snapshot, cloud: Option<&CloudSnapshotRecord>) -> SnapshotDto {
    SnapshotDto {
        id: s.id.clone(),
        game_id: s.game_id.clone(),
        created_at: s.created_at.clone(),
        note: s.note.clone(),
        reason: reason_str(s.reason),
        locked: s.locked,
        file_count: s.file_count,
        total_size: s.total_size,
        source_count: s.source_count,
        cloud_status: cloud.map(|record| record.sync_status.as_str().to_string()),
        cloud_error_code: cloud.and_then(|record| record.last_error_code.clone()),
    }
}

fn game_to_dto(repo: &Arc<dyn Repository>, g: &Game) -> GameDto {
    let snaps = repo.list_snapshots(&g.id).unwrap_or_default();
    GameDto {
        id: g.id.clone(),
        name: g.name.clone(),
        icon: g.icon.clone(),
        save_paths: g
            .save_paths
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
        snapshot_count: snaps.len(),
        last_snapshot_at: snaps.first().map(|s| s.created_at.clone()),
    }
}

pub(crate) fn baidu_connection_status(
    cloud_repo: &Arc<dyn CloudStateRepository>,
    token_store: &FileBaiduTokenStore,
) -> Result<BaiduConnectionDto, String> {
    let account = cloud_repo
        .get_cloud_account(BAIDU_ACCOUNT_ID)
        .map_err(|error| error.to_string())?;
    let token = token_store.load().map_err(|error| error.to_string())?;
    let connected = token.as_ref().is_some_and(|token| {
        !token.access_token.trim().is_empty()
            && (!token.expires_within(Duration::ZERO)
                || token
                    .refresh_token
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty()))
    });
    Ok(BaiduConnectionDto {
        connected,
        provider: BAIDU_PROVIDER.into(),
        display_name: account.and_then(|account| account.display_name),
        expires_at: token.and_then(|token| token.expires_at),
    })
}

pub(crate) struct AuthBusyGuard(Arc<AtomicBool>);

impl Drop for AuthBusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

pub(crate) struct BaiduCloudRuntime {
    sqlite_repo: Arc<SqliteRepo>,
    snapshot_store: Arc<dyn SnapshotStore>,
    token_store: FileBaiduTokenStore,
    clock: Arc<dyn Clock>,
    work_dir: PathBuf,
    device_id: String,
    repository_id: String,
}

impl BaiduCloudRuntime {
    pub(crate) fn from_state(state: &AppState) -> Self {
        Self {
            sqlite_repo: state.sqlite_repo.clone(),
            snapshot_store: state.store.clone(),
            token_store: state.baidu_token_store.clone(),
            clock: state.clock.clone(),
            work_dir: state.data_dir.join("cloud-work"),
            device_id: state.device_id.clone(),
            repository_id: state.cloud_repository_id.clone(),
        }
    }

    pub(crate) fn build(self) -> Result<CloudSyncService<SqliteRepo>, String> {
        let config = baidu_oauth_config().map_err(|error| error.to_string())?;
        let oauth_client = BaiduOAuthClient::new(config).map_err(|error| error.to_string())?;
        let token_provider = Arc::new(RefreshingBaiduTokenProvider::new(
            self.token_store,
            oauth_client,
        ));
        let cloud_store =
            Arc::new(BaiduNetdiskStore::new(token_provider).map_err(|error| error.to_string())?);
        CloudSyncService::new(
            self.sqlite_repo,
            self.snapshot_store,
            cloud_store,
            Arc::new(ZipCloudArchiveCodec::new()),
            self.clock,
            self.work_dir,
            BAIDU_ACCOUNT_ID,
            self.device_id,
            self.repository_id,
        )
        .map_err(|error| error.to_string())
    }
}

pub(crate) fn acquire_cloud_sync_guard(state: &AppState) -> Result<AuthBusyGuard, String> {
    let busy = state.baidu_sync_in_progress.clone();
    if busy
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("已有云端任务正在进行，请等待当前任务完成".into());
    }
    Ok(AuthBusyGuard(busy))
}

/* ---------- Tauri 命令（前端通过 invoke 调用，类似发 HTTP 到 Controller） ---------- */

use tauri::{path::BaseDirectory, AppHandle, Manager, State};

#[tauri::command]
pub fn list_games(state: State<'_, AppState>) -> Result<Vec<GameDto>, String> {
    let games = state.repo.list_games().map_err(|e| e.to_string())?;
    Ok(games.iter().map(|g| game_to_dto(&state.repo, g)).collect())
}

#[tauri::command]
pub fn get_repository_path(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state.repository_dir.to_string_lossy().to_string())
}

#[tauri::command]
pub fn get_app_info(state: State<'_, AppState>) -> Result<AppInfoDto, String> {
    Ok(AppInfoDto {
        version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir: state.data_dir.to_string_lossy().to_string(),
        repository_dir: state.repository_dir.to_string_lossy().to_string(),
        database_path: state
            .data_dir
            .join("savelink.db")
            .to_string_lossy()
            .to_string(),
        profile_label: state.profile_label.clone(),
    })
}

#[tauri::command]
pub fn get_auto_backup_settings(
    state: State<'_, AppState>,
) -> Result<AutoBackupSettingsDto, String> {
    Ok(AutoBackupSettingsDto {
        enabled: auto_backup::enabled(&state.cloud_repo)?,
        interval_minutes: auto_backup::AUTO_BACKUP_INTERVAL_MINUTES,
    })
}

#[tauri::command]
pub fn set_auto_backup_enabled(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<AutoBackupSettingsDto, String> {
    auto_backup::set_enabled(&state.cloud_repo, enabled)?;
    if enabled {
        auto_backup::trigger(app);
    }
    Ok(AutoBackupSettingsDto {
        enabled,
        interval_minutes: auto_backup::AUTO_BACKUP_INTERVAL_MINUTES,
    })
}

#[tauri::command]
pub fn get_baidu_connection_status(
    state: State<'_, AppState>,
) -> Result<BaiduConnectionDto, String> {
    baidu_connection_status(&state.cloud_repo, &state.baidu_token_store)
}

#[tauri::command]
pub async fn connect_baidu(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BaiduConnectionDto, String> {
    let current = baidu_connection_status(&state.cloud_repo, &state.baidu_token_store)?;
    if current.connected {
        return Ok(current);
    }

    let busy = state.baidu_auth_in_progress.clone();
    if busy
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("百度网盘授权正在进行，请在浏览器中完成授权".into());
    }
    let _busy_guard = AuthBusyGuard(busy);

    let config = baidu_oauth_config().map_err(|error| error.to_string())?;
    let client = BaiduOAuthClient::new(config.clone()).map_err(|error| error.to_string())?;
    let oauth_state = new_oauth_state().map_err(|error| error.to_string())?;
    let callback =
        OAuthCallbackListener::bind(config.redirect_uri()).map_err(|error| error.to_string())?;
    let authorization_url = client
        .authorization_url(&oauth_state)
        .map_err(|error| error.to_string())?;

    app.opener()
        .open_url(authorization_url, None::<&str>)
        .map_err(|error| format!("无法打开百度授权页面: {error}"))?;

    let cloud_repo = state.cloud_repo.clone();
    let token_store = state.baidu_token_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let code = callback
            .wait_for_code(&oauth_state, BAIDU_CALLBACK_TIMEOUT)
            .map_err(|error| error.to_string())?;
        let token = client
            .exchange_code(&code)
            .map_err(|error| error.to_string())?;
        token_store
            .save(&token)
            .map_err(|error| error.to_string())?;

        let now = chrono::Utc::now().to_rfc3339();
        let created_at = cloud_repo
            .get_cloud_account(BAIDU_ACCOUNT_ID)
            .map_err(|error| error.to_string())?
            .map(|account| account.created_at)
            .unwrap_or_else(|| now.clone());
        cloud_repo
            .upsert_cloud_account(CloudAccount {
                id: BAIDU_ACCOUNT_ID.into(),
                provider: BAIDU_PROVIDER.into(),
                account_identity: None,
                display_name: Some("百度网盘".into()),
                token_ref: Some(BAIDU_TOKEN_REF.into()),
                created_at,
                updated_at: now,
            })
            .map_err(|error| error.to_string())?;

        baidu_connection_status(&cloud_repo, &token_store)
    })
    .await
    .map_err(|error| format!("百度授权任务异常结束: {error}"))?
}

#[tauri::command]
pub async fn upload_snapshot_to_baidu(
    state: State<'_, AppState>,
    game_id: String,
    snapshot_id: String,
) -> Result<CloudUploadDto, String> {
    let _busy_guard = acquire_cloud_sync_guard(&state)?;
    let runtime = BaiduCloudRuntime::from_state(&state);
    let token_store_on_error = state.baidu_token_store.clone();
    let requested_snapshot_id = snapshot_id.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let service = runtime.build()?;

        match service.upload_snapshot(&game_id, &snapshot_id) {
            Ok(outcome) => Ok(CloudUploadDto {
                snapshot_id,
                outcome: match outcome {
                    UploadOutcome::Uploaded => "uploaded",
                    UploadOutcome::AlreadyPresent => "already_present",
                }
                .into(),
                cloud_status: "uploaded".into(),
            }),
            Err(error) => {
                if error.code() == "auth_required" {
                    let _ = token_store_on_error.clear();
                }
                Err(cloud_upload_error_message(&error))
            }
        }
    })
    .await
    .map_err(|error| format!("快照上传任务异常结束: {error}"))?
    .map_err(|error| {
        if error.trim().is_empty() {
            format!("快照 {requested_snapshot_id} 上传失败")
        } else {
            error
        }
    })
}

#[tauri::command]
pub async fn discover_baidu_snapshots(
    state: State<'_, AppState>,
) -> Result<Vec<CloudSnapshotDto>, String> {
    let _busy_guard = acquire_cloud_sync_guard(&state)?;
    let runtime = BaiduCloudRuntime::from_state(&state);
    let token_store_on_error = state.baidu_token_store.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let service = runtime.build()?;
        match service.discover_remote_catalog() {
            Ok(discovered) => Ok(discovered.into_iter().map(cloud_snapshot_to_dto).collect()),
            Err(error) => {
                if error.code() == "auth_required" {
                    let _ = token_store_on_error.clear();
                }
                Err(cloud_discovery_error_message(&error))
            }
        }
    })
    .await
    .map_err(|error| format!("云端存档发现任务异常结束: {error}"))?
}

#[tauri::command]
pub async fn receive_baidu_snapshot(
    state: State<'_, AppState>,
    snapshot_id: String,
) -> Result<CloudReceiveDto, String> {
    let _busy_guard = acquire_cloud_sync_guard(&state)?;
    let runtime = BaiduCloudRuntime::from_state(&state);
    let repo = runtime.sqlite_repo.clone();
    let token_store_on_error = state.baidu_token_store.clone();
    let snapshot_operation_lock = state.snapshot_operation_lock.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let _operation = snapshot_operation_lock
            .lock()
            .map_err(|_| "快照操作锁已损坏".to_string())?;
        let cloud_game_id = repo
            .get_cloud_snapshot(BAIDU_ACCOUNT_ID, &snapshot_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "尚未发现这条云端快照，请先刷新云端存档列表".to_string())?
            .cloud_game_id;
        let service = runtime.build()?;
        match service.receive_remote_snapshot(&snapshot_id) {
            Ok(outcome) => Ok(CloudReceiveDto {
                snapshot_id,
                game_id: cloud_game_id,
                outcome: match outcome {
                    ReceiveOutcome::Downloaded => "downloaded",
                    ReceiveOutcome::AlreadyPresent => "already_present",
                }
                .into(),
            }),
            Err(error) => {
                if error.code() == "auth_required" {
                    let _ = token_store_on_error.clear();
                }
                Err(cloud_receive_error_message(&error))
            }
        }
    })
    .await
    .map_err(|error| format!("云端快照下载任务异常结束: {error}"))?
}

fn cloud_upload_error_message(error: &CloudSyncError) -> String {
    match error.code() {
        "auth_required" => "百度网盘授权已失效，请重新授权后再上传".into(),
        "network_unavailable" => "无法连接百度网盘，请检查网络后重试".into(),
        "rate_limited" => "百度网盘请求过于频繁，请稍后重试".into(),
        "snapshot_id_conflict" => "云端存在同编号但内容不同的快照，已停止上传".into(),
        "snapshot_content_mismatch" | "local_store_failed" => {
            "本机快照校验失败，未上传任何发布记录".into()
        }
        "archive_size_mismatch" | "archive_hash_mismatch" => {
            "云端快照文件校验失败，请稍后重试".into()
        }
        _ => format!("快照上传失败：{error}"),
    }
}

fn cloud_discovery_error_message(error: &CloudSyncError) -> String {
    match error.code() {
        "auth_required" => "百度网盘授权已失效，请重新授权后刷新".into(),
        "network_unavailable" => "无法连接百度网盘，请检查网络后刷新".into(),
        "rate_limited" => "百度网盘请求过于频繁，请稍后刷新".into(),
        "snapshot_id_conflict" => "云端存在相互冲突的快照记录，已停止读取".into(),
        "remote_zip_missing" => "云端快照缺少 zip 文件，未加入可下载列表".into(),
        _ => format!("读取云端存档失败：{error}"),
    }
}

fn cloud_receive_error_message(error: &CloudSyncError) -> String {
    match error.code() {
        "auth_required" => "百度网盘授权已失效，请重新授权后下载".into(),
        "network_unavailable" => "无法连接百度网盘，请检查网络后重试".into(),
        "rate_limited" => "百度网盘请求过于频繁，请稍后重试".into(),
        "archive_size_mismatch" | "archive_hash_mismatch" | "snapshot_content_mismatch" => {
            "下载的云端快照校验失败，未写入本机快照仓库".into()
        }
        "snapshot_id_conflict" => "本机存在同编号但内容不同的快照，已停止下载".into(),
        "remote_zip_missing" => "云端快照文件不完整，无法下载".into(),
        _ => format!("下载云端存档失败：{error}"),
    }
}

#[tauri::command]
pub fn list_snapshots(
    state: State<'_, AppState>,
    game_id: String,
) -> Result<Vec<SnapshotDto>, String> {
    let snaps = state
        .repo
        .list_snapshots(&game_id)
        .map_err(|e| e.to_string())?;
    snaps
        .iter()
        .map(|snapshot| {
            let cloud = state
                .cloud_repo
                .get_cloud_snapshot(BAIDU_ACCOUNT_ID, &snapshot.id)
                .map_err(|error| error.to_string())?;
            Ok(snapshot_to_dto_with_cloud(snapshot, cloud.as_ref()))
        })
        .collect()
}

#[tauri::command]
pub fn scan_path(path: String) -> Result<SnapshotDto, String> {
    // "测试读取"：返回文件数/大小的轻量探测。复用 scan，结果塞进 DTO 的相应字段。
    let p = std::path::PathBuf::from(&path);
    let r = savelink_core::scan::scan(&[p]).map_err(|e| e.to_string())?;
    Ok(SnapshotDto {
        id: String::new(),
        game_id: String::new(),
        created_at: String::new(),
        note: None,
        reason: "scan".into(),
        locked: false,
        file_count: r.file_count,
        total_size: r.total_size,
        source_count: 1,
        cloud_status: None,
        cloud_error_code: None,
    })
}

#[tauri::command]
pub fn scan_steam_games(
    app: AppHandle,
    state: State<'_, AppState>,
    steam_root: Option<String>,
) -> Result<SteamDiscoveryReportDto, String> {
    let database = resolve_manifest_database(&app)?;
    let explicit_root = steam_root
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let report = SteamDiscoveryService::new(database)
        .scan(explicit_root.as_deref())
        .map_err(|error| error.to_string())?;
    let existing = state.repo.list_games().map_err(|error| error.to_string())?;
    let games = report
        .games
        .into_iter()
        .map(|game| steam_game_to_dto(game, &existing))
        .collect();
    Ok(SteamDiscoveryReportDto {
        steam_root: report.steam_root.to_string_lossy().to_string(),
        library_count: report.library_count,
        registered_app_count: report.registered_app_count,
        manifest_match_count: report.manifest_match_count,
        games,
    })
}

fn resolve_manifest_database(app: &AppHandle) -> Result<PathBuf, String> {
    let bundled = app
        .path()
        .resolve("resources/manifest.db", BaseDirectory::Resource)
        .map_err(|error| format!("无法定位存档规则库：{error}"))?;
    if bundled.is_file() {
        return Ok(bundled);
    }

    // `cargo run`/部分 IDE 调试方式不会先布置 bundle resources。
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/manifest.db");
    if source.is_file() {
        return Ok(source);
    }
    Err(format!("存档规则库不存在：{}", bundled.display()))
}

fn steam_game_to_dto(game: SteamDiscoveredGame, existing: &[Game]) -> SteamDiscoveredGameDto {
    let discovered_paths = game
        .save_paths
        .iter()
        .map(|path| normalized_path(path))
        .collect::<Vec<_>>();
    let already_added = existing.iter().any(|saved| {
        saved
            .save_paths
            .iter()
            .map(|path| normalized_path(path))
            .any(|path| discovered_paths.contains(&path))
    });
    SteamDiscoveredGameDto {
        name: game.name,
        steam_name: game.steam_name,
        app_id: game.app_id,
        install_dir: game.install_dir.to_string_lossy().to_string(),
        save_paths: game
            .save_paths
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect(),
        config_paths: game
            .config_paths
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect(),
        current_system_unresolved_rules: game.current_system_unresolved_rules,
        other_environment_rules: game.other_environment_rules,
        already_added,
        can_add_directly: !game.save_paths.is_empty(),
    }
}

fn normalized_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

#[tauri::command]
pub fn add_game(
    state: State<'_, AppState>,
    name: String,
    save_paths: Vec<String>,
) -> Result<GameDto, String> {
    if name.trim().is_empty() {
        return Err("游戏名称不能为空".into());
    }
    if save_paths.is_empty() {
        return Err("请至少选择一个存档目录".into());
    }
    let save_paths = save_paths
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .map(std::path::PathBuf::from)
        .collect::<Vec<_>>();
    if save_paths.is_empty() {
        return Err("请至少选择一个存档目录".into());
    }
    savelink_core::scan::validate_save_paths(&save_paths).map_err(|e| e.to_string())?;
    let _operation = acquire_snapshot_operation_guard(&state)?;
    let now = state.clock.now_stamp();
    let game = Game {
        id: state.ids.new_id("game"),
        name,
        icon: None,
        repo_path: std::path::PathBuf::new(), // 仓库由 store 管理，DTO 不暴露
        save_paths,
        created_at: now.clone(),
        updated_at: now,
    };
    state
        .repo
        .insert_game(game.clone())
        .map_err(|e| e.to_string())?;
    Ok(game_to_dto(&state.repo, &game))
}

#[tauri::command]
pub fn update_game(
    state: State<'_, AppState>,
    game_id: String,
    name: String,
    save_paths: Vec<String>,
) -> Result<GameDto, String> {
    if name.trim().is_empty() {
        return Err("游戏名称不能为空".into());
    }
    if save_paths.is_empty() {
        return Err("请至少选择一个存档目录".into());
    }
    let trimmed_paths: Vec<String> = save_paths
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if trimmed_paths.is_empty() {
        return Err("请至少选择一个存档目录".into());
    }
    let trimmed_paths = trimmed_paths
        .into_iter()
        .map(std::path::PathBuf::from)
        .collect::<Vec<_>>();
    savelink_core::scan::validate_save_paths(&trimmed_paths).map_err(|e| e.to_string())?;
    let _operation = acquire_snapshot_operation_guard(&state)?;

    let mut game = state
        .repo
        .get_game(&game_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "游戏不存在".to_string())?;
    game.name = name.trim().to_string();
    game.save_paths = trimmed_paths;
    game.updated_at = state.clock.now_stamp();
    state
        .repo
        .update_game(game.clone())
        .map_err(|e| e.to_string())?;
    Ok(game_to_dto(&state.repo, &game))
}

#[tauri::command]
pub fn create_snapshot(
    state: State<'_, AppState>,
    game_id: String,
    note: Option<String>,
) -> Result<Option<SnapshotDto>, String> {
    let _operation = acquire_snapshot_operation_guard(&state)?;
    match state
        .snapshots()
        .create_snapshot(&game_id, note, Reason::Manual)
        .map_err(|e| e.to_string())?
    {
        CreateOutcome::Created(s) => Ok(Some(snapshot_to_dto(&s))),
        CreateOutcome::NoChange => Ok(None), // 前端据此提示"存档未变化"
    }
}

#[tauri::command]
pub fn update_snapshot_meta(
    state: State<'_, AppState>,
    snapshot_id: String,
    note: Option<String>,
    locked: Option<bool>,
) -> Result<(), String> {
    let _operation = acquire_snapshot_operation_guard(&state)?;
    state
        .snapshots()
        .update_meta(&snapshot_id, note, locked)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_snapshot(state: State<'_, AppState>, snapshot_id: String) -> Result<(), String> {
    let _operation = acquire_snapshot_operation_guard(&state)?;
    state
        .snapshots()
        .delete_snapshot(&snapshot_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_game(state: State<'_, AppState>, game_id: String) -> Result<(), String> {
    let _operation = acquire_snapshot_operation_guard(&state)?;
    state
        .snapshots()
        .delete_game(&game_id)
        .map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct RestoreResultDto {
    pub target_id: String,
    pub restored: bool,
}

#[tauri::command]
pub fn restore_snapshot(
    state: State<'_, AppState>,
    game_id: String,
    snapshot_id: String,
) -> Result<RestoreResultDto, String> {
    let _operation = acquire_snapshot_operation_guard(&state)?;
    // 进度事件第 4 步接前端时再接 emit；这里先用空回调，保证链路可调。
    let out = state
        .restorer()
        .restore_snapshot(&game_id, &snapshot_id, &|_step| {})
        .map_err(|e| e.to_string())?;
    Ok(RestoreResultDto {
        target_id: out.target_id,
        restored: out.restored,
    })
}

/// 真实存档目录不存在时、用户已做出决策后的续走命令（安全规则 5）。
/// choice: "create"=创建目录并恢复；"reselect"=重新选择（暂未在 UI 接通）；其它=取消。
#[tauri::command]
pub fn restore_snapshot_with_choice(
    state: State<'_, AppState>,
    game_id: String,
    snapshot_id: String,
    choice: String,
) -> Result<RestoreResultDto, String> {
    let _operation = acquire_snapshot_operation_guard(&state)?;
    let c = match choice.as_str() {
        "create" => MissingDirChoice::CreateAndRestore,
        "reselect" => MissingDirChoice::Reselect,
        _ => MissingDirChoice::Cancel,
    };
    let out = state
        .restorer()
        .restore_with_choice(&game_id, &snapshot_id, c, &|_step| {})
        .map_err(|e| e.to_string())?;
    Ok(RestoreResultDto {
        target_id: out.target_id,
        restored: out.restored,
    })
}
