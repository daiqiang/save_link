//! Tauri 命令层（薄壳）。
//!
//! 职责：把 savelink-core 的纯逻辑包成前端可调用的命令，并做 DTO 序列化。
//! 不写业务逻辑——业务在 core 里，已被测试焊死。
//!
//! 对照 Java BS：这一层相当于 Spring Controller，core 相当于 Service 层。

use crate::{
    auto_backup,
    oauth_config::baidu_oauth_config,
    save_discovery_runtime::{
        SaveDiscoveryManager, SaveDiscoveryStartRequest, SaveDiscoveryStatus,
    },
};
use savelink_core::baidu_oauth::{
    new_oauth_state, BaiduOAuthClient, FileBaiduTokenStore, OAuthCallbackListener,
    RefreshingBaiduTokenProvider,
};
use savelink_core::baidu_store::BaiduNetdiskStore;
use savelink_core::cloud_archive::ZipCloudArchiveCodec;
use savelink_core::cloud_model::{
    CloudAccount, CloudMetadataSyncStatus, CloudSnapshotMetadataState, CloudSnapshotRecord,
};
use savelink_core::cloud_repo::CloudStateRepository;
use savelink_core::cloud_service::{
    CloudSnapshotDiscovery, CloudSyncError, CloudSyncService, ReceiveOutcome, UploadOutcome,
};
use savelink_core::desmume_discovery::{
    compare_rom_identity, DesmumeDiscoveredGame, DesmumeDiscoveryService, RomMatch,
};
use savelink_core::model::{
    CreateOutcome, Game, GameConfigurationState, GameLaunchBinding, MissingDirChoice, Reason,
    Snapshot, SnapshotDisplayZone,
};
use savelink_core::program_discovery::{
    ProgramDiscoveredGame, ProgramDiscoveryService, ProgramMatchKind, ProgramSelectionKind,
};
use savelink_core::repo::{Clock, IdGen, Repository};
use savelink_core::scan::{path_is_same_or_descendant, save_paths_overlap};
use savelink_core::service::{RestoreService, SnapshotService};
use savelink_core::sqlite_repo::SqliteRepo;
use savelink_core::steam_discovery::{SteamDiscoveredGame, SteamDiscoveryService};
use savelink_core::store::{FsStore, SnapshotStore};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    pub save_discovery: SaveDiscoveryManager,
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
        let has_existing_games = !repo
            .list_games()
            .map_err(|error| error.to_string())?
            .is_empty();
        auto_backup::ensure_default(&cloud_repo, has_existing_games)?;
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
            save_discovery: SaveDiscoveryManager::default(),
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
    pub emulator: Option<String>,
    pub configuration_state: String,
    pub launch_kind: Option<String>,
    pub launch_executable_path: Option<String>,
    pub launch_arguments: Vec<String>,
    pub steam_app_id: Option<u32>,
    pub install_dir: Option<String>,
    pub rom_path: Option<String>,
    pub snapshot_count: usize,
    pub last_snapshot_at: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct SnapshotDto {
    pub id: String,
    pub game_id: String,
    pub created_at: String,
    pub note: Option<String>,
    pub display_name: String,
    pub display_zone: String,
    pub pending_reorganization: bool,
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
    pub retention_limit: usize,
    pub retention_policy_confirmed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FirstBackupOutcomeDto {
    Disabled,
    Created,
    NoChange,
    Failed,
}

#[derive(Serialize)]
pub struct ConfirmSaveDiscoveryPathsDto {
    pub game: GameDto,
    pub first_backup: FirstBackupOutcomeDto,
    pub snapshot: Option<SnapshotDto>,
    pub backup_error: Option<String>,
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
    pub existing_game_id: Option<String>,
    pub existing_game_name: Option<String>,
    pub can_bind_existing_launch: bool,
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

#[derive(Debug, Clone, Serialize)]
pub struct ProgramDiscoveredGameDto {
    pub name: String,
    pub app_id: u32,
    pub match_kind: String,
    pub save_paths: Vec<String>,
    pub config_paths: Vec<String>,
    pub current_system_unresolved_rules: usize,
    pub other_environment_rules: usize,
    pub already_added: bool,
    pub can_add_directly: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgramDiscoveryReportDto {
    pub selected_path: String,
    pub selection_kind: String,
    pub resolved_program_path: Option<String>,
    pub install_dir: String,
    pub detected_app_id: Option<u32>,
    pub app_id_source: Option<String>,
    pub ignored_app_id_game_names: Vec<String>,
    pub identity_hints: Vec<String>,
    pub suggested_name: String,
    pub program_already_added: bool,
    pub existing_game_id: Option<String>,
    pub existing_game_name: Option<String>,
    pub can_bind_existing_launch: bool,
    pub games: Vec<ProgramDiscoveredGameDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DesmumeGameMatchDto {
    pub game_id: String,
    pub game_name: String,
    pub match_kind: String,
    pub already_bound_here: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DesmumeDiscoveredGameDto {
    pub name: String,
    pub rom_path: String,
    pub save_path: String,
    pub has_save: bool,
    pub rom_sha256: String,
    pub rom_header_title: String,
    pub rom_game_code: String,
    pub matches: Vec<DesmumeGameMatchDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DesmumeDiscoveryReportDto {
    pub emulator_root: String,
    pub configured_rom_root: Option<String>,
    pub rom_root: Option<String>,
    pub configured_rom_root_missing: bool,
    pub battery_dir: String,
    pub games: Vec<DesmumeDiscoveredGameDto>,
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
    snapshot_to_dto_with_view(
        s,
        None,
        s.note.clone().unwrap_or_else(|| "未命名快照".into()),
    )
}

fn snapshot_to_dto_with_view(
    s: &Snapshot,
    cloud: Option<&CloudSnapshotRecord>,
    display_name: String,
) -> SnapshotDto {
    SnapshotDto {
        id: s.id.clone(),
        game_id: s.game_id.clone(),
        created_at: s.created_at.clone(),
        note: s.note.clone(),
        display_name,
        display_zone: match s.display_zone {
            SnapshotDisplayZone::Normal => "normal",
            SnapshotDisplayZone::Locked => "locked",
        }
        .into(),
        pending_reorganization: s.display_zone.is_pending(s.locked),
        reason: reason_str(s.reason),
        locked: s.locked,
        file_count: s.file_count,
        total_size: s.total_size,
        source_count: s.source_count,
        cloud_status: cloud.map(|record| record.sync_status.as_str().to_string()),
        cloud_error_code: cloud.and_then(|record| record.last_error_code.clone()),
    }
}

fn snapshot_display_names(snapshots: &[Snapshot]) -> HashMap<String, String> {
    let mut normal_number = 0usize;
    let mut locked_number = 0usize;
    let mut names = HashMap::with_capacity(snapshots.len());
    for snapshot in snapshots {
        let name = match snapshot.display_zone {
            SnapshotDisplayZone::Normal => {
                normal_number += 1;
                format!("存档{normal_number}")
            }
            SnapshotDisplayZone::Locked => {
                locked_number += 1;
                format!("锁定存档{locked_number}")
            }
        };
        names.insert(snapshot.id.clone(), snapshot.note.clone().unwrap_or(name));
    }
    names
}

#[cfg(test)]
mod snapshot_display_tests {
    use super::*;
    use savelink_core::model::{Reason, SnapshotStatus};

    fn snapshot(id: &str, zone: SnapshotDisplayZone, note: Option<&str>) -> Snapshot {
        Snapshot {
            id: id.into(),
            game_id: "game".into(),
            created_at: id.into(),
            note: note.map(str::to_string),
            note_updated_at: "2026-08-29T00:00:00Z".into(),
            reason: Reason::Manual,
            locked: zone == SnapshotDisplayZone::Locked,
            locked_updated_at: "2026-08-29T00:00:00Z".into(),
            display_zone: zone,
            file_count: 1,
            total_size: 1,
            source_count: 1,
            content_hash: id.into(),
            storage_key: id.into(),
            status: SnapshotStatus::Complete,
        }
    }

    #[test]
    fn automatic_names_are_numbered_separately_by_display_zone() {
        let snapshots = vec![
            snapshot("locked-new", SnapshotDisplayZone::Locked, None),
            snapshot("locked-old", SnapshotDisplayZone::Locked, None),
            snapshot("normal-new", SnapshotDisplayZone::Normal, None),
            snapshot(
                "normal-noted",
                SnapshotDisplayZone::Normal,
                Some("打Boss前"),
            ),
            snapshot("normal-old", SnapshotDisplayZone::Normal, None),
        ];
        let names = snapshot_display_names(&snapshots);

        assert_eq!(names["locked-new"], "锁定存档1");
        assert_eq!(names["locked-old"], "锁定存档2");
        assert_eq!(names["normal-new"], "存档1");
        assert_eq!(names["normal-noted"], "打Boss前");
        assert_eq!(names["normal-old"], "存档3");
    }

    #[test]
    fn pending_lock_or_unlock_keeps_its_old_zone_and_name() {
        let mut pending_lock = snapshot("normal", SnapshotDisplayZone::Normal, None);
        pending_lock.locked = true;
        let pending_unlock = snapshot("locked", SnapshotDisplayZone::Locked, None);
        let names = snapshot_display_names(&[pending_lock.clone(), pending_unlock.clone()]);

        assert_eq!(names["normal"], "存档1");
        assert_eq!(names["locked"], "锁定存档1");
        assert!(pending_lock.display_zone.is_pending(pending_lock.locked));
        assert!(!pending_unlock
            .display_zone
            .is_pending(pending_unlock.locked));

        let mut unlocked = pending_unlock;
        unlocked.locked = false;
        assert!(unlocked.display_zone.is_pending(unlocked.locked));
    }
}

fn game_to_dto(repo: &Arc<dyn Repository>, g: &Game) -> GameDto {
    let snaps = repo.list_snapshots(&g.id).unwrap_or_default();
    let launch_kind = if g.emulator_binding.is_some() {
        Some("emulator".into())
    } else {
        g.launch_binding.as_ref().map(|binding| {
            if binding.steam_app_id.is_some() {
                "steam".into()
            } else {
                "executable".into()
            }
        })
    };
    GameDto {
        id: g.id.clone(),
        name: g.name.clone(),
        icon: g.icon.clone(),
        save_paths: g
            .save_paths
            .iter()
            .map(|path| display_path_string(path))
            .collect(),
        emulator: g
            .emulator_identity
            .as_ref()
            .map(|identity| identity.emulator.clone()),
        configuration_state: match g.configuration_state() {
            GameConfigurationState::Configured => "configured",
            GameConfigurationState::PendingDiscovery => "pending_discovery",
            GameConfigurationState::PendingBinding => "pending_binding",
        }
        .into(),
        launch_kind,
        launch_executable_path: g
            .launch_binding
            .as_ref()
            .map(|binding| display_path_string(&binding.executable_path)),
        launch_arguments: g
            .launch_binding
            .as_ref()
            .map(|binding| binding.launch_arguments.clone())
            .unwrap_or_default(),
        steam_app_id: g
            .launch_binding
            .as_ref()
            .and_then(|binding| binding.steam_app_id),
        install_dir: g
            .launch_binding
            .as_ref()
            .map(|binding| display_path_string(&binding.install_dir)),
        rom_path: g
            .emulator_binding
            .as_ref()
            .map(|binding| display_path_string(&binding.rom_path)),
        snapshot_count: snaps.len(),
        last_snapshot_at: snaps.first().map(|s| s.created_at.clone()),
    }
}

fn display_path_string(path: &Path) -> String {
    let value = path.to_string_lossy();
    let lowercase = value.to_ascii_lowercase();
    if lowercase.starts_with("\\\\?\\unc\\") {
        return format!("\\\\{}", &value[8..]);
    }
    if lowercase.starts_with("\\\\?\\") {
        return value[4..].to_string();
    }
    value.into_owned()
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
pub fn get_save_discovery_status(
    state: State<'_, AppState>,
) -> Result<SaveDiscoveryStatus, String> {
    state.save_discovery.status()
}

#[tauri::command]
pub fn start_save_discovery(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: String,
) -> Result<SaveDiscoveryStatus, String> {
    let app_local_data_dir = app
        .path()
        .app_local_data_dir()
        .map_err(|error| format!("无法获取应用本地数据目录：{error}"))?;
    let game = state
        .repo
        .get_game(&game_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "游戏不存在".to_string())?;
    if game.configuration_state() != GameConfigurationState::PendingDiscovery {
        return Err("只有尚未设置存档目录的普通 PC 游戏可以启动动态发现".into());
    }
    if game.emulator_identity.is_some() {
        return Err("模拟器游戏不使用普通 PC 游戏的存档动态发现".into());
    }
    let launch_binding = game
        .launch_binding
        .clone()
        .ok_or_else(|| "该游戏尚未绑定本机启动程序".to_string())?;
    state.save_discovery.start(
        app,
        SaveDiscoveryStartRequest {
            game_id: game.id,
            game_name: game.name,
            launch_binding,
            data_dir: state.data_dir.clone(),
            app_local_data_dir,
            repository_dir: state.repository_dir.clone(),
        },
    )
}

#[derive(Serialize)]
pub struct LaunchGameResult {
    pub pid: u32,
}

#[tauri::command]
pub fn launch_game(
    state: State<'_, AppState>,
    game_id: String,
) -> Result<LaunchGameResult, String> {
    let game = state
        .repo
        .get_game(&game_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "游戏不存在".to_string())?;
    if game.configuration_state() != GameConfigurationState::Configured {
        return Err("只有已经设置存档目录的普通 PC 游戏可以直接启动".into());
    }
    if game.emulator_identity.is_some() {
        return Err("模拟器游戏不使用普通 PC 游戏的启动方式".into());
    }
    let launch_binding = game
        .launch_binding
        .clone()
        .ok_or_else(|| "该游戏尚未绑定本机启动程序".to_string())?;
    let child = state.save_discovery.launch_only(launch_binding)?;
    Ok(LaunchGameResult { pid: child.id() })
}

#[tauri::command]
pub fn stop_save_discovery(state: State<'_, AppState>) -> Result<SaveDiscoveryStatus, String> {
    state.save_discovery.stop_and_analyze()
}

#[tauri::command]
pub fn cancel_save_discovery(state: State<'_, AppState>) -> Result<SaveDiscoveryStatus, String> {
    state.save_discovery.cancel()
}

#[tauri::command]
pub fn confirm_save_discovery_paths(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: String,
    save_paths: Vec<String>,
) -> Result<ConfirmSaveDiscoveryPathsDto, String> {
    let requested_paths = save_paths
        .into_iter()
        .map(|path| PathBuf::from(path.trim()))
        .collect::<Vec<_>>();
    let selected_paths = state
        .save_discovery
        .begin_confirmation(&game_id, &requested_paths)?;

    match confirm_save_discovery_paths_inner(&state, &game_id, selected_paths) {
        Ok(result) => {
            state.save_discovery.complete_confirmation(&game_id);
            if result.first_backup == FirstBackupOutcomeDto::Created {
                auto_backup::trigger_sync(app);
            }
            Ok(result)
        }
        Err(error) => {
            state.save_discovery.abort_confirmation(&game_id);
            Err(error)
        }
    }
}

fn confirm_save_discovery_paths_inner(
    state: &AppState,
    game_id: &str,
    save_paths: Vec<PathBuf>,
) -> Result<ConfirmSaveDiscoveryPathsDto, String> {
    let _operation = acquire_snapshot_operation_guard(state)?;
    let mut game = state
        .repo
        .get_game(game_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "游戏不存在".to_string())?;
    if game.configuration_state() != GameConfigurationState::PendingDiscovery
        || game.emulator_identity.is_some()
    {
        return Err("只有尚未设置存档目录的普通 PC 游戏可以确认监测结果".into());
    }
    savelink_core::scan::validate_save_paths(&save_paths).map_err(|error| error.to_string())?;
    let existing = state.repo.list_games().map_err(|error| error.to_string())?;
    if let Some(conflict) = find_save_path_conflict(&existing, &save_paths, Some(game.id.as_str()))
    {
        return Err(duplicate_save_path_message(conflict));
    }
    let auto_backup_enabled = auto_backup::enabled(&state.cloud_repo)?;

    game.save_paths = save_paths;
    game.updated_at = state.clock.now_stamp();
    state
        .repo
        .update_game(game.clone())
        .map_err(|error| error.to_string())?;

    let (first_backup, snapshot, backup_error) = if !auto_backup_enabled {
        (FirstBackupOutcomeDto::Disabled, None, None)
    } else {
        match state
            .snapshots()
            .create_snapshot(game_id, None, Reason::Auto)
        {
            Ok(CreateOutcome::Created(snapshot)) => (
                FirstBackupOutcomeDto::Created,
                Some(snapshot_to_dto(&snapshot)),
                None,
            ),
            Ok(CreateOutcome::NoChange) => (FirstBackupOutcomeDto::NoChange, None, None),
            Err(error) => (FirstBackupOutcomeDto::Failed, None, Some(error.to_string())),
        }
    };

    Ok(ConfirmSaveDiscoveryPathsDto {
        game: game_to_dto(&state.repo, &game),
        first_backup,
        snapshot,
        backup_error,
    })
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
        retention_limit: auto_backup::retention_limit(&state.cloud_repo)?,
        retention_policy_confirmed: auto_backup::retention_policy_confirmed(&state.cloud_repo)?,
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
        retention_limit: auto_backup::retention_limit(&state.cloud_repo)?,
        retention_policy_confirmed: auto_backup::retention_policy_confirmed(&state.cloud_repo)?,
    })
}

#[tauri::command]
pub fn set_auto_backup_retention(
    app: AppHandle,
    state: State<'_, AppState>,
    limit: u32,
) -> Result<AutoBackupSettingsDto, String> {
    let limit = usize::try_from(limit).map_err(|_| "快照保留数量无效".to_string())?;
    auto_backup::set_retention_limit(&state.cloud_repo, limit)?;
    auto_backup::trigger_sync(app);
    Ok(AutoBackupSettingsDto {
        enabled: auto_backup::enabled(&state.cloud_repo)?,
        interval_minutes: auto_backup::AUTO_BACKUP_INTERVAL_MINUTES,
        retention_limit: limit,
        retention_policy_confirmed: true,
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
    let display_names = snapshot_display_names(&snaps);
    snaps
        .iter()
        .map(|snapshot| {
            let cloud = state
                .cloud_repo
                .get_cloud_snapshot(BAIDU_ACCOUNT_ID, &snapshot.id)
                .map_err(|error| error.to_string())?;
            Ok(snapshot_to_dto_with_view(
                snapshot,
                cloud.as_ref(),
                display_names
                    .get(&snapshot.id)
                    .cloned()
                    .unwrap_or_else(|| "未命名快照".into()),
            ))
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
        display_name: "".into(),
        display_zone: "normal".into(),
        pending_reorganization: false,
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

#[tauri::command]
pub fn scan_program_game(
    app: AppHandle,
    state: State<'_, AppState>,
    selected_path: String,
) -> Result<ProgramDiscoveryReportDto, String> {
    let selected_path = selected_path.trim();
    if selected_path.is_empty() {
        return Err("请选择游戏快捷方式、EXE 或安装目录".into());
    }
    let database = resolve_manifest_database(&app)?;
    let report = ProgramDiscoveryService::new(database)
        .scan(&PathBuf::from(selected_path))
        .map_err(|error| error.to_string())?;
    let existing = state.repo.list_games().map_err(|error| error.to_string())?;
    let resolved_program_path = report.resolved_program_path.clone();
    let candidate_save_paths = report
        .games
        .iter()
        .flat_map(|game| game.save_paths.iter().cloned())
        .collect::<Vec<_>>();
    let existing_program_game = resolved_program_path.as_deref().and_then(|program_path| {
        find_existing_program_game(
            &existing,
            program_path,
            &report.install_dir,
            &candidate_save_paths,
        )
    });
    let program_already_added = existing_program_game.is_some();
    let existing_game_id = existing_program_game.map(|game| game.id.clone());
    let existing_game_name = existing_program_game.map(|game| game.name.clone());
    let can_bind_existing_launch = existing_program_game
        .is_some_and(|game| game.launch_binding.is_none() && game.emulator_identity.is_none());
    let suggested_name = report
        .games
        .first()
        .map(|game| game.name.clone())
        .or_else(|| report.identity_hints.first().cloned())
        .or_else(|| {
            resolved_program_path
                .as_ref()
                .and_then(|path| path.file_stem())
                .map(|value| value.to_string_lossy().to_string())
        })
        .or_else(|| {
            report
                .install_dir
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "未命名游戏".into());
    let games = report
        .games
        .into_iter()
        .map(|game| program_game_to_dto(game, &existing, resolved_program_path.as_deref()))
        .collect();
    Ok(ProgramDiscoveryReportDto {
        selected_path: report.selected_path.to_string_lossy().to_string(),
        selection_kind: match report.selection_kind {
            ProgramSelectionKind::Directory => "directory",
            ProgramSelectionKind::Executable => "executable",
            ProgramSelectionKind::Shortcut => "shortcut",
        }
        .into(),
        resolved_program_path: resolved_program_path.map(|path| path.to_string_lossy().to_string()),
        install_dir: report.install_dir.to_string_lossy().to_string(),
        detected_app_id: report.detected_app_id,
        app_id_source: report
            .app_id_source
            .map(|path| path.to_string_lossy().to_string()),
        ignored_app_id_game_names: report.ignored_app_id_game_names,
        identity_hints: report.identity_hints,
        suggested_name,
        program_already_added,
        existing_game_id,
        existing_game_name,
        can_bind_existing_launch,
        games,
    })
}

#[tauri::command]
pub fn scan_desmume_games(
    state: State<'_, AppState>,
    emulator_root: String,
    rom_root: Option<String>,
) -> Result<DesmumeDiscoveryReportDto, String> {
    let emulator_root = PathBuf::from(emulator_root.trim());
    let rom_root = rom_root
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let existing = state.repo.list_games().map_err(|error| error.to_string())?;
    let cached_bindings = existing
        .iter()
        .filter_map(|game| game.emulator_binding.clone())
        .collect::<Vec<_>>();
    let report = DesmumeDiscoveryService::scan_with_cache(
        &emulator_root,
        rom_root.as_deref(),
        &cached_bindings,
    )
    .map_err(|error| error.to_string())?;
    let games = report
        .games
        .into_iter()
        .map(|game| desmume_game_to_dto(game, &existing))
        .collect();
    Ok(DesmumeDiscoveryReportDto {
        emulator_root: report.emulator_root.to_string_lossy().to_string(),
        configured_rom_root: report
            .configured_rom_root
            .map(|path| path.to_string_lossy().to_string()),
        rom_root: report
            .rom_root
            .map(|path| path.to_string_lossy().to_string()),
        configured_rom_root_missing: report.configured_rom_root_missing,
        battery_dir: report.battery_dir.to_string_lossy().to_string(),
        games,
    })
}

#[tauri::command]
pub fn register_desmume_game(
    state: State<'_, AppState>,
    emulator_root: String,
    rom_root: Option<String>,
    rom_path: String,
    bind_game_id: Option<String>,
) -> Result<GameDto, String> {
    let emulator_root = PathBuf::from(emulator_root.trim());
    let rom_root = rom_root
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let requested_rom = PathBuf::from(rom_path.trim());
    let report = DesmumeDiscoveryService::scan(&emulator_root, rom_root.as_deref())
        .map_err(|error| error.to_string())?;
    let discovered = report
        .games
        .into_iter()
        .find(|game| normalized_path(&game.rom_path) == normalized_path(&requested_rom))
        .ok_or_else(|| "所选 ROM 不在本次 DeSmuME 扫描结果中".to_string())?;
    if !discovered.has_save {
        return Err("该游戏还没有生成 .dsv 存档，暂时无法添加".into());
    }
    let save_source = discovered
        .save_source()
        .map_err(|error| error.to_string())?;
    savelink_core::scan::validate_save_sources(std::slice::from_ref(&save_source))
        .map_err(|error| error.to_string())?;
    let save_root = save_source.root().to_path_buf();
    let _operation = acquire_snapshot_operation_guard(&state)?;
    let now = state.clock.now_stamp();

    let game = if let Some(game_id) = bind_game_id.filter(|value| !value.trim().is_empty()) {
        let mut game = state
            .repo
            .get_game(&game_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "要绑定的云端游戏不存在".to_string())?;
        let expected = game
            .emulator_identity
            .as_ref()
            .ok_or_else(|| "目标游戏没有模拟器身份信息".to_string())?;
        if compare_rom_identity(expected, &discovered.identity) == RomMatch::None {
            return Err("所选 ROM 与目标云端游戏不匹配".into());
        }
        game.save_paths = vec![save_root];
        game.save_sources = vec![save_source];
        game.emulator_binding = Some(discovered.binding);
        game.updated_at = now;
        state
            .repo
            .update_game(game.clone())
            .map_err(|error| error.to_string())?;
        game
    } else {
        let game = Game {
            id: state.ids.new_id("game"),
            name: discovered.name,
            icon: None,
            repo_path: PathBuf::new(),
            save_paths: vec![save_root],
            save_sources: vec![save_source],
            emulator_identity: Some(discovered.identity),
            emulator_binding: Some(discovered.binding),
            launch_binding: None,
            created_at: now.clone(),
            updated_at: now,
        };
        state
            .repo
            .insert_game(game.clone())
            .map_err(|error| error.to_string())?;
        game
    };
    Ok(game_to_dto(&state.repo, &game))
}

fn desmume_game_to_dto(game: DesmumeDiscoveredGame, existing: &[Game]) -> DesmumeDiscoveredGameDto {
    let mut matches = existing
        .iter()
        .filter_map(|saved| {
            let expected = saved.emulator_identity.as_ref()?;
            let match_kind = compare_rom_identity(expected, &game.identity);
            if match_kind == RomMatch::None {
                return None;
            }
            let already_bound_here = saved.emulator_binding.as_ref().is_some_and(|binding| {
                normalized_path(&binding.rom_path) == normalized_path(&game.rom_path)
            });
            Some(DesmumeGameMatchDto {
                game_id: saved.id.clone(),
                game_name: saved.name.clone(),
                match_kind: match match_kind {
                    RomMatch::Exact => "exact",
                    RomMatch::Possible => "possible",
                    RomMatch::None => unreachable!(),
                }
                .into(),
                already_bound_here,
            })
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|candidate| {
        (
            candidate.match_kind != "exact",
            candidate.already_bound_here,
            candidate.game_name.to_ascii_lowercase(),
        )
    });
    DesmumeDiscoveredGameDto {
        name: game.name,
        rom_path: game.rom_path.to_string_lossy().to_string(),
        save_path: game.save_path.to_string_lossy().to_string(),
        has_save: game.has_save,
        rom_sha256: game.identity.rom.sha256,
        rom_header_title: game.identity.rom.header_title,
        rom_game_code: game.identity.rom.game_code,
        matches,
    }
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
    let existing_game = find_existing_steam_game(existing, game.app_id, &game.save_paths);
    let already_added = existing_game.is_some();
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
        existing_game_id: existing_game.map(|saved| saved.id.clone()),
        existing_game_name: existing_game.map(|saved| saved.name.clone()),
        can_bind_existing_launch: existing_game.is_some_and(|saved| {
            saved.launch_binding.is_none() && saved.emulator_identity.is_none()
        }),
        can_add_directly: !game.save_paths.is_empty(),
    }
}

fn program_game_to_dto(
    game: ProgramDiscoveredGame,
    existing: &[Game],
    resolved_program_path: Option<&std::path::Path>,
) -> ProgramDiscoveredGameDto {
    let already_added = existing.iter().any(|saved| {
        let same_save_path = saved.save_paths.iter().any(|saved_path| {
            game.save_paths
                .iter()
                .any(|path| save_paths_overlap(saved_path, path))
        });
        let same_program = resolved_program_path.is_some_and(|program_path| {
            saved.launch_binding.as_ref().is_some_and(|binding| {
                normalized_path(&binding.executable_path) == normalized_path(program_path)
            })
        });
        same_save_path || same_program
    });
    ProgramDiscoveredGameDto {
        name: game.name,
        app_id: game.app_id,
        match_kind: match game.match_kind {
            ProgramMatchKind::AppId => "app_id",
            ProgramMatchKind::Name => "name",
        }
        .into(),
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
    let mut normalized = path.to_string_lossy().replace('/', "\\");
    let lowercase = normalized.to_ascii_lowercase();
    if lowercase.starts_with("\\\\?\\unc\\") {
        normalized = format!("\\\\{}", &normalized[8..]);
    } else if lowercase.starts_with("\\\\?\\") {
        normalized = normalized[4..].to_string();
    }
    normalized.to_lowercase()
}

fn find_save_path_conflict<'a>(
    existing: &'a [Game],
    candidate_paths: &[PathBuf],
    excluded_game_id: Option<&str>,
) -> Option<&'a Game> {
    existing.iter().find(|game| {
        excluded_game_id != Some(game.id.as_str())
            && game.save_paths.iter().any(|saved_path| {
                candidate_paths
                    .iter()
                    .any(|candidate| save_paths_overlap(saved_path, candidate))
            })
    })
}

fn find_existing_steam_game<'a>(
    existing: &'a [Game],
    app_id: u32,
    candidate_paths: &[PathBuf],
) -> Option<&'a Game> {
    existing
        .iter()
        .find(|game| {
            game.launch_binding
                .as_ref()
                .and_then(|binding| binding.steam_app_id)
                == Some(app_id)
        })
        .or_else(|| find_save_path_conflict(existing, candidate_paths, None))
}

fn find_existing_program_game<'a>(
    existing: &'a [Game],
    executable_path: &Path,
    install_dir: &Path,
    candidate_save_paths: &[PathBuf],
) -> Option<&'a Game> {
    existing.iter().find(|game| {
        let same_launch = game.launch_binding.as_ref().is_some_and(|binding| {
            binding.steam_app_id.is_none()
                && (normalized_path(&binding.executable_path) == normalized_path(executable_path)
                    || normalized_path(&binding.install_dir) == normalized_path(install_dir))
        });
        let existing_save_inside_install = game
            .save_paths
            .iter()
            .any(|save_path| path_is_same_or_descendant(install_dir, save_path));
        let overlapping_save = game.save_paths.iter().any(|saved_path| {
            candidate_save_paths
                .iter()
                .any(|candidate| save_paths_overlap(saved_path, candidate))
        });
        same_launch || existing_save_inside_install || overlapping_save
    })
}

fn same_launch_target(first: &GameLaunchBinding, second: &GameLaunchBinding) -> bool {
    match (first.steam_app_id, second.steam_app_id) {
        (Some(first_app_id), Some(second_app_id)) => first_app_id == second_app_id,
        (None, None) => {
            normalized_path(&first.executable_path) == normalized_path(&second.executable_path)
                || normalized_path(&first.install_dir) == normalized_path(&second.install_dir)
        }
        _ => false,
    }
}

fn find_launch_binding_conflict<'a>(
    existing: &'a [Game],
    candidate: &GameLaunchBinding,
    excluded_game_id: Option<&str>,
) -> Option<&'a Game> {
    existing.iter().find(|game| {
        excluded_game_id != Some(game.id.as_str())
            && game
                .launch_binding
                .as_ref()
                .is_some_and(|binding| same_launch_target(binding, candidate))
    })
}

fn validate_executable_binding(
    executable_path: &str,
    install_dir: Option<&str>,
) -> Result<GameLaunchBinding, String> {
    let executable_path = PathBuf::from(executable_path.trim());
    if !executable_path.is_absolute() || !executable_path.is_file() {
        return Err("请选择真实存在的游戏 EXE".into());
    }
    if !executable_path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return Err("游戏启动文件必须是 EXE".into());
    }
    let executable_path =
        std::fs::canonicalize(&executable_path).map_err(|error| error.to_string())?;
    let install_dir = install_dir
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| executable_path.parent().map(Path::to_path_buf))
        .ok_or_else(|| "无法确定游戏安装目录".to_string())?;
    if !install_dir.is_absolute() || !install_dir.is_dir() {
        return Err("游戏安装目录不存在或不是绝对路径".into());
    }
    let install_dir = std::fs::canonicalize(&install_dir).map_err(|error| error.to_string())?;
    if !executable_path.starts_with(&install_dir) {
        return Err("游戏 EXE 不在识别到的安装目录中".into());
    }
    Ok(GameLaunchBinding::executable(executable_path, install_dir))
}

fn validate_steam_binding(
    steam_root: &str,
    install_dir: &str,
    app_id: u32,
) -> Result<GameLaunchBinding, String> {
    let steam_root = PathBuf::from(steam_root.trim());
    let steam_executable = steam_root.join("steam.exe");
    if !steam_executable.is_file() {
        return Err("Steam 启动程序不存在，请重新选择 Steam 目录".into());
    }
    let install_dir = PathBuf::from(install_dir.trim());
    if !install_dir.is_absolute() || !install_dir.is_dir() {
        return Err("Steam 游戏安装目录不存在".into());
    }
    let steam_executable =
        std::fs::canonicalize(steam_executable).map_err(|error| error.to_string())?;
    let install_dir = std::fs::canonicalize(install_dir).map_err(|error| error.to_string())?;
    Ok(GameLaunchBinding::steam(
        steam_executable,
        install_dir,
        app_id,
    ))
}

fn duplicate_game_message(game: &Game) -> String {
    format!(
        "这个游戏已作为“{}”添加到 SaveLink，请编辑现有游戏",
        game.name
    )
}

fn duplicate_save_path_message(game: &Game) -> String {
    format!("存档目录已由“{}”管理，不能重复或嵌套添加", game.name)
}

#[tauri::command]
pub fn add_game(
    state: State<'_, AppState>,
    name: String,
    save_paths: Vec<String>,
    executable_path: Option<String>,
    install_dir: Option<String>,
) -> Result<GameDto, String> {
    if name.trim().is_empty() {
        return Err("游戏名称不能为空".into());
    }
    if save_paths.is_empty() {
        return Err("请至少选择一个存档目录".into());
    }
    let launch_binding = validate_executable_binding(
        executable_path.as_deref().unwrap_or_default(),
        install_dir.as_deref(),
    )?;
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
    let existing = state.repo.list_games().map_err(|error| error.to_string())?;
    if let Some(conflict) = find_save_path_conflict(&existing, &save_paths, None) {
        return Err(duplicate_save_path_message(conflict));
    }
    if let Some(conflict) = find_launch_binding_conflict(&existing, &launch_binding, None) {
        return Err(duplicate_game_message(conflict));
    }
    let now = state.clock.now_stamp();
    let game = Game {
        id: state.ids.new_id("game"),
        name,
        icon: None,
        repo_path: std::path::PathBuf::new(), // 仓库由 store 管理，DTO 不暴露
        save_paths,
        save_sources: Vec::new(),
        emulator_identity: None,
        emulator_binding: None,
        launch_binding: Some(launch_binding),
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
pub fn add_program_game(
    state: State<'_, AppState>,
    name: String,
    save_paths: Vec<String>,
    executable_path: String,
    install_dir: String,
) -> Result<GameDto, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("游戏名称不能为空".into());
    }
    let launch_binding = validate_executable_binding(&executable_path, Some(&install_dir))?;
    let save_paths = save_paths
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if !save_paths.is_empty() {
        savelink_core::scan::validate_save_paths(&save_paths).map_err(|error| error.to_string())?;
    }

    let _operation = acquire_snapshot_operation_guard(&state)?;
    let existing = state.repo.list_games().map_err(|error| error.to_string())?;
    if let Some(conflict) = find_existing_program_game(
        &existing,
        &launch_binding.executable_path,
        &launch_binding.install_dir,
        &save_paths,
    ) {
        return Err(duplicate_game_message(conflict));
    }
    if let Some(conflict) = find_launch_binding_conflict(&existing, &launch_binding, None) {
        return Err(duplicate_game_message(conflict));
    }
    let now = state.clock.now_stamp();
    let game = Game {
        id: state.ids.new_id("game"),
        name: name.to_string(),
        icon: None,
        repo_path: PathBuf::new(),
        save_paths,
        save_sources: Vec::new(),
        emulator_identity: None,
        emulator_binding: None,
        launch_binding: Some(launch_binding),
        created_at: now.clone(),
        updated_at: now,
    };
    state
        .repo
        .insert_game(game.clone())
        .map_err(|error| error.to_string())?;
    Ok(game_to_dto(&state.repo, &game))
}

#[tauri::command]
pub fn register_steam_game(
    state: State<'_, AppState>,
    name: String,
    save_paths: Vec<String>,
    steam_root: String,
    install_dir: String,
    app_id: u32,
) -> Result<GameDto, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("游戏名称不能为空".into());
    }
    let save_paths = save_paths
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if save_paths.is_empty() {
        return Err("请至少选择一个存档目录".into());
    }
    savelink_core::scan::validate_save_paths(&save_paths).map_err(|error| error.to_string())?;
    let launch_binding = validate_steam_binding(&steam_root, &install_dir, app_id)?;

    let _operation = acquire_snapshot_operation_guard(&state)?;
    let existing = state.repo.list_games().map_err(|error| error.to_string())?;
    if let Some(saved) = find_save_path_conflict(&existing, &save_paths, None) {
        if saved.emulator_identity.is_some() {
            return Err(duplicate_save_path_message(saved));
        }
        let mut saved = saved.clone();
        if let Some(current) = saved.launch_binding.as_ref() {
            if same_launch_target(current, &launch_binding) {
                return Ok(game_to_dto(&state.repo, &saved));
            }
            return Err(duplicate_game_message(&saved));
        }
        if let Some(conflict) =
            find_launch_binding_conflict(&existing, &launch_binding, Some(saved.id.as_str()))
        {
            return Err(duplicate_game_message(conflict));
        }
        state.save_discovery.ensure_game_mutable(&saved.id)?;
        saved.launch_binding = Some(launch_binding);
        saved.updated_at = state.clock.now_stamp();
        state
            .repo
            .update_game(saved.clone())
            .map_err(|error| error.to_string())?;
        return Ok(game_to_dto(&state.repo, &saved));
    }
    if let Some(conflict) = find_launch_binding_conflict(&existing, &launch_binding, None) {
        return Err(duplicate_game_message(conflict));
    }
    let now = state.clock.now_stamp();
    let game = Game {
        id: state.ids.new_id("game"),
        name: name.to_string(),
        icon: None,
        repo_path: PathBuf::new(),
        save_paths,
        save_sources: Vec::new(),
        emulator_identity: None,
        emulator_binding: None,
        launch_binding: Some(launch_binding),
        created_at: now.clone(),
        updated_at: now,
    };
    state
        .repo
        .insert_game(game.clone())
        .map_err(|error| error.to_string())?;
    Ok(game_to_dto(&state.repo, &game))
}

#[tauri::command]
pub fn bind_program_to_game(
    state: State<'_, AppState>,
    game_id: String,
    executable_path: String,
    install_dir: Option<String>,
    replace_existing: bool,
) -> Result<GameDto, String> {
    state.save_discovery.ensure_game_mutable(&game_id)?;
    let launch_binding = validate_executable_binding(&executable_path, install_dir.as_deref())?;
    let _operation = acquire_snapshot_operation_guard(&state)?;
    let mut game = state
        .repo
        .get_game(&game_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "游戏不存在".to_string())?;
    if game.emulator_identity.is_some() {
        return Err("模拟器游戏请通过对应模拟器重新绑定".into());
    }
    let existing = state.repo.list_games().map_err(|error| error.to_string())?;
    if let Some(conflict) =
        find_launch_binding_conflict(&existing, &launch_binding, Some(game.id.as_str()))
    {
        return Err(duplicate_game_message(conflict));
    }
    if let Some(current) = game.launch_binding.as_ref() {
        if same_launch_target(current, &launch_binding) {
            return Ok(game_to_dto(&state.repo, &game));
        }
        if !replace_existing {
            return Err("该游戏已有启动方式，请在编辑游戏中确认更换".into());
        }
    }
    game.launch_binding = Some(launch_binding);
    game.updated_at = state.clock.now_stamp();
    state
        .repo
        .update_game(game.clone())
        .map_err(|error| error.to_string())?;
    Ok(game_to_dto(&state.repo, &game))
}

#[tauri::command]
pub fn update_game(
    state: State<'_, AppState>,
    game_id: String,
    name: String,
    save_paths: Vec<String>,
) -> Result<GameDto, String> {
    state.save_discovery.ensure_game_mutable(&game_id)?;
    if name.trim().is_empty() {
        return Err("游戏名称不能为空".into());
    }
    let trimmed_paths: Vec<String> = save_paths
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    let trimmed_paths = trimmed_paths
        .into_iter()
        .map(std::path::PathBuf::from)
        .collect::<Vec<_>>();
    let _operation = acquire_snapshot_operation_guard(&state)?;

    let mut game = state
        .repo
        .get_game(&game_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "游戏不存在".to_string())?;
    if game.emulator_identity.is_some() {
        if game
            .save_paths
            .iter()
            .map(|path| normalized_path(path))
            .collect::<Vec<_>>()
            != trimmed_paths
                .iter()
                .map(|path| normalized_path(path))
                .collect::<Vec<_>>()
        {
            return Err("模拟器游戏请通过 DeSmuME 重新扫描和绑定，不能直接修改存档目录".into());
        }
    } else {
        if trimmed_paths.is_empty() && (game.is_configured() || game.launch_binding.is_none()) {
            return Err("请至少选择一个存档目录".into());
        }
        if !trimmed_paths.is_empty() {
            savelink_core::scan::validate_save_paths(&trimmed_paths).map_err(|e| e.to_string())?;
            let existing = state.repo.list_games().map_err(|error| error.to_string())?;
            if let Some(conflict) =
                find_save_path_conflict(&existing, &trimmed_paths, Some(game.id.as_str()))
            {
                return Err(duplicate_save_path_message(conflict));
            }
            game.save_paths = trimmed_paths;
        }
    }
    game.name = name.trim().to_string();
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
    app: AppHandle,
    state: State<'_, AppState>,
    snapshot_id: String,
    note: Option<String>,
    locked: Option<bool>,
) -> Result<(), String> {
    let _operation = acquire_snapshot_operation_guard(&state)?;
    if (note.is_some() || locked.is_some())
        && state
            .cloud_repo
            .get_cloud_snapshot(BAIDU_ACCOUNT_ID, &snapshot_id)
            .map_err(|error| error.to_string())?
            .is_some()
    {
        // 先持久化 pending，再更新本地字段，保证进程在两步之间退出后仍会重试。
        state
            .cloud_repo
            .update_cloud_snapshot_metadata_status(
                BAIDU_ACCOUNT_ID,
                &snapshot_id,
                CloudSnapshotMetadataState {
                    status: CloudMetadataSyncStatus::Pending,
                    last_synced_at: None,
                    last_error_code: None,
                    remote_note_updated_at: None,
                    remote_locked_updated_at: None,
                },
            )
            .map_err(|error| error.to_string())?;
    }
    state
        .snapshots()
        .update_meta(&snapshot_id, note, locked)
        .map_err(|e| e.to_string())?;
    drop(_operation);
    auto_backup::trigger_sync(app);
    Ok(())
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
    state.save_discovery.ensure_game_mutable(&game_id)?;
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

#[cfg(test)]
mod duplicate_game_tests {
    use super::*;

    fn game(
        name: &str,
        save_paths: Vec<PathBuf>,
        launch_binding: Option<GameLaunchBinding>,
    ) -> Game {
        Game {
            id: format!("game-{name}"),
            name: name.into(),
            icon: None,
            repo_path: PathBuf::new(),
            save_paths,
            save_sources: Vec::new(),
            emulator_identity: None,
            emulator_binding: None,
            launch_binding,
            created_at: "2026-08-25T00:00:00Z".into(),
            updated_at: "2026-08-25T00:00:00Z".into(),
        }
    }

    #[test]
    fn legacy_game_with_install_local_save_blocks_program_duplicate() {
        let existing = vec![game(
            "奥术扳机",
            vec![PathBuf::from(r"C:\Games\Arcane.Trigger\Processes")],
            None,
        )];

        let conflict = find_existing_program_game(
            &existing,
            Path::new(r"\\?\C:\Games\Arcane.Trigger\GunWizard.exe"),
            Path::new(r"\\?\C:\Games\Arcane.Trigger"),
            &[],
        );

        assert_eq!(conflict.map(|game| game.name.as_str()), Some("奥术扳机"));
    }

    #[test]
    fn unrelated_sibling_install_does_not_block_program() {
        let existing = vec![game(
            "Other Game",
            vec![PathBuf::from(r"C:\Games\Other\Save")],
            None,
        )];

        assert!(find_existing_program_game(
            &existing,
            Path::new(r"C:\Games\Arcane.Trigger\GunWizard.exe"),
            Path::new(r"C:\Games\Arcane.Trigger"),
            &[],
        )
        .is_none());
    }

    #[test]
    fn save_path_conflict_detects_cross_game_parent_child_paths() {
        let existing = vec![game(
            "Existing",
            vec![PathBuf::from(r"C:\Saves\Game")],
            None,
        )];
        let candidate = vec![PathBuf::from(r"C:\Saves\Game\Profile")];

        assert_eq!(
            find_save_path_conflict(&existing, &candidate, None).map(|game| game.name.as_str()),
            Some("Existing")
        );
        assert!(
            find_save_path_conflict(&existing, &candidate, Some(existing[0].id.as_str())).is_none()
        );
    }

    #[test]
    fn normalized_path_ignores_windows_verbatim_prefix() {
        assert_eq!(
            normalized_path(Path::new(r"C:\Games\Arcane\game.exe")),
            normalized_path(Path::new(r"\\?\C:\Games\Arcane\game.exe"))
        );
    }

    #[test]
    fn display_path_string_removes_windows_verbatim_prefix() {
        assert_eq!(
            display_path_string(Path::new(r"\\?\C:\Games\Arcane\game.exe")),
            r"C:\Games\Arcane\game.exe"
        );
        assert_eq!(
            display_path_string(Path::new(r"\\?\UNC\server\share\save")),
            r"\\server\share\save"
        );
    }

    #[test]
    fn steam_app_id_is_the_launch_binding_identity() {
        let existing = vec![game(
            "Elden Ring",
            Vec::new(),
            Some(GameLaunchBinding::steam(
                PathBuf::from(r"C:\Steam\steam.exe"),
                PathBuf::from(r"C:\SteamLibrary\steamapps\common\Elden Ring"),
                1245620,
            )),
        )];
        let candidate = GameLaunchBinding::steam(
            PathBuf::from(r"D:\Steam\steam.exe"),
            PathBuf::from(r"D:\Library\steamapps\common\Elden Ring"),
            1245620,
        );

        assert_eq!(
            find_launch_binding_conflict(&existing, &candidate, None)
                .map(|game| game.name.as_str()),
            Some("Elden Ring")
        );
    }

    #[test]
    fn ordinary_executable_matches_legacy_game_by_install_local_save() {
        let existing = vec![game(
            "Arcane Trigger",
            vec![PathBuf::from(r"C:\Games\Arcane.Trigger\Processes")],
            None,
        )];
        let candidate = GameLaunchBinding::executable(
            PathBuf::from(r"C:\Games\Arcane.Trigger\GunWizard.exe"),
            PathBuf::from(r"C:\Games\Arcane.Trigger"),
        );

        assert_eq!(
            find_existing_program_game(
                &existing,
                &candidate.executable_path,
                &candidate.install_dir,
                &[],
            )
            .map(|game| game.name.as_str()),
            Some("Arcane Trigger")
        );
    }

    #[test]
    fn existing_launch_binding_is_not_marked_as_available_for_rebinding() {
        let existing = vec![game(
            "Already Bound",
            vec![PathBuf::from(r"C:\Saves\Already Bound")],
            Some(GameLaunchBinding::executable(
                PathBuf::from(r"C:\Games\Already Bound\game.exe"),
                PathBuf::from(r"C:\Games\Already Bound"),
            )),
        )];
        let found = find_existing_program_game(
            &existing,
            Path::new(r"C:\Games\Already Bound\game.exe"),
            Path::new(r"C:\Games\Already Bound"),
            &[],
        );

        assert!(found.is_some());
        assert!(!found.is_some_and(|game| {
            game.launch_binding.is_none() && game.emulator_identity.is_none()
        }));
    }

    #[test]
    fn steam_discovery_detects_existing_app_id_after_save_path_changes() {
        let existing = vec![game(
            "Elden Ring",
            vec![PathBuf::from(r"D:\Moved Saves\Elden Ring")],
            Some(GameLaunchBinding::steam(
                PathBuf::from(r"C:\Steam\steam.exe"),
                PathBuf::from(r"C:\Steam\steamapps\common\ELDEN RING"),
                1245620,
            )),
        )];

        assert_eq!(
            find_existing_steam_game(
                &existing,
                1245620,
                &[PathBuf::from(r"C:\Users\User\AppData\Roaming\EldenRing")],
            )
            .map(|game| game.name.as_str()),
            Some("Elden Ring")
        );
    }
}

#[cfg(test)]
mod save_discovery_confirmation_tests {
    use super::*;
    use std::fs;

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "savelink-confirm-command-{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn pending_game(id: &str, root: &Path) -> Game {
        Game {
            id: id.into(),
            name: "Discovery Game".into(),
            icon: None,
            repo_path: PathBuf::new(),
            save_paths: Vec::new(),
            save_sources: Vec::new(),
            emulator_identity: None,
            emulator_binding: None,
            launch_binding: Some(GameLaunchBinding::executable(
                root.join("game.exe"),
                root.to_path_buf(),
            )),
            created_at: "2026-08-25T00:00:00Z".into(),
            updated_at: "2026-08-25T00:00:00Z".into(),
        }
    }

    #[test]
    fn confirmed_paths_create_the_first_auto_backup() {
        let root = temp_root("created");
        let save = root.join("save");
        fs::create_dir_all(&save).unwrap();
        fs::write(save.join("slot.dat"), b"v1").unwrap();
        let state = AppState::init(&root.join("data")).unwrap();
        state
            .repo
            .insert_game(pending_game("game-created", &root))
            .unwrap();

        let result =
            confirm_save_discovery_paths_inner(&state, "game-created", vec![save.clone()]).unwrap();

        assert_eq!(result.first_backup, FirstBackupOutcomeDto::Created);
        assert_eq!(
            result
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.reason.as_str()),
            Some("auto")
        );
        assert_eq!(result.game.configuration_state, "configured");
        assert_eq!(state.repo.list_snapshots("game-created").unwrap().len(), 1);
        drop(state);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn disabled_auto_backup_saves_paths_without_a_snapshot() {
        let root = temp_root("disabled");
        let save = root.join("save");
        fs::create_dir_all(&save).unwrap();
        let state = AppState::init(&root.join("data")).unwrap();
        auto_backup::set_enabled(&state.cloud_repo, false).unwrap();
        state
            .repo
            .insert_game(pending_game("game-disabled", &root))
            .unwrap();

        let result =
            confirm_save_discovery_paths_inner(&state, "game-disabled", vec![save]).unwrap();

        assert_eq!(result.first_backup, FirstBackupOutcomeDto::Disabled);
        assert!(result.snapshot.is_none());
        assert!(state
            .repo
            .list_snapshots("game-disabled")
            .unwrap()
            .is_empty());
        drop(state);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unchanged_first_backup_is_reported_without_a_duplicate() {
        let root = temp_root("no-change");
        let save = root.join("save");
        fs::create_dir_all(&save).unwrap();
        fs::write(save.join("slot.dat"), b"same").unwrap();
        let state = AppState::init(&root.join("data")).unwrap();
        let mut game = pending_game("game-no-change", &root);
        game.save_paths = vec![save.clone()];
        state.repo.insert_game(game.clone()).unwrap();
        state
            .snapshots()
            .create_snapshot(&game.id, None, Reason::Auto)
            .unwrap();
        game.save_paths.clear();
        state.repo.update_game(game).unwrap();

        let result =
            confirm_save_discovery_paths_inner(&state, "game-no-change", vec![save]).unwrap();

        assert_eq!(result.first_backup, FirstBackupOutcomeDto::NoChange);
        assert!(result.snapshot.is_none());
        assert_eq!(
            state.repo.list_snapshots("game-no-change").unwrap().len(),
            1
        );
        drop(state);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn first_backup_failure_keeps_the_confirmed_paths() {
        let root = temp_root("failed");
        let missing = root.join("removed-after-validation");
        let state = AppState::init(&root.join("data")).unwrap();
        state
            .repo
            .insert_game(pending_game("game-failed", &root))
            .unwrap();

        let result =
            confirm_save_discovery_paths_inner(&state, "game-failed", vec![missing.clone()])
                .unwrap();

        assert_eq!(result.first_backup, FirstBackupOutcomeDto::Failed);
        assert!(result.backup_error.is_some());
        assert_eq!(
            state
                .repo
                .get_game("game-failed")
                .unwrap()
                .unwrap()
                .save_paths,
            vec![missing]
        );
        drop(state);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cross_game_path_conflict_does_not_modify_the_pending_game() {
        let root = temp_root("conflict");
        let save = root.join("save");
        let child = save.join("profile");
        fs::create_dir_all(&child).unwrap();
        let state = AppState::init(&root.join("data")).unwrap();
        let mut existing = pending_game("existing", &root);
        existing.name = "Existing Game".into();
        existing.save_paths = vec![save];
        state.repo.insert_game(existing).unwrap();
        state
            .repo
            .insert_game(pending_game("pending", &root))
            .unwrap();

        let error = confirm_save_discovery_paths_inner(&state, "pending", vec![child])
            .err()
            .expect("父子目录冲突应被拒绝");

        assert!(error.contains("Existing Game"));
        assert!(state
            .repo
            .get_game("pending")
            .unwrap()
            .unwrap()
            .save_paths
            .is_empty());
        drop(state);
        let _ = fs::remove_dir_all(root);
    }
}
