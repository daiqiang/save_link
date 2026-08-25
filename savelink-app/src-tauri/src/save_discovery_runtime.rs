use savelink_core::model::GameLaunchBinding;
use savelink_core::save_activity::{
    analyze_save_activity, FileActivityEvent, FileActivityKind, SaveActivityAnalysisContext,
    SaveDirectoryCandidate,
};
use savelink_core::scan::path_is_same_or_descendant;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

const STATUS_EVENT: &str = "save-discovery-status-changed";
const WATCH_BUFFER_SIZE: u32 = 64 * 1024;
const EVENT_LIMIT: usize = 20_000;
const READY_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveDiscoveryPhase {
    Idle,
    StartingWatchers,
    LaunchingGame,
    Monitoring,
    ExitGracePeriod,
    Analyzing,
    AwaitingConfirmation,
    Failed,
    Cancelled,
}

impl SaveDiscoveryPhase {
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::StartingWatchers
                | Self::LaunchingGame
                | Self::Monitoring
                | Self::ExitGracePeriod
                | Self::Analyzing
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SaveDiscoveryStatus {
    pub phase: SaveDiscoveryPhase,
    pub game_id: Option<String>,
    pub game_name: Option<String>,
    pub pid: Option<u32>,
    pub started_at_unix_ms: Option<u64>,
    pub launcher_fallback: bool,
    pub incomplete: bool,
    pub event_count: usize,
    pub dropped_event_count: usize,
    pub monitored_roots: Vec<PathBuf>,
    pub candidates: Vec<SaveDirectoryCandidate>,
    pub errors: Vec<String>,
}

impl Default for SaveDiscoveryStatus {
    fn default() -> Self {
        Self {
            phase: SaveDiscoveryPhase::Idle,
            game_id: None,
            game_name: None,
            pid: None,
            started_at_unix_ms: None,
            launcher_fallback: false,
            incomplete: false,
            event_count: 0,
            dropped_event_count: 0,
            monitored_roots: Vec::new(),
            candidates: Vec::new(),
            errors: Vec::new(),
        }
    }
}

pub struct SaveDiscoveryStartRequest {
    pub game_id: String,
    pub game_name: String,
    pub launch_binding: GameLaunchBinding,
    pub data_dir: PathBuf,
    pub app_local_data_dir: PathBuf,
    pub repository_dir: PathBuf,
}

#[derive(Clone, Copy)]
struct LifecycleTimings {
    launcher_threshold: Duration,
    exit_grace_period: Duration,
    poll_interval: Duration,
}

impl Default for LifecycleTimings {
    fn default() -> Self {
        Self {
            launcher_threshold: Duration::from_secs(10),
            exit_grace_period: Duration::from_secs(5),
            poll_interval: Duration::from_millis(200),
        }
    }
}

enum SessionControl {
    StopAndAnalyze,
    Cancel,
}

struct SessionResources {
    watchers: NativeWatchSession,
    collector: Arc<EventCollector>,
    health: Arc<WatchHealth>,
    control_rx: mpsc::Receiver<SessionControl>,
    status: Arc<Mutex<SaveDiscoveryStatus>>,
    emitter: StatusEmitter,
    analysis_context: SaveActivityAnalysisContext,
}

impl SessionResources {
    fn publish_progress(&self) {
        publish_observed_progress(&self.status, &self.emitter, &self.collector, &self.health);
    }

    fn fatal_error(&self) -> Option<String> {
        self.health.has_fatal_error().then(|| {
            self.health
                .errors()
                .first()
                .cloned()
                .unwrap_or_else(|| "目录监听器异常停止".into())
        })
    }

    fn analyze(self) {
        analyze_and_publish(
            self.watchers,
            self.collector,
            self.health,
            self.status,
            self.emitter,
            self.analysis_context,
        );
    }

    fn cancel(self) {
        cancel_and_publish(self.watchers, &self.status, &self.emitter);
    }

    fn fail(self, error: String) {
        self.watchers.stop();
        publish_failure(&self.status, &self.emitter, error, true);
    }
}

#[derive(Default)]
struct ManagerInner {
    control: Option<mpsc::Sender<SessionControl>>,
    worker: Option<JoinHandle<()>>,
}

pub struct SaveDiscoveryManager {
    inner: Mutex<ManagerInner>,
    status: Arc<Mutex<SaveDiscoveryStatus>>,
    timings: LifecycleTimings,
}

impl Default for SaveDiscoveryManager {
    fn default() -> Self {
        Self {
            inner: Mutex::new(ManagerInner::default()),
            status: Arc::new(Mutex::new(SaveDiscoveryStatus::default())),
            timings: LifecycleTimings::default(),
        }
    }
}

impl SaveDiscoveryManager {
    pub fn status(&self) -> Result<SaveDiscoveryStatus, String> {
        self.status
            .lock()
            .map(|status| status.clone())
            .map_err(|_| "存档发现状态锁已损坏".into())
    }

    pub fn ensure_game_mutable(&self, game_id: &str) -> Result<(), String> {
        let status = self.status()?;
        if status.phase.is_active() && status.game_id.as_deref() == Some(game_id) {
            return Err("该游戏正在查找存档，请先停止或取消监测".into());
        }
        Ok(())
    }

    pub fn start(
        &self,
        app: AppHandle,
        request: SaveDiscoveryStartRequest,
    ) -> Result<SaveDiscoveryStatus, String> {
        let roots = discovery_roots(&request.launch_binding)?;
        self.start_with_roots(app, request, roots)
    }

    fn start_with_roots(
        &self,
        app: AppHandle,
        request: SaveDiscoveryStartRequest,
        roots: Vec<PathBuf>,
    ) -> Result<SaveDiscoveryStatus, String> {
        self.start_with_roots_and_emitter(request, roots, status_emitter(app))
    }

    fn start_with_roots_and_emitter(
        &self,
        request: SaveDiscoveryStartRequest,
        roots: Vec<PathBuf>,
        emitter: StatusEmitter,
    ) -> Result<SaveDiscoveryStatus, String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "存档发现会话锁已损坏".to_string())?;
        reap_finished_worker(&mut inner);
        let current = self.status()?;
        if current.phase.is_active() {
            let game = current.game_name.as_deref().unwrap_or("另一款游戏");
            return Err(format!("{game} 正在查找存档，同一时间只能监测一个游戏"));
        }

        validate_launch_binding(&request.launch_binding)?;
        if roots.is_empty() {
            return Err("没有可用的存档活动监测目录".into());
        }
        replace_status(
            &self.status,
            &emitter,
            SaveDiscoveryStatus {
                phase: SaveDiscoveryPhase::StartingWatchers,
                game_id: Some(request.game_id.clone()),
                game_name: Some(request.game_name.clone()),
                started_at_unix_ms: Some(now_unix_ms()),
                monitored_roots: roots.clone(),
                ..SaveDiscoveryStatus::default()
            },
        );

        let collector = Arc::new(EventCollector::new(
            EVENT_LIMIT,
            vec![
                request.data_dir.clone(),
                request.app_local_data_dir.clone(),
                request.repository_dir.clone(),
            ],
        ));
        let health = Arc::new(WatchHealth::default());
        let watchers = match NativeWatchSession::start(
            &roots,
            collector.clone(),
            health.clone(),
            WATCH_BUFFER_SIZE,
        ) {
            Ok(watchers) => watchers,
            Err(error) => {
                publish_failure(&self.status, &emitter, error.clone(), true);
                return Err(error);
            }
        };

        update_status(&self.status, &emitter, |status| {
            status.phase = SaveDiscoveryPhase::LaunchingGame;
        });
        let child = match launch_game(&request.launch_binding) {
            Ok(child) => child,
            Err(error) => {
                watchers.stop();
                publish_failure(&self.status, &emitter, error.clone(), false);
                return Err(error);
            }
        };
        let pid = child.id();
        let process_started_at = Instant::now();
        update_status(&self.status, &emitter, |status| {
            status.phase = SaveDiscoveryPhase::Monitoring;
            status.pid = Some(pid);
        });

        let (control_tx, control_rx) = mpsc::channel();
        let status = self.status.clone();
        let timings = self.timings;
        let analysis_context = SaveActivityAnalysisContext {
            game_name: request.game_name,
            executable_stem: request
                .launch_binding
                .executable_path
                .file_stem()
                .map(|value| value.to_string_lossy().into_owned()),
            install_dir: Some(request.launch_binding.install_dir),
            watched_roots: roots,
            excluded_roots: vec![
                request.data_dir,
                request.app_local_data_dir,
                request.repository_dir,
            ],
        };
        let child_holder = Arc::new(Mutex::new(Some(child)));
        let worker_child = child_holder.clone();
        let worker_emitter = emitter.clone();
        let failure_emitter = emitter.clone();
        let worker = thread::Builder::new()
            .name("save-discovery-session".into())
            .spawn(move || {
                let Some(mut child) = worker_child.lock().ok().and_then(|mut child| child.take())
                else {
                    publish_failure(
                        &status,
                        &worker_emitter,
                        "无法取得已启动游戏的进程句柄".into(),
                        true,
                    );
                    return;
                };
                run_session(
                    &mut child,
                    process_started_at,
                    SessionResources {
                        watchers,
                        collector,
                        health,
                        control_rx,
                        status,
                        emitter: worker_emitter,
                        analysis_context,
                    },
                    timings,
                );
            })
            .map_err(|error| {
                let message = format!("无法启动存档发现会话线程：{error}");
                if let Ok(mut child) = child_holder.lock() {
                    if let Some(mut child) = child.take() {
                        let _ = child.kill();
                        let _ = child.wait();
                    }
                }
                publish_failure(&self.status, &failure_emitter, message.clone(), false);
                message
            })?;
        inner.control = Some(control_tx);
        inner.worker = Some(worker);
        self.status()
    }

    pub fn stop_and_analyze(&self) -> Result<SaveDiscoveryStatus, String> {
        self.send_control(SessionControl::StopAndAnalyze)?;
        self.status()
    }

    pub fn cancel(&self) -> Result<SaveDiscoveryStatus, String> {
        self.send_control(SessionControl::Cancel)?;
        self.status()
    }

    fn send_control(&self, control: SessionControl) -> Result<(), String> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| "存档发现会话锁已损坏".to_string())?;
        let status = self.status()?;
        if !status.phase.is_active() {
            return Err("当前没有正在运行的存档发现会话".into());
        }
        inner
            .control
            .as_ref()
            .ok_or_else(|| "存档发现会话尚未准备好".to_string())?
            .send(control)
            .map_err(|_| "存档发现会话已经结束".to_string())
    }

    pub fn shutdown(&self) {
        let worker = self.inner.lock().ok().and_then(|mut inner| {
            if let Some(control) = inner.control.take() {
                let _ = control.send(SessionControl::Cancel);
            }
            inner.worker.take()
        });
        if let Some(worker) = worker {
            let _ = worker.join();
        }
    }
}

impl Drop for SaveDiscoveryManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run_session(
    child: &mut Child,
    process_started_at: Instant,
    session: SessionResources,
    timings: LifecycleTimings,
) {
    loop {
        match session.control_rx.try_recv() {
            Ok(SessionControl::StopAndAnalyze) => {
                session.analyze();
                return;
            }
            Ok(SessionControl::Cancel) => {
                session.cancel();
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                session.cancel();
                return;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }

        session.publish_progress();
        if let Some(error) = session.fatal_error() {
            session.fail(error);
            return;
        }

        match child.try_wait() {
            Ok(Some(_)) if process_started_at.elapsed() < timings.launcher_threshold => {
                update_status(&session.status, &session.emitter, |current| {
                    current.launcher_fallback = true;
                    current.pid = None;
                });
                run_launcher_fallback(session, timings.poll_interval);
                return;
            }
            Ok(Some(_)) => {
                update_status(&session.status, &session.emitter, |current| {
                    current.phase = SaveDiscoveryPhase::ExitGracePeriod;
                    current.pid = None;
                });
                run_grace_period(session, timings);
                return;
            }
            Ok(None) => thread::sleep(timings.poll_interval),
            Err(error) => {
                session.fail(format!("检查游戏进程状态失败：{error}"));
                return;
            }
        }
    }
}

fn run_launcher_fallback(session: SessionResources, poll_interval: Duration) {
    loop {
        session.publish_progress();
        if let Some(error) = session.fatal_error() {
            session.fail(error);
            return;
        }
        match session.control_rx.recv_timeout(poll_interval) {
            Ok(SessionControl::StopAndAnalyze) => {
                session.analyze();
                return;
            }
            Ok(SessionControl::Cancel) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                session.cancel();
                return;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn run_grace_period(session: SessionResources, timings: LifecycleTimings) {
    let deadline = Instant::now() + timings.exit_grace_period;
    loop {
        session.publish_progress();
        if let Some(error) = session.fatal_error() {
            session.fail(error);
            return;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            session.analyze();
            return;
        }
        match session
            .control_rx
            .recv_timeout(remaining.min(timings.poll_interval))
        {
            Ok(SessionControl::StopAndAnalyze) => {
                session.analyze();
                return;
            }
            Ok(SessionControl::Cancel) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                session.cancel();
                return;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn analyze_and_publish(
    watchers: NativeWatchSession,
    collector: Arc<EventCollector>,
    health: Arc<WatchHealth>,
    status: Arc<Mutex<SaveDiscoveryStatus>>,
    emitter: StatusEmitter,
    analysis_context: SaveActivityAnalysisContext,
) {
    update_status(&status, &emitter, |current| {
        current.phase = SaveDiscoveryPhase::Analyzing;
        current.pid = None;
    });
    watchers.stop();
    let events = collector.events();
    let dropped = collector.dropped();
    let mut errors = health.errors();
    if dropped > 0 {
        errors.push(format!(
            "文件变化事件超过内存上限，已丢弃 {dropped} 条；本次结果不完整"
        ));
    }
    let candidates = analyze_save_activity(&events, &analysis_context);
    update_status(&status, &emitter, |current| {
        current.phase = SaveDiscoveryPhase::AwaitingConfirmation;
        current.incomplete = health.is_incomplete() || dropped > 0;
        current.event_count = events.len();
        current.dropped_event_count = dropped;
        current.candidates = candidates;
        current.errors = errors;
    });
}

fn cancel_and_publish(
    watchers: NativeWatchSession,
    status: &Arc<Mutex<SaveDiscoveryStatus>>,
    emitter: &StatusEmitter,
) {
    watchers.stop();
    update_status(status, emitter, |current| {
        current.phase = SaveDiscoveryPhase::Cancelled;
        current.pid = None;
        current.candidates.clear();
    });
}

type StatusEmitter = Arc<dyn Fn(&SaveDiscoveryStatus) + Send + Sync>;

fn status_emitter(app: AppHandle) -> StatusEmitter {
    Arc::new(move |status| {
        let _ = app.emit(STATUS_EVENT, status);
    })
}

fn replace_status(
    status: &Arc<Mutex<SaveDiscoveryStatus>>,
    emitter: &StatusEmitter,
    next: SaveDiscoveryStatus,
) {
    if let Ok(mut current) = status.lock() {
        *current = next;
        emitter(&current);
    }
}

fn update_status(
    status: &Arc<Mutex<SaveDiscoveryStatus>>,
    emitter: &StatusEmitter,
    update: impl FnOnce(&mut SaveDiscoveryStatus),
) {
    if let Ok(mut current) = status.lock() {
        update(&mut current);
        emitter(&current);
    }
}

fn publish_observed_progress(
    status: &Arc<Mutex<SaveDiscoveryStatus>>,
    emitter: &StatusEmitter,
    collector: &EventCollector,
    health: &WatchHealth,
) {
    let event_count = collector.len();
    let dropped = collector.dropped();
    let incomplete = health.is_incomplete() || dropped > 0;
    if let Ok(mut current) = status.lock() {
        if current.event_count == event_count
            && current.dropped_event_count == dropped
            && current.incomplete == incomplete
        {
            return;
        }
        current.event_count = event_count;
        current.dropped_event_count = dropped;
        current.incomplete = incomplete;
        emitter(&current);
    }
}

fn publish_failure(
    status: &Arc<Mutex<SaveDiscoveryStatus>>,
    emitter: &StatusEmitter,
    error: String,
    incomplete: bool,
) {
    update_status(status, emitter, |current| {
        current.phase = SaveDiscoveryPhase::Failed;
        current.pid = None;
        current.incomplete = incomplete;
        current.candidates.clear();
        if !current.errors.contains(&error) {
            current.errors.push(error);
        }
    });
}

fn reap_finished_worker(inner: &mut ManagerInner) {
    if inner.worker.as_ref().is_some_and(JoinHandle::is_finished) {
        if let Some(worker) = inner.worker.take() {
            let _ = worker.join();
        }
        inner.control = None;
    }
}

fn validate_launch_binding(binding: &GameLaunchBinding) -> Result<(), String> {
    if !binding.executable_path.is_file() {
        return Err(format!(
            "游戏启动程序不存在：{}",
            display_path(&binding.executable_path).display()
        ));
    }
    if !binding.install_dir.is_dir() {
        return Err(format!(
            "游戏安装目录不存在：{}",
            display_path(&binding.install_dir).display()
        ));
    }
    Ok(())
}

fn launch_game(binding: &GameLaunchBinding) -> Result<Child, String> {
    Command::new(&binding.executable_path)
        .args(&binding.launch_arguments)
        .current_dir(&binding.install_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            format!(
                "启动游戏失败（{}）：{error}",
                display_path(&binding.executable_path).display()
            )
        })
}

fn discovery_roots(binding: &GameLaunchBinding) -> Result<Vec<PathBuf>, String> {
    let mut candidates = vec![display_path(&binding.install_dir)];
    if let Some(local_app_data) = env_absolute_path("LOCALAPPDATA") {
        candidates.push(local_app_data);
    }
    if let Some(profile) = env_absolute_path("USERPROFILE") {
        candidates.push(profile.join("AppData").join("LocalLow"));
    }
    if let Some(app_data) = env_absolute_path("APPDATA") {
        candidates.push(app_data);
    }
    candidates.extend(known_save_folders());

    let mut roots: Vec<PathBuf> = Vec::new();
    for candidate in candidates {
        if !candidate.is_dir() {
            continue;
        }
        let display = display_path(&candidate);
        if roots
            .iter()
            .any(|existing| normalized_path(existing) == normalized_path(&display))
        {
            continue;
        }
        roots.push(display);
    }
    if !roots
        .iter()
        .any(|root| normalized_path(root) == normalized_path(&binding.install_dir))
    {
        return Err("游戏安装目录不可用于活动监测".into());
    }
    Ok(roots)
}

fn env_absolute_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn normalized_path(path: &Path) -> String {
    display_path(path)
        .to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn display_path(path: &Path) -> PathBuf {
    let value = path.to_string_lossy();
    let lowercase = value.to_ascii_lowercase();
    if lowercase.starts_with("\\\\?\\unc\\") {
        return PathBuf::from(format!("\\\\{}", &value[8..]));
    }
    if lowercase.starts_with("\\\\?\\") {
        return PathBuf::from(&value[4..]);
    }
    path.to_path_buf()
}

#[derive(Default)]
struct WatchHealth {
    incomplete: AtomicBool,
    errors: Mutex<Vec<String>>,
}

impl WatchHealth {
    fn mark_overflow(&self) {
        self.incomplete.store(true, Ordering::SeqCst);
    }

    fn mark_error(&self, error: String) {
        self.incomplete.store(true, Ordering::SeqCst);
        if let Ok(mut errors) = self.errors.lock() {
            errors.push(error);
        }
    }

    fn is_incomplete(&self) -> bool {
        self.incomplete.load(Ordering::SeqCst)
    }

    fn has_fatal_error(&self) -> bool {
        self.errors
            .lock()
            .map(|errors| !errors.is_empty())
            .unwrap_or(true)
    }

    fn errors(&self) -> Vec<String> {
        self.errors
            .lock()
            .map(|errors| errors.clone())
            .unwrap_or_else(|_| vec!["目录监听错误状态锁已损坏".into()])
    }
}

struct EventCollector {
    events: Mutex<Vec<FileActivityEvent>>,
    event_limit: usize,
    dropped: AtomicUsize,
    excluded_roots: Vec<PathBuf>,
}

impl EventCollector {
    fn new(event_limit: usize, excluded_roots: Vec<PathBuf>) -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            event_limit,
            dropped: AtomicUsize::new(0),
            excluded_roots,
        }
    }

    fn push(&self, event: FileActivityEvent) {
        if self
            .excluded_roots
            .iter()
            .any(|root| path_is_same_or_descendant(root, &event.path))
        {
            return;
        }
        let Ok(mut events) = self.events.lock() else {
            self.dropped.fetch_add(1, Ordering::SeqCst);
            return;
        };
        if events.len() >= self.event_limit {
            self.dropped.fetch_add(1, Ordering::SeqCst);
            return;
        }
        events.push(event);
    }

    fn len(&self) -> usize {
        self.events.lock().map(|events| events.len()).unwrap_or(0)
    }

    fn events(&self) -> Vec<FileActivityEvent> {
        self.events
            .lock()
            .map(|events| events.clone())
            .unwrap_or_default()
    }

    fn dropped(&self) -> usize {
        self.dropped.load(Ordering::SeqCst)
    }
}

#[cfg(windows)]
mod windows_native {
    use super::*;
    use std::ffi::{c_void, OsString};
    use std::mem;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::ptr;
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, GetLastError, ERROR_IO_PENDING, ERROR_NOTIFY_ENUM_DIR,
            ERROR_OPERATION_ABORTED, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
        },
        Storage::FileSystem::{
            CreateFileW, ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED,
            FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_CREATION, FILE_NOTIFY_CHANGE_DIR_NAME,
            FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        },
        System::{
            Com::CoTaskMemFree,
            Threading::{CreateEventW, ResetEvent, SetEvent, WaitForMultipleObjects, INFINITE},
            IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        },
        UI::Shell::{
            FOLDERID_Documents, FOLDERID_SavedGames, SHGetKnownFolderPath, KF_FLAG_DEFAULT,
        },
    };

    pub(super) struct NativeWatchSession {
        watchers: Vec<NativeWatcher>,
    }

    impl NativeWatchSession {
        pub(super) fn start(
            roots: &[PathBuf],
            collector: Arc<EventCollector>,
            health: Arc<WatchHealth>,
            buffer_size: u32,
        ) -> Result<Self, String> {
            let mut watchers = Vec::new();
            for root in roots {
                match NativeWatcher::start(root, collector.clone(), health.clone(), buffer_size) {
                    Ok(watcher) => watchers.push(watcher),
                    Err(error) => {
                        for watcher in watchers {
                            watcher.join();
                        }
                        return Err(error);
                    }
                }
            }
            Ok(Self { watchers })
        }

        pub(super) fn stop(self) {
            for watcher in &self.watchers {
                watcher.request_stop();
            }
            for watcher in self.watchers {
                watcher.join();
            }
        }
    }

    struct NativeWatcher {
        stop_event: usize,
        worker: Option<JoinHandle<()>>,
    }

    impl NativeWatcher {
        fn start(
            root: &Path,
            collector: Arc<EventCollector>,
            health: Arc<WatchHealth>,
            requested_buffer_size: u32,
        ) -> Result<Self, String> {
            if requested_buffer_size < 4 {
                return Err("ReadDirectoryChangesW 缓冲区至少需要 4 字节".into());
            }
            let canonical_root = std::fs::canonicalize(root).map_err(|error| {
                format!(
                    "无法规范化监测目录 {}：{error}",
                    display_path(root).display()
                )
            })?;
            let display_root = display_path(&canonical_root);
            let stop_event = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
            if stop_event.is_null() {
                return Err(format!(
                    "无法创建目录监听停止事件：Windows error {}",
                    unsafe { GetLastError() }
                ));
            }
            let stop_event_value = stop_event as usize;
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            let thread_root = display_root.clone();
            let worker = match thread::Builder::new()
                .name("save-discovery-directory-watch".into())
                .spawn(move || {
                    run_watch_worker(
                        canonical_root,
                        thread_root,
                        requested_buffer_size,
                        stop_event_value,
                        collector,
                        health,
                        ready_tx,
                    );
                }) {
                Ok(worker) => worker,
                Err(error) => {
                    unsafe {
                        CloseHandle(stop_event_value as HANDLE);
                    }
                    return Err(format!("无法启动目录监听线程：{error}"));
                }
            };
            match ready_rx.recv_timeout(READY_TIMEOUT) {
                Ok(Ok(())) => Ok(Self {
                    stop_event: stop_event_value,
                    worker: Some(worker),
                }),
                Ok(Err(error)) => {
                    unsafe {
                        SetEvent(stop_event_value as HANDLE);
                    }
                    let _ = worker.join();
                    unsafe {
                        CloseHandle(stop_event_value as HANDLE);
                    }
                    Err(format!(
                        "监测目录 {} 启动失败：{error}",
                        display_root.display()
                    ))
                }
                Err(error) => {
                    unsafe {
                        SetEvent(stop_event_value as HANDLE);
                    }
                    let _ = worker.join();
                    unsafe {
                        CloseHandle(stop_event_value as HANDLE);
                    }
                    Err(format!(
                        "等待监测目录 {} 就绪超时：{error}",
                        display_root.display()
                    ))
                }
            }
        }

        fn request_stop(&self) {
            if self.stop_event != 0 {
                unsafe {
                    SetEvent(self.stop_event as HANDLE);
                }
            }
        }

        fn join(mut self) {
            self.request_stop();
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            if self.stop_event != 0 {
                unsafe {
                    CloseHandle(self.stop_event as HANDLE);
                }
                self.stop_event = 0;
            }
        }
    }

    impl Drop for NativeWatcher {
        fn drop(&mut self) {
            self.request_stop();
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            if self.stop_event != 0 {
                unsafe {
                    CloseHandle(self.stop_event as HANDLE);
                }
                self.stop_event = 0;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn run_watch_worker(
        canonical_root: PathBuf,
        display_root: PathBuf,
        requested_buffer_size: u32,
        stop_event: usize,
        collector: Arc<EventCollector>,
        health: Arc<WatchHealth>,
        ready_tx: mpsc::SyncSender<Result<(), String>>,
    ) {
        let path_wide = canonical_root
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let directory = unsafe {
            CreateFileW(
                path_wide.as_ptr(),
                FILE_LIST_DIRECTORY,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                ptr::null_mut(),
            )
        };
        if directory == INVALID_HANDLE_VALUE {
            let _ = ready_tx.send(Err(format!("CreateFileW 返回 Windows error {}", unsafe {
                GetLastError()
            })));
            return;
        }
        let directory = OwnedHandle(directory as usize);
        let io_event = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
        if io_event.is_null() {
            let _ = ready_tx.send(Err(format!(
                "创建 I/O 完成事件失败：Windows error {}",
                unsafe { GetLastError() }
            )));
            return;
        }
        let io_event = OwnedHandle(io_event as usize);
        let buffer_words = requested_buffer_size.div_ceil(4) as usize;
        let mut buffer = vec![0_u32; buffer_words];
        let actual_buffer_size = (buffer.len() * mem::size_of::<u32>()) as u32;
        let mut first_request = true;

        loop {
            unsafe {
                ResetEvent(io_event.raw());
            }
            let mut overlapped: OVERLAPPED = unsafe { mem::zeroed() };
            overlapped.hEvent = io_event.raw();
            let mut ignored_bytes = 0_u32;
            let started = unsafe {
                ReadDirectoryChangesW(
                    directory.raw(),
                    buffer.as_mut_ptr() as *mut c_void,
                    actual_buffer_size,
                    1,
                    FILE_NOTIFY_CHANGE_FILE_NAME
                        | FILE_NOTIFY_CHANGE_DIR_NAME
                        | FILE_NOTIFY_CHANGE_SIZE
                        | FILE_NOTIFY_CHANGE_LAST_WRITE
                        | FILE_NOTIFY_CHANGE_CREATION,
                    &mut ignored_bytes,
                    &mut overlapped,
                    None,
                )
            };
            if started == 0 {
                let error = unsafe { GetLastError() };
                if error != ERROR_IO_PENDING {
                    let message = format!("ReadDirectoryChangesW 返回 Windows error {error}");
                    if first_request {
                        let _ = ready_tx.send(Err(message));
                    } else {
                        health.mark_error(format!("{}：{message}", display_root.display()));
                    }
                    return;
                }
            }
            if first_request {
                if ready_tx.send(Ok(())).is_err() {
                    return;
                }
                first_request = false;
            }

            let handles = [stop_event as HANDLE, io_event.raw()];
            let wait = unsafe {
                WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, INFINITE)
            };
            if wait == WAIT_OBJECT_0 {
                unsafe {
                    CancelIoEx(directory.raw(), &overlapped);
                    let mut ignored = 0_u32;
                    GetOverlappedResult(directory.raw(), &overlapped, &mut ignored, 1);
                }
                return;
            }
            if wait != WAIT_OBJECT_0 + 1 {
                health.mark_error(format!(
                    "等待目录 {} 的文件事件失败：Windows wait result {wait}",
                    display_root.display()
                ));
                return;
            }

            let mut bytes_written = 0_u32;
            let completed =
                unsafe { GetOverlappedResult(directory.raw(), &overlapped, &mut bytes_written, 0) };
            if completed == 0 {
                let error = unsafe { GetLastError() };
                if error == ERROR_NOTIFY_ENUM_DIR {
                    health.mark_overflow();
                    continue;
                }
                if error == ERROR_OPERATION_ABORTED {
                    return;
                }
                health.mark_error(format!(
                    "读取目录 {} 的文件事件失败：Windows error {error}",
                    display_root.display()
                ));
                return;
            }
            if bytes_written == 0 {
                health.mark_overflow();
                continue;
            }
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    buffer.as_ptr() as *const u8,
                    actual_buffer_size as usize,
                )
            };
            match parse_events(bytes, bytes_written as usize) {
                Ok(events) => {
                    let observed_at_unix_ms = now_unix_ms();
                    for (relative_path, kind) in events {
                        collector.push(FileActivityEvent {
                            path: display_root.join(relative_path),
                            kind,
                            observed_at_unix_ms,
                        });
                    }
                }
                Err(error) => {
                    health.mark_error(format!(
                        "解析目录 {} 的文件事件失败：{error}",
                        display_root.display()
                    ));
                    return;
                }
            }
        }
    }

    fn parse_events(
        buffer: &[u8],
        bytes_written: usize,
    ) -> Result<Vec<(PathBuf, FileActivityKind)>, String> {
        if bytes_written > buffer.len() {
            return Err("ReadDirectoryChangesW 返回长度超过缓冲区".into());
        }
        let mut offset = 0_usize;
        let mut events = Vec::new();
        loop {
            if offset + 12 > bytes_written {
                return Err("FILE_NOTIFY_INFORMATION 头部不完整".into());
            }
            let next_offset = u32::from_ne_bytes(
                buffer[offset..offset + 4]
                    .try_into()
                    .map_err(|_| "下一条事件偏移无效")?,
            ) as usize;
            let action = u32::from_ne_bytes(
                buffer[offset + 4..offset + 8]
                    .try_into()
                    .map_err(|_| "事件动作无效")?,
            );
            let name_bytes = u32::from_ne_bytes(
                buffer[offset + 8..offset + 12]
                    .try_into()
                    .map_err(|_| "文件名长度无效")?,
            ) as usize;
            if !name_bytes.is_multiple_of(2) || offset + 12 + name_bytes > bytes_written {
                return Err("FILE_NOTIFY_INFORMATION 文件名越界或未按 UTF-16 对齐".into());
            }
            let name_start = offset + 12;
            let name_wide = buffer[name_start..name_start + name_bytes]
                .chunks_exact(2)
                .map(|bytes| u16::from_ne_bytes([bytes[0], bytes[1]]))
                .collect::<Vec<_>>();
            if let Some(kind) = activity_kind(action) {
                events.push((OsString::from_wide(&name_wide).into(), kind));
            }
            if next_offset == 0 {
                break;
            }
            if next_offset < 12 || offset + next_offset >= bytes_written {
                return Err("FILE_NOTIFY_INFORMATION 下一条事件偏移无效".into());
            }
            offset += next_offset;
        }
        Ok(events)
    }

    fn activity_kind(action: u32) -> Option<FileActivityKind> {
        match action {
            1 => Some(FileActivityKind::Create),
            2 => Some(FileActivityKind::Delete),
            3 => Some(FileActivityKind::Modify),
            4 => Some(FileActivityKind::RenameFrom),
            5 => Some(FileActivityKind::RenameTo),
            _ => None,
        }
    }

    struct OwnedHandle(usize);

    impl OwnedHandle {
        fn raw(&self) -> HANDLE {
            self.0 as HANDLE
        }
    }

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.raw());
            }
        }
    }

    pub(super) fn known_save_folders() -> Vec<PathBuf> {
        [FOLDERID_Documents, FOLDERID_SavedGames]
            .into_iter()
            .filter_map(known_folder)
            .collect()
    }

    fn known_folder(folder_id: windows_sys::core::GUID) -> Option<PathBuf> {
        unsafe {
            let mut path_ptr = ptr::null_mut();
            let result = SHGetKnownFolderPath(
                &folder_id,
                KF_FLAG_DEFAULT as u32,
                ptr::null_mut(),
                &mut path_ptr,
            );
            if result != 0 || path_ptr.is_null() {
                if !path_ptr.is_null() {
                    CoTaskMemFree(path_ptr as *const c_void);
                }
                return None;
            }
            let mut len = 0_usize;
            while *path_ptr.add(len) != 0 {
                len += 1;
            }
            let path = PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(
                path_ptr, len,
            )));
            CoTaskMemFree(path_ptr as *const c_void);
            Some(path)
        }
    }
}

#[cfg(windows)]
use windows_native::{known_save_folders, NativeWatchSession};

#[cfg(not(windows))]
struct NativeWatchSession;

#[cfg(not(windows))]
impl NativeWatchSession {
    fn start(
        _roots: &[PathBuf],
        _collector: Arc<EventCollector>,
        _health: Arc<WatchHealth>,
        _buffer_size: u32,
    ) -> Result<Self, String> {
        Err("存档动态发现目前只支持 Windows".into())
    }

    fn stop(self) {}
}

#[cfg(not(windows))]
fn known_save_folders() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_root(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "savelink-save-discovery-{name}-{}-{}",
            std::process::id(),
            now_unix_ms()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn test_manager() -> SaveDiscoveryManager {
        SaveDiscoveryManager {
            inner: Mutex::new(ManagerInner::default()),
            status: Arc::new(Mutex::new(SaveDiscoveryStatus::default())),
            timings: LifecycleTimings {
                launcher_threshold: Duration::from_millis(200),
                exit_grace_period: Duration::from_millis(120),
                poll_interval: Duration::from_millis(20),
            },
        }
    }

    fn powershell_binding(root: &Path, script: String) -> GameLaunchBinding {
        let powershell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        GameLaunchBinding {
            executable_path: powershell,
            install_dir: root.to_path_buf(),
            launch_arguments: vec!["-NoProfile".into(), "-Command".into(), script],
            steam_app_id: None,
        }
    }

    fn request(root: &Path, binding: GameLaunchBinding) -> SaveDiscoveryStartRequest {
        SaveDiscoveryStartRequest {
            game_id: "test-game".into(),
            game_name: "Test Game".into(),
            launch_binding: binding,
            data_dir: root.join("savelink-data"),
            app_local_data_dir: root.join("savelink-webview-data"),
            repository_dir: root.join("savelink-data").join("repository"),
        }
    }

    fn wait_for_phase(
        manager: &SaveDiscoveryManager,
        phase: SaveDiscoveryPhase,
    ) -> SaveDiscoveryStatus {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = manager.status().unwrap();
            if status.phase == phase {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "等待状态 {phase:?} 超时，当前状态 {:?}，错误 {:?}",
                status.phase,
                status.errors
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn native_watcher_captures_unicode_events_and_stops_without_new_activity() {
        let root = temp_root("native").join("中文目录");
        fs::create_dir_all(&root).unwrap();
        let collector = Arc::new(EventCollector::new(100, Vec::new()));
        let health = Arc::new(WatchHealth::default());
        let session = NativeWatchSession::start(
            std::slice::from_ref(&root),
            collector.clone(),
            health.clone(),
            16 * 1024,
        )
        .unwrap();
        let save = root.join("第一存档.hole");
        fs::write(&save, b"v1").unwrap();
        thread::sleep(Duration::from_millis(250));
        let started = Instant::now();
        session.stop();

        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(collector.events().iter().any(|event| event.path == save));
        assert!(!health.is_incomplete());
        let _ = fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn collector_marks_events_over_the_limit_as_dropped() {
        let collector = EventCollector::new(1, Vec::new());
        for index in 0..2 {
            collector.push(FileActivityEvent {
                path: PathBuf::from(format!(r"C:\Saves\slot-{index}.dat")),
                kind: FileActivityKind::Modify,
                observed_at_unix_ms: index,
            });
        }

        assert_eq!(collector.len(), 1);
        assert_eq!(collector.dropped(), 1);
    }

    #[test]
    fn active_session_rejects_a_second_session() {
        let root = temp_root("single-session");
        fs::create_dir_all(&root).unwrap();
        let binding = powershell_binding(&root, "Start-Sleep -Seconds 5".into());
        let manager = test_manager();

        manager
            .start_with_roots_and_emitter(
                request(&root, binding.clone()),
                vec![root.clone()],
                Arc::new(|_| {}),
            )
            .unwrap();
        let error = manager
            .start_with_roots_and_emitter(
                request(&root, binding),
                vec![root.clone()],
                Arc::new(|_| {}),
            )
            .unwrap_err();

        assert!(error.contains("同一时间只能监测一个游戏"));
        manager.cancel().unwrap();
        wait_for_phase(&manager, SaveDiscoveryPhase::Cancelled);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cancellation_discards_candidates_and_stops_the_session() {
        let root = temp_root("cancel-session");
        let save = root.join("Save").join("slot.dat");
        fs::create_dir_all(save.parent().unwrap()).unwrap();
        let script = format!(
            "Set-Content -LiteralPath '{}' -Value 'v1'; Start-Sleep -Seconds 5",
            save.display()
        );
        let manager = test_manager();

        manager
            .start_with_roots_and_emitter(
                request(&root, powershell_binding(&root, script)),
                vec![root.clone()],
                Arc::new(|_| {}),
            )
            .unwrap();
        wait_for_phase(&manager, SaveDiscoveryPhase::Monitoring);
        manager.cancel().unwrap();
        let status = wait_for_phase(&manager, SaveDiscoveryPhase::Cancelled);

        assert!(status.candidates.is_empty());
        assert!(status.pid.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn normal_process_exit_runs_grace_period_and_analyzes() {
        let root = temp_root("normal-process");
        let save = root.join("Save").join("progress.hole");
        fs::create_dir_all(save.parent().unwrap()).unwrap();
        let script = format!(
            "Set-Content -LiteralPath '{}' -Value 'v1'; Start-Sleep -Milliseconds 350",
            save.display()
        );
        let binding = powershell_binding(&root, script);
        let manager = test_manager();

        manager
            .start_with_roots_and_emitter(
                request(&root, binding),
                vec![root.clone()],
                Arc::new(|_| {}),
            )
            .unwrap();
        let status = wait_for_phase(&manager, SaveDiscoveryPhase::AwaitingConfirmation);

        assert!(!status.launcher_fallback);
        assert!(!status.incomplete);
        assert!(status.event_count > 0);
        assert!(status
            .candidates
            .iter()
            .any(|candidate| candidate.directory == root.join("Save")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fast_launcher_keeps_monitoring_until_manual_stop() {
        let root = temp_root("fast-launcher");
        let save = root.join("Save").join("slot.dat");
        fs::create_dir_all(save.parent().unwrap()).unwrap();
        let script = format!("Set-Content -LiteralPath '{}' -Value 'v1'", save.display());
        let binding = powershell_binding(&root, script);
        let mut manager = test_manager();
        manager.timings.launcher_threshold = Duration::from_secs(2);

        manager
            .start_with_roots_and_emitter(
                request(&root, binding),
                vec![root.clone()],
                Arc::new(|_| {}),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = manager.status().unwrap();
            if status.phase == SaveDiscoveryPhase::Monitoring && status.launcher_fallback {
                break;
            }
            assert!(Instant::now() < deadline, "未进入快速启动器兜底状态");
            thread::sleep(Duration::from_millis(20));
        }
        manager.stop_and_analyze().unwrap();
        let status = wait_for_phase(&manager, SaveDiscoveryPhase::AwaitingConfirmation);

        assert!(status.launcher_fallback);
        assert!(status.event_count > 0);
        assert!(!status.candidates.is_empty());
        let _ = fs::remove_dir_all(root);
    }
}
