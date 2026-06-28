//! Tauri 命令层（薄壳）。
//!
//! 职责：把 savelink-core 的纯逻辑包成前端可调用的命令，并做 DTO 序列化。
//! 不写业务逻辑——业务在 core 里，已被测试焊死。
//!
//! 对照 Java BS：这一层相当于 Spring Controller，core 相当于 Service 层。

use savelink_core::model::{CreateOutcome, Game, MissingDirChoice, Reason, Snapshot};
use savelink_core::repo::{Clock, IdGen, Repository};
use savelink_core::service::{RestoreService, SnapshotService};
use savelink_core::sqlite_repo::SqliteRepo;
use savelink_core::store::{FsStore, SnapshotStore};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

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
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        format!("{prefix}_{nanos}_{n}")
    }
}

/// 应用全局状态（类似 Spring 单例 Bean）：共享一份 repo / store / clock / ids。
pub struct AppState {
    pub repo: Arc<dyn Repository>,
    pub store: Arc<dyn SnapshotStore>,
    pub repository_dir: PathBuf,
    pub clock: Arc<dyn Clock>,
    pub ids: Arc<dyn IdGen>,
}

impl AppState {
    /// 在指定数据目录下初始化：savelink.db + repository/ 仓库目录。
    pub fn init(data_dir: &std::path::Path) -> Result<Self, String> {
        std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        let repo = SqliteRepo::open(data_dir.join("savelink.db")).map_err(|e| e.to_string())?;
        let repository_dir = data_dir.join("repository");
        let store = FsStore::new(repository_dir.clone());
        Ok(Self {
            repo: Arc::new(repo),
            store: Arc::new(store),
            repository_dir,
            clock: Arc::new(SystemClock),
            ids: Arc::new(TimeIdGen { counter: std::sync::atomic::AtomicU64::new(0) }),
        })
    }

    fn snapshots(&self) -> SnapshotService {
        SnapshotService::new(self.repo.clone(), self.store.clone(), self.clock.clone(), self.ids.clone())
    }

    fn restorer(&self) -> RestoreService {
        RestoreService::new(self.repo.clone(), self.store.clone(), self.clock.clone(), self.ids.clone())
    }
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
    SnapshotDto {
        id: s.id.clone(),
        game_id: s.game_id.clone(),
        created_at: s.created_at.clone(),
        note: s.note.clone(),
        reason: reason_str(s.reason),
        locked: s.locked,
        file_count: s.file_count,
        total_size: s.total_size,
    }
}

fn game_to_dto(repo: &Arc<dyn Repository>, g: &Game) -> GameDto {
    let snaps = repo.list_snapshots(&g.id).unwrap_or_default();
    GameDto {
        id: g.id.clone(),
        name: g.name.clone(),
        icon: g.icon.clone(),
        save_paths: g.save_paths.iter().map(|p| p.to_string_lossy().to_string()).collect(),
        snapshot_count: snaps.len(),
        last_snapshot_at: snaps.first().map(|s| s.created_at.clone()),
    }
}

/* ---------- Tauri 命令（前端通过 invoke 调用，类似发 HTTP 到 Controller） ---------- */

use tauri::State;

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
pub fn list_snapshots(state: State<'_, AppState>, game_id: String) -> Result<Vec<SnapshotDto>, String> {
    let snaps = state.repo.list_snapshots(&game_id).map_err(|e| e.to_string())?;
    Ok(snaps.iter().map(snapshot_to_dto).collect())
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
        save_paths: save_paths.into_iter().map(std::path::PathBuf::from).collect(),
        created_at: now.clone(),
        updated_at: now,
    };
    state.repo.insert_game(game.clone()).map_err(|e| e.to_string())?;
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
    state.snapshots().update_meta(&snapshot_id, note, locked).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_snapshot(state: State<'_, AppState>, snapshot_id: String) -> Result<(), String> {
    state.snapshots().delete_snapshot(&snapshot_id).map_err(|e| e.to_string())
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
    Ok(RestoreResultDto { target_id: out.target_id, backup_id: out.backup_id })
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
    Ok(RestoreResultDto { target_id: out.target_id, backup_id: out.backup_id })
}

