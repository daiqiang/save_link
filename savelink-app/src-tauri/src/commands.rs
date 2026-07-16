//! Tauri 命令层（薄壳）。
//!
//! 职责：把 savelink-core 的纯逻辑包成前端可调用的命令，并做 DTO 序列化。
//! 不写业务逻辑——业务在 core 里，已被测试焊死。
//!
//! 对照 Java BS：这一层相当于 Spring Controller，core 相当于 Service 层。

use crate::oauth_config::baidu_oauth_config;
use savelink_core::baidu_oauth::{
    new_oauth_state, BaiduOAuthClient, FileBaiduTokenStore, OAuthCallbackListener,
    RefreshingBaiduTokenProvider,
};
use savelink_core::baidu_store::BaiduNetdiskStore;
use savelink_core::cloud_archive::ZipCloudArchiveCodec;
use savelink_core::cloud_model::{CloudAccount, CloudSnapshotRecord};
use savelink_core::cloud_repo::CloudStateRepository;
use savelink_core::cloud_service::{CloudSyncError, CloudSyncService, UploadOutcome};
use savelink_core::model::{CreateOutcome, Game, MissingDirChoice, Reason, Snapshot};
use savelink_core::repo::{Clock, IdGen, Repository};
use savelink_core::service::{RestoreService, SnapshotService};
use savelink_core::sqlite_repo::SqliteRepo;
use savelink_core::store::{FsStore, SnapshotStore};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri_plugin_opener::OpenerExt;

const BAIDU_ACCOUNT_ID: &str = "baidu-netdisk";
const BAIDU_PROVIDER: &str = "baidu_netdisk";
const BAIDU_TOKEN_REF: &str = "credentials/baidu-oauth.json";
const BAIDU_CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);
const DEVICE_ID_SETTING: &str = "device_id";
const REPOSITORY_ID_SETTING: &str = "repository_id";

/// 真实时钟：输出本地时间 "YYYY-MM-DD HH:MM"。
/// 该格式既能按字符串正确排序（时间线倒序依赖此），又便于前端直接展示。
struct SystemClock;
impl Clock for SystemClock {
    fn now_stamp(&self) -> String {
        chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
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
    pub baidu_upload_in_progress: Arc<AtomicBool>,
    pub device_id: String,
    pub cloud_repository_id: String,
    pub data_dir: PathBuf,
    pub repository_dir: PathBuf,
    pub clock: Arc<dyn Clock>,
    pub ids: Arc<dyn IdGen>,
}

impl AppState {
    /// 在指定数据目录下初始化：savelink.db + repository/ 仓库目录。
    pub fn init(data_dir: &std::path::Path) -> Result<Self, String> {
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
        Ok(Self {
            sqlite_repo,
            repo,
            cloud_repo,
            store: Arc::new(store),
            baidu_token_store: FileBaiduTokenStore::new(data_dir.join(BAIDU_TOKEN_REF)),
            baidu_auth_in_progress: Arc::new(AtomicBool::new(false)),
            baidu_upload_in_progress: Arc::new(AtomicBool::new(false)),
            device_id,
            cloud_repository_id,
            data_dir: data_dir.to_path_buf(),
            repository_dir,
            clock: Arc::new(SystemClock),
            ids: Arc::new(TimeIdGen {
                counter: std::sync::atomic::AtomicU64::new(0),
            }),
        })
    }

    fn snapshots(&self) -> SnapshotService {
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
    pub cloud_status: Option<String>,
    pub cloud_error_code: Option<String>,
}

#[derive(Serialize)]
pub struct AppInfoDto {
    pub version: String,
    pub data_dir: String,
    pub repository_dir: String,
    pub database_path: String,
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

fn reason_str(r: Reason) -> String {
    match r {
        Reason::Manual => "manual",
        Reason::BeforeRestore => "before_restore",
        Reason::Auto => "auto",
    }
    .to_string()
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

fn baidu_connection_status(
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

struct AuthBusyGuard(Arc<AtomicBool>);

impl Drop for AuthBusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/* ---------- Tauri 命令（前端通过 invoke 调用，类似发 HTTP 到 Controller） ---------- */

use tauri::{AppHandle, State};

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
    let busy = state.baidu_upload_in_progress.clone();
    if busy
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("已有快照正在上传，请等待当前上传完成".into());
    }
    let _busy_guard = AuthBusyGuard(busy);

    let config = baidu_oauth_config().map_err(|error| error.to_string())?;
    let sqlite_repo = state.sqlite_repo.clone();
    let snapshot_store = state.store.clone();
    let token_store = state.baidu_token_store.clone();
    let token_store_on_error = token_store.clone();
    let clock = state.clock.clone();
    let work_dir = state.data_dir.join("cloud-work");
    let account_id = BAIDU_ACCOUNT_ID.to_string();
    let device_id = state.device_id.clone();
    let repository_id = state.cloud_repository_id.clone();
    let requested_snapshot_id = snapshot_id.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let oauth_client = BaiduOAuthClient::new(config).map_err(|error| error.to_string())?;
        let token_provider = Arc::new(RefreshingBaiduTokenProvider::new(token_store, oauth_client));
        let cloud_store =
            Arc::new(BaiduNetdiskStore::new(token_provider).map_err(|error| error.to_string())?);
        let service = CloudSyncService::new(
            sqlite_repo,
            snapshot_store,
            cloud_store,
            Arc::new(ZipCloudArchiveCodec::new()),
            clock,
            work_dir,
            account_id,
            device_id,
            repository_id,
        )
        .map_err(|error| cloud_upload_error_message(&error))?;

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
        cloud_status: None,
        cloud_error_code: None,
    })
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
    let now = state.clock.now_stamp();
    let game = Game {
        id: state.ids.new_id("game"),
        name,
        icon: None,
        repo_path: std::path::PathBuf::new(), // 仓库由 store 管理，DTO 不暴露
        save_paths: save_paths
            .into_iter()
            .map(std::path::PathBuf::from)
            .collect(),
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

    let mut game = state
        .repo
        .get_game(&game_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "游戏不存在".to_string())?;
    game.name = name.trim().to_string();
    game.save_paths = trimmed_paths
        .into_iter()
        .map(std::path::PathBuf::from)
        .collect();
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
    state
        .snapshots()
        .update_meta(&snapshot_id, note, locked)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_snapshot(state: State<'_, AppState>, snapshot_id: String) -> Result<(), String> {
    state
        .snapshots()
        .delete_snapshot(&snapshot_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_game(state: State<'_, AppState>, game_id: String) -> Result<(), String> {
    state
        .snapshots()
        .delete_game(&game_id)
        .map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct RestoreResultDto {
    pub target_id: String,
    pub backup_id: String,
}

#[tauri::command]
pub fn restore_snapshot(
    state: State<'_, AppState>,
    game_id: String,
    snapshot_id: String,
) -> Result<RestoreResultDto, String> {
    // 进度事件第 4 步接前端时再接 emit；这里先用空回调，保证链路可调。
    let out = state
        .restorer()
        .restore_snapshot(&game_id, &snapshot_id, &|_step| {})
        .map_err(|e| e.to_string())?;
    Ok(RestoreResultDto {
        target_id: out.target_id,
        backup_id: out.backup_id,
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
        backup_id: out.backup_id,
    })
}
