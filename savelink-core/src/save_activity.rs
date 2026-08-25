//! Platform-independent aggregation and ranking for save discovery activity.

use crate::scan::path_is_same_or_descendant;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const DEDUP_WINDOW_MS: u64 = 250;
const MAX_CANDIDATES: usize = 50;
const MAX_FILES_PER_CANDIDATE: usize = 20;
const MAX_OBSERVED_FILES_PER_DIRECTORY: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileActivityKind {
    Create,
    Modify,
    Delete,
    RenameFrom,
    RenameTo,
    Observed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileActivityEvent {
    pub path: PathBuf,
    pub kind: FileActivityKind,
    pub observed_at_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveActivityAnalysisContext {
    pub game_name: String,
    pub executable_stem: Option<String>,
    pub install_dir: Option<PathBuf>,
    pub watched_roots: Vec<PathBuf>,
    pub excluded_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveCandidateConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityFileSummary {
    pub path: PathBuf,
    pub kinds: Vec<FileActivityKind>,
    pub event_count: usize,
    pub last_activity_unix_ms: u64,
    pub exists_after_monitoring: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveDirectoryCandidate {
    pub directory: PathBuf,
    pub confidence: SaveCandidateConfidence,
    pub score: i32,
    pub confirmable: bool,
    pub unsafe_reason: Option<String>,
    pub event_count: usize,
    pub distinct_file_count: usize,
    pub last_activity_unix_ms: u64,
    pub files: Vec<ActivityFileSummary>,
    pub positive_signals: Vec<String>,
    pub downgrade_reasons: Vec<String>,
}

#[derive(Debug)]
struct AggregatedFile {
    path: PathBuf,
    kinds: BTreeSet<FileActivityKind>,
    event_count: usize,
    last_activity_unix_ms: u64,
}

#[derive(Debug)]
struct CandidateGroup {
    directory: PathBuf,
    files: Vec<AggregatedFile>,
}

/// Merge burst duplicates, group activity by direct parent, and rank every candidate.
///
/// Unknown extensions are deliberately retained. Only paths under `excluded_roots`
/// are removed; noisy logs and caches remain visible with a lower score.
pub fn analyze_save_activity(
    events: &[FileActivityEvent],
    context: &SaveActivityAnalysisContext,
) -> Vec<SaveDirectoryCandidate> {
    let mut sorted = events
        .iter()
        .filter(|event| !is_excluded(&event.path, &context.excluded_roots))
        .flat_map(expand_directory_event)
        .collect::<Vec<_>>();
    sorted.sort_by(|left, right| {
        left.observed_at_unix_ms
            .cmp(&right.observed_at_unix_ms)
            .then_with(|| normalized_path(&left.path).cmp(&normalized_path(&right.path)))
            .then_with(|| left.kind.cmp(&right.kind))
    });

    let mut files = BTreeMap::<String, AggregatedFile>::new();
    let mut last_seen = BTreeMap::<(String, FileActivityKind), u64>::new();
    for event in sorted {
        let key = normalized_path(&event.path);
        let dedup_key = (key.clone(), event.kind);
        if last_seen
            .get(&dedup_key)
            .is_some_and(|last| event.observed_at_unix_ms.saturating_sub(*last) <= DEDUP_WINDOW_MS)
        {
            continue;
        }
        last_seen.insert(dedup_key, event.observed_at_unix_ms);
        let file = files.entry(key).or_insert_with(|| AggregatedFile {
            path: event.path.clone(),
            kinds: BTreeSet::new(),
            event_count: 0,
            last_activity_unix_ms: 0,
        });
        file.kinds.insert(event.kind);
        file.event_count += 1;
        file.last_activity_unix_ms = file.last_activity_unix_ms.max(event.observed_at_unix_ms);
    }

    let mut groups = BTreeMap::<String, CandidateGroup>::new();
    for file in files.into_values() {
        let Some(parent) = file
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        else {
            continue;
        };
        let directory = parent.to_path_buf();
        groups
            .entry(normalized_path(&directory))
            .or_insert_with(|| CandidateGroup {
                directory,
                files: Vec::new(),
            })
            .files
            .push(file);
    }

    let mut candidates = groups
        .into_values()
        .map(|group| rank_group(group, context))
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .confirmable
            .cmp(&left.confirmable)
            .then_with(|| confidence_rank(right.confidence).cmp(&confidence_rank(left.confidence)))
            .then_with(|| right.score.cmp(&left.score))
            .then_with(|| right.last_activity_unix_ms.cmp(&left.last_activity_unix_ms))
            .then_with(|| normalized_path(&left.directory).cmp(&normalized_path(&right.directory)))
    });
    candidates.truncate(MAX_CANDIDATES);
    candidates
}

fn expand_directory_event(event: &FileActivityEvent) -> Vec<FileActivityEvent> {
    if !event.path.is_dir() {
        return vec![event.clone()];
    }

    let Ok(entries) = std::fs::read_dir(&event.path) else {
        return Vec::new();
    };
    let mut files = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|file_type| file_type.is_file())
                .map(|_| entry.path())
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|path| normalized_path(path));
    files
        .into_iter()
        .take(MAX_OBSERVED_FILES_PER_DIRECTORY)
        .map(|path| FileActivityEvent {
            path,
            kind: FileActivityKind::Observed,
            observed_at_unix_ms: event.observed_at_unix_ms,
        })
        .collect()
}

fn confidence_rank(confidence: SaveCandidateConfidence) -> u8 {
    match confidence {
        SaveCandidateConfidence::High => 3,
        SaveCandidateConfidence::Medium => 2,
        SaveCandidateConfidence::Low => 1,
    }
}

fn rank_group(
    mut group: CandidateGroup,
    context: &SaveActivityAnalysisContext,
) -> SaveDirectoryCandidate {
    group.files.sort_by(|left, right| {
        right
            .last_activity_unix_ms
            .cmp(&left.last_activity_unix_ms)
            .then_with(|| normalized_path(&left.path).cmp(&normalized_path(&right.path)))
    });
    let event_count = group.files.iter().map(|file| file.event_count).sum();
    let distinct_file_count = group.files.len();
    let last_activity_unix_ms = group
        .files
        .iter()
        .map(|file| file.last_activity_unix_ms)
        .max()
        .unwrap_or(0);
    let mut score = (distinct_file_count.min(5) as i32) * 5;
    let mut positive_signals = Vec::new();
    let mut downgrade_reasons = Vec::new();

    if distinct_file_count >= 2 {
        score += 8;
        positive_signals.push("同一目录有多个文件发生变化".into());
    }
    if group.files.iter().any(|file| {
        file.kinds.contains(&FileActivityKind::Create)
            || file.kinds.contains(&FileActivityKind::RenameTo)
    }) {
        score += 5;
        positive_signals.push("本次游玩创建了新文件".into());
    }
    if group
        .files
        .iter()
        .any(|file| file.kinds.contains(&FileActivityKind::Modify))
    {
        score += 4;
        positive_signals.push("本次游玩修改了已有文件".into());
    }
    let save_path_hint = has_save_path_hint(&group.directory);
    if save_path_hint {
        score += 10;
        positive_signals.push("路径包含常见存档层级".into());
    }
    let game_identity_match = path_matches_game_identity(&group.directory, context);
    if game_identity_match {
        score += 12;
        positive_signals.push("路径与游戏名称或程序名称相关".into());
    }
    let companion_backup_pair = has_companion_backup_pair(&group.files);
    if companion_backup_pair {
        score += 8;
        positive_signals.push("同目录出现主文件和备份文件".into());
    }
    let has_domain_signal = save_path_hint || game_identity_match || companion_backup_pair;
    if !has_domain_signal {
        downgrade_reasons.push("路径与当前游戏或常见存档结构缺少直接关联".into());
    }

    let noisy_files = group
        .files
        .iter()
        .filter(|file| is_noisy_file(&file.path))
        .count();
    let only_noisy_files = noisy_files == distinct_file_count && distinct_file_count > 0;
    if only_noisy_files {
        score -= 18;
        downgrade_reasons.push("仅检测到日志、运行时或统计文件".into());
    } else if noisy_files > 0 {
        score -= 4;
        downgrade_reasons.push("部分变化来自日志或运行时文件".into());
    }
    let noisy_directory = has_noise_path_component(&group.directory);
    if noisy_directory {
        score -= 14;
        downgrade_reasons.push("路径位于缓存、日志、临时或崩溃数据目录".into());
    }
    if contains_only_configuration_files(&group.files) {
        score -= 8;
        downgrade_reasons.push("仅检测到配置文件变化".into());
    }

    let unsafe_reason = unsafe_candidate_reason(&group.directory, context);
    let confirmable = unsafe_reason.is_none();
    if let Some(reason) = unsafe_reason.as_ref() {
        score -= 25;
        downgrade_reasons.push(reason.clone());
    }
    let confidence = if !confirmable || noisy_directory || only_noisy_files || !has_domain_signal {
        SaveCandidateConfidence::Low
    } else if score >= 30 {
        SaveCandidateConfidence::High
    } else if score >= 15 {
        SaveCandidateConfidence::Medium
    } else {
        SaveCandidateConfidence::Low
    };

    let files = group
        .files
        .into_iter()
        .take(MAX_FILES_PER_CANDIDATE)
        .map(|file| ActivityFileSummary {
            exists_after_monitoring: file.path.exists(),
            path: file.path,
            kinds: file.kinds.into_iter().collect(),
            event_count: file.event_count,
            last_activity_unix_ms: file.last_activity_unix_ms,
        })
        .collect();
    SaveDirectoryCandidate {
        directory: group.directory,
        confidence,
        score,
        confirmable,
        unsafe_reason,
        event_count,
        distinct_file_count,
        last_activity_unix_ms,
        files,
        positive_signals,
        downgrade_reasons,
    }
}

fn is_excluded(path: &Path, excluded_roots: &[PathBuf]) -> bool {
    excluded_roots
        .iter()
        .any(|root| path_is_same_or_descendant(root, path))
}

fn unsafe_candidate_reason(
    directory: &Path,
    context: &SaveActivityAnalysisContext,
) -> Option<String> {
    if context
        .install_dir
        .as_ref()
        .is_some_and(|install_dir| normalized_path(install_dir) == normalized_path(directory))
    {
        return Some("该目录是游戏安装根目录，不能直接作为整目录存档确认".into());
    }
    if context
        .watched_roots
        .iter()
        .any(|root| normalized_path(root) == normalized_path(directory))
    {
        return Some("该目录是监测根目录，范围过大，不能直接确认".into());
    }
    None
}

fn path_matches_game_identity(path: &Path, context: &SaveActivityAnalysisContext) -> bool {
    let compact_path = compact_identity(&path.to_string_lossy());
    let game = compact_identity(&context.game_name);
    (!game.is_empty() && compact_path.contains(&game))
        || context.executable_stem.as_ref().is_some_and(|stem| {
            let stem = compact_identity(stem);
            !stem.is_empty() && compact_path.contains(&stem)
        })
}

fn compact_identity(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn has_save_path_hint(path: &Path) -> bool {
    path.components().any(|component| {
        let value = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        [
            "save",
            "saves",
            "saved",
            "savegame",
            "savegames",
            "savedata",
            "profile",
            "profiles",
            "slot",
            "slots",
            "release",
        ]
        .contains(&value.as_str())
    })
}

fn has_noise_path_component(path: &Path) -> bool {
    path.components().any(|component| {
        let value = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        [
            "cache",
            "caches",
            "code cache",
            "ebwebview",
            "gpucache",
            "localcache",
            "log",
            "logs",
            "analytics",
            "archivedevents",
            "crashpad",
            "insights",
            "shader",
            "shaders",
            "shadervariantanalytics",
            "sentry",
            "temp",
            "tmp",
            "crash",
            "crashes",
            "crashdumps",
            "webstorage",
        ]
        .contains(&value.as_str())
    })
}

fn is_noisy_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    name == "player.log"
        || name == "player-prev.log"
        || name == "playtime.txt"
        || name == "cookies"
        || name == "cookies-journal"
        || name == "network persistent state"
        || name == "quota manager"
        || name == "quota manager-journal"
        || name == "reporting and nel"
        || name == "reporting and nel-journal"
        || name.ends_with(".log")
        || name.ends_with(".dmp")
        || name.ends_with(".tmp")
}

fn contains_only_configuration_files(files: &[AggregatedFile]) -> bool {
    !files.is_empty()
        && files.iter().all(|file| {
            matches!(
                file.path
                    .extension()
                    .map(|value| value.to_string_lossy().to_ascii_lowercase())
                    .as_deref(),
                Some("ini" | "cfg" | "conf" | "xml")
            )
        })
}

fn has_companion_backup_pair(files: &[AggregatedFile]) -> bool {
    let names = files
        .iter()
        .filter_map(|file| file.path.file_name())
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    names.iter().any(|name| {
        [".bak", ".bac", ".backup"].iter().any(|suffix| {
            name.strip_suffix(suffix)
                .is_some_and(|base| names.contains(base))
        })
    })
}

fn normalized_path(path: &Path) -> String {
    let mut normalized = path.to_string_lossy().replace('/', "\\");
    let lowercase = normalized.to_ascii_lowercase();
    if lowercase.starts_with("\\\\?\\unc\\") {
        normalized = format!("\\\\{}", &normalized[8..]);
    } else if lowercase.starts_with("\\\\?\\") {
        normalized = normalized[4..].to_string();
    }
    normalized.trim_end_matches('\\').to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> SaveActivityAnalysisContext {
        SaveActivityAnalysisContext {
            game_name: "Hole Is Mine".into(),
            executable_stem: Some("HoleIsMine".into()),
            install_dir: Some(PathBuf::from(r"C:\Games\Hole Is Mine")),
            watched_roots: vec![PathBuf::from(r"C:\Users\Tester\AppData\LocalLow")],
            excluded_roots: vec![PathBuf::from(r"C:\SaveLink")],
        }
    }

    fn event(path: &str, kind: FileActivityKind, at: u64) -> FileActivityEvent {
        FileActivityEvent {
            path: PathBuf::from(path),
            kind,
            observed_at_unix_ms: at,
        }
    }

    #[test]
    fn unknown_save_extensions_are_ranked_instead_of_filtered() {
        let events = vec![
            event(
                r"C:\Users\Tester\AppData\LocalLow\Incrementalist\Hole Is Mine\Save\slot.hole",
                FileActivityKind::Modify,
                1_000,
            ),
            event(
                r"C:\Users\Tester\AppData\LocalLow\Incrementalist\Hole Is Mine\Save\slot.hole.bac",
                FileActivityKind::Create,
                1_500,
            ),
        ];

        let candidates = analyze_save_activity(&events, &context());

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].confidence, SaveCandidateConfidence::High);
        assert!(candidates[0].confirmable);
        assert_eq!(candidates[0].distinct_file_count, 2);
    }

    #[test]
    fn logs_are_visible_but_rank_below_progress_files() {
        let events = vec![
            event(
                r"C:\Users\Tester\AppData\LocalLow\Studio\Hole Is Mine\Logs\Player.log",
                FileActivityKind::Modify,
                1_000,
            ),
            event(
                r"C:\Users\Tester\AppData\Roaming\Hole Is Mine\Save\progress.dat",
                FileActivityKind::Modify,
                1_100,
            ),
        ];

        let candidates = analyze_save_activity(&events, &context());

        assert_eq!(candidates.len(), 2);
        assert!(candidates[0].directory.ends_with("Save"));
        assert!(candidates[1]
            .downgrade_reasons
            .iter()
            .any(|reason| reason.contains("日志")));
    }

    #[test]
    fn browser_runtime_data_is_visible_but_low_confidence() {
        let events = vec![
            event(
                r"C:\Users\Tester\AppData\Local\Browser\User Data\Default\WebStorage\QuotaManager",
                FileActivityKind::Modify,
                1_000,
            ),
            event(
                r"C:\Users\Tester\AppData\Local\Browser\User Data\Default\WebStorage\QuotaManager-journal",
                FileActivityKind::Modify,
                1_100,
            ),
        ];

        let candidates = analyze_save_activity(&events, &context());

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].confidence, SaveCandidateConfidence::Low);
        assert!(candidates[0]
            .downgrade_reasons
            .iter()
            .any(|reason| reason.contains("缓存")));
        assert!(candidates[0]
            .downgrade_reasons
            .iter()
            .any(|reason| reason.contains("日志")));
    }

    #[test]
    fn busy_game_analytics_directory_cannot_become_high_confidence() {
        let events = (0..6)
            .flat_map(|index| {
                let path = format!(
                    r"C:\Users\Tester\AppData\LocalLow\Incrementalist\Hole Is Mine\Unity\session\Analytics\ArchivedEvents\event-{index}"
                );
                [
                    event(&path, FileActivityKind::Create, 1_000 + index * 100),
                    event(&path, FileActivityKind::Modify, 1_050 + index * 100),
                ]
            })
            .collect::<Vec<_>>();

        let candidates = analyze_save_activity(&events, &context());

        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].score >= 30);
        assert_eq!(candidates[0].confidence, SaveCandidateConfidence::Low);
        assert!(candidates[0]
            .downgrade_reasons
            .iter()
            .any(|reason| reason.contains("缓存")));
    }

    #[test]
    fn unity_insights_and_shader_analytics_are_low_confidence() {
        let events = vec![
            event(
                r"C:\Users\Tester\AppData\LocalLow\Studio\Hole Is Mine\Unity\Insights\ArchivedEvents\Session\session_end_event",
                FileActivityKind::Observed,
                1_000,
            ),
            event(
                r"C:\Users\Tester\AppData\LocalLow\Studio\Hole Is Mine\Unity\ShaderVariantAnalytics\ShaderRuntimeInfoEvent.json",
                FileActivityKind::Observed,
                1_100,
            ),
        ];

        let candidates = analyze_save_activity(&events, &context());

        assert_eq!(candidates.len(), 2);
        assert!(candidates
            .iter()
            .all(|candidate| candidate.confidence == SaveCandidateConfidence::Low));
        assert!(candidates.iter().all(|candidate| candidate
            .downgrade_reasons
            .iter()
            .any(|reason| reason.contains("缓存"))));
    }

    #[test]
    fn unrelated_busy_background_directory_is_low_confidence() {
        let events = (0..5)
            .flat_map(|index| {
                let path = format!(
                    r"C:\Users\Tester\AppData\Local\Microsoft\Edge\User Data\Default\entry-{index}.dat"
                );
                [
                    event(&path, FileActivityKind::Create, 1_000 + index * 100),
                    event(&path, FileActivityKind::Modify, 1_050 + index * 100),
                ]
            })
            .collect::<Vec<_>>();

        let candidates = analyze_save_activity(&events, &context());

        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].score >= 30);
        assert_eq!(candidates[0].confidence, SaveCandidateConfidence::Low);
        assert!(candidates[0]
            .downgrade_reasons
            .iter()
            .any(|reason| reason.contains("缺少直接关联")));
    }

    #[test]
    fn existing_directory_notifications_expand_to_direct_files_without_selecting_parent() {
        let unique = format!(
            "savelink-save-activity-directory-event-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("test-data")
            .join(unique)
            .join("Hole Is Mine");
        let release = root.join("Save").join("Release");
        std::fs::create_dir_all(&release).unwrap();
        let progress = release.join("progress.hole");
        std::fs::write(&progress, b"progress").unwrap();
        std::fs::write(release.join("progress.hole.bac"), b"progress").unwrap();
        let context = SaveActivityAnalysisContext {
            game_name: "Hole Is Mine".into(),
            executable_stem: Some("HoleIsMine".into()),
            install_dir: None,
            watched_roots: vec![root.clone()],
            excluded_roots: Vec::new(),
        };
        let events = vec![FileActivityEvent {
            path: release.clone(),
            kind: FileActivityKind::Modify,
            observed_at_unix_ms: 1_000,
        }];

        let candidates = analyze_save_activity(&events, &context);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].directory, release);
        assert_eq!(candidates[0].confidence, SaveCandidateConfidence::High);
        assert_eq!(candidates[0].distinct_file_count, 2);
        assert!(candidates[0]
            .files
            .iter()
            .all(|file| file.kinds == vec![FileActivityKind::Observed]));
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn install_and_watched_roots_are_not_confirmable() {
        let events = vec![
            event(
                r"C:\Games\Hole Is Mine\progress.dat",
                FileActivityKind::Modify,
                1_000,
            ),
            event(
                r"C:\Users\Tester\AppData\LocalLow\loose.dat",
                FileActivityKind::Modify,
                1_100,
            ),
        ];

        let candidates = analyze_save_activity(&events, &context());

        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().all(|candidate| !candidate.confirmable));
        assert!(candidates
            .iter()
            .all(|candidate| candidate.confidence == SaveCandidateConfidence::Low));
    }

    #[test]
    fn burst_duplicates_are_merged_and_savelink_data_is_excluded() {
        let events = vec![
            event(
                r"C:\Users\Tester\AppData\Roaming\Hole Is Mine\Save\slot.dat",
                FileActivityKind::Modify,
                1_000,
            ),
            event(
                r"C:\Users\Tester\AppData\Roaming\Hole Is Mine\Save\slot.dat",
                FileActivityKind::Modify,
                1_100,
            ),
            event(
                r"C:\Users\Tester\AppData\Roaming\Hole Is Mine\Save\slot.dat",
                FileActivityKind::Modify,
                1_500,
            ),
            event(
                r"C:\SaveLink\repository\snapshot.dat",
                FileActivityKind::Create,
                1_600,
            ),
        ];

        let candidates = analyze_save_activity(&events, &context());

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].event_count, 2);
        assert_eq!(candidates[0].files[0].event_count, 2);
    }
}
