//! Steam 游戏与存档目录自动发现。
//!
//! 扫描范围严格限制为 Steam 注册的游戏库和 `appmanifest_*.acf`，不会遍历磁盘。
//! Ludusavi Manifest 以预编译只读 SQLite 提供，运行时只查询当前已安装游戏的规则。

use glob::{glob_with, MatchOptions};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::collections::{BTreeSet, HashSet};
use std::env;
use std::fmt;
use std::path::{Path, PathBuf};
use steamlocate::SteamDir;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamDiscoveredGame {
    pub name: String,
    pub steam_name: String,
    pub app_id: u32,
    pub steam_root: PathBuf,
    pub library_root: PathBuf,
    pub install_dir: PathBuf,
    /// 可交给 SaveLink 保护的目录。Manifest 命中单个文件时会归一到其父目录；
    /// `<storeUserId>` 作为末尾目录占位符时只接受目录匹配。
    pub save_paths: Vec<PathBuf>,
    /// 只有 `config` 标签的路径，默认不保护。
    pub config_paths: Vec<PathBuf>,
    pub current_system_unresolved_rules: usize,
    pub other_environment_rules: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamDiscoveryReport {
    pub steam_root: PathBuf,
    pub library_count: usize,
    pub registered_app_count: usize,
    pub manifest_match_count: usize,
    pub games: Vec<SteamDiscoveredGame>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SteamDiscoveryError {
    SteamNotFound,
    InvalidSteamRoot(PathBuf),
    ManifestDatabaseMissing(PathBuf),
    ManifestDatabaseInvalid(String),
    SteamRead(String),
    Pattern(String),
}

impl fmt::Display for SteamDiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SteamNotFound => write!(formatter, "未找到 Steam，请确认 Steam 已安装"),
            Self::InvalidSteamRoot(path) => {
                write!(formatter, "Steam 目录无效：{}", path.display())
            }
            Self::ManifestDatabaseMissing(path) => {
                write!(formatter, "存档规则库不存在：{}", path.display())
            }
            Self::ManifestDatabaseInvalid(message) => {
                write!(formatter, "存档规则库不可用：{message}")
            }
            Self::SteamRead(message) => write!(formatter, "读取 Steam 游戏库失败：{message}"),
            Self::Pattern(message) => write!(formatter, "解析存档路径规则失败：{message}"),
        }
    }
}

impl std::error::Error for SteamDiscoveryError {}

pub type SteamDiscoveryResult<T> = std::result::Result<T, SteamDiscoveryError>;

pub struct SteamDiscoveryService {
    manifest_database: PathBuf,
}

impl SteamDiscoveryService {
    pub fn new(manifest_database: impl Into<PathBuf>) -> Self {
        Self {
            manifest_database: manifest_database.into(),
        }
    }

    /// 自动定位 Steam；测试和诊断时可显式传入 Steam 根目录。
    pub fn scan(&self, steam_root: Option<&Path>) -> SteamDiscoveryResult<SteamDiscoveryReport> {
        let connection = self.open_manifest_database()?;
        let steam = match steam_root {
            Some(path) => SteamDir::from_dir(path)
                .map_err(|_| SteamDiscoveryError::InvalidSteamRoot(path.to_path_buf()))?,
            None => SteamDir::locate().map_err(|_| SteamDiscoveryError::SteamNotFound)?,
        };
        let steam_root = steam.path().to_path_buf();
        let mut report = SteamDiscoveryReport {
            steam_root: steam_root.clone(),
            library_count: 0,
            registered_app_count: 0,
            manifest_match_count: 0,
            games: Vec::new(),
        };

        let libraries = steam
            .libraries()
            .map_err(|error| SteamDiscoveryError::SteamRead(error.to_string()))?;
        for library_result in libraries {
            let library = library_result
                .map_err(|error| SteamDiscoveryError::SteamRead(error.to_string()))?;
            report.library_count += 1;
            let library_root = library.path().to_path_buf();

            for app_result in library.apps() {
                let app = app_result
                    .map_err(|error| SteamDiscoveryError::SteamRead(error.to_string()))?;
                report.registered_app_count += 1;
                let source_games = query_games_by_store_id(&connection, "steam", app.app_id)?;
                if source_games.is_empty() {
                    continue;
                }
                report.manifest_match_count += 1;
                let install_dir = library.resolve_app_dir(&app);

                for source_game in source_games {
                    let game = resolve_alias(&connection, source_game)?;
                    let rules = query_rules(&connection, game.id)?;
                    let context = ResolutionContext::for_steam(
                        &steam_root,
                        &install_dir,
                        &app.install_dir,
                        app.app_id,
                    );
                    let scan = scan_rules(&rules, &context)?;
                    report.games.push(SteamDiscoveredGame {
                        name: game.name,
                        steam_name: app.name.clone().unwrap_or_else(|| "未知游戏".into()),
                        app_id: app.app_id,
                        steam_root: steam_root.clone(),
                        library_root: library_root.clone(),
                        install_dir: install_dir.clone(),
                        save_paths: scan.recommended.into_iter().collect(),
                        config_paths: scan.config_only.into_iter().collect(),
                        current_system_unresolved_rules: scan.unresolved_count,
                        other_environment_rules: scan.other_environment_rule_count,
                    });
                }
            }
        }

        report.games.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then(left.app_id.cmp(&right.app_id))
        });
        report
            .games
            .dedup_by(|left, right| left.app_id == right.app_id && left.name == right.name);
        Ok(report)
    }

    fn open_manifest_database(&self) -> SteamDiscoveryResult<Connection> {
        if !self.manifest_database.is_file() {
            return Err(SteamDiscoveryError::ManifestDatabaseMissing(
                self.manifest_database.clone(),
            ));
        }
        let connection = Connection::open_with_flags(
            &self.manifest_database,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(database_error)?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(database_error)?;
        if version != 1 {
            return Err(SteamDiscoveryError::ManifestDatabaseInvalid(format!(
                "不支持的规则库版本：{version}"
            )));
        }
        Ok(connection)
    }
}

#[derive(Debug)]
struct DbGame {
    id: i64,
    name: String,
    alias: Option<String>,
}

#[derive(Debug)]
struct DbRule {
    path_template: String,
    tags: BTreeSet<String>,
    constraints: Vec<RuleConstraint>,
}

#[derive(Debug)]
struct RuleConstraint {
    os: Option<String>,
    store: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compatibility {
    Compatible,
    Incompatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleKind {
    ConfigOnly,
    Protected,
}

impl RuleKind {
    fn from_tags(tags: &BTreeSet<String>) -> Self {
        if tags.len() == 1 && tags.contains("config") {
            Self::ConfigOnly
        } else {
            Self::Protected
        }
    }
}

#[derive(Debug)]
struct ResolutionContext {
    os: String,
    store: String,
    root: String,
    base: String,
    game: String,
    store_game_id: String,
    home: Option<String>,
    app_data: Option<String>,
    local_app_data: Option<String>,
    user_name: Option<String>,
}

impl ResolutionContext {
    fn for_steam(steam_root: &Path, base: &Path, game: &str, app_id: u32) -> Self {
        Self {
            os: "windows".into(),
            store: "steam".into(),
            root: path_text(steam_root),
            base: path_text(base),
            game: game.into(),
            store_game_id: app_id.to_string(),
            home: env_path("USERPROFILE").or_else(|| env_path("HOME")),
            app_data: env_path("APPDATA"),
            local_app_data: env_path("LOCALAPPDATA"),
            user_name: env::var("USERNAME").ok(),
        }
    }
}

#[derive(Debug, Default)]
struct RuleScan {
    recommended: BTreeSet<PathBuf>,
    config_only: BTreeSet<PathBuf>,
    unresolved_count: usize,
    other_environment_rule_count: usize,
}

fn query_games_by_store_id(
    connection: &Connection,
    store: &str,
    store_game_id: u32,
) -> SteamDiscoveryResult<Vec<DbGame>> {
    let mut statement = connection
        .prepare(
            "SELECT g.id, g.name, g.alias
             FROM manifest_store_ids ids
             JOIN manifest_games g ON g.id = ids.game_id
             WHERE ids.store = ?1 AND ids.store_game_id = ?2
             ORDER BY ids.is_primary DESC, g.name COLLATE NOCASE",
        )
        .map_err(database_error)?;
    let games = statement
        .query_map(params![store, store_game_id.to_string()], |row| {
            Ok(DbGame {
                id: row.get(0)?,
                name: row.get(1)?,
                alias: row.get(2)?,
            })
        })
        .map_err(database_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(database_error)?;
    Ok(games)
}

fn query_game(connection: &Connection, name: &str) -> SteamDiscoveryResult<Option<DbGame>> {
    connection
        .query_row(
            "SELECT id, name, alias FROM manifest_games WHERE name = ?1 COLLATE NOCASE",
            params![name],
            |row| {
                Ok(DbGame {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    alias: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(database_error)
}

fn resolve_alias(connection: &Connection, mut game: DbGame) -> SteamDiscoveryResult<DbGame> {
    let mut visited = HashSet::new();
    while let Some(alias) = game.alias.clone() {
        if !visited.insert(game.name.clone()) {
            return Err(SteamDiscoveryError::ManifestDatabaseInvalid(format!(
                "检测到循环别名：{}",
                game.name
            )));
        }
        game = query_game(connection, &alias)?.ok_or_else(|| {
            SteamDiscoveryError::ManifestDatabaseInvalid(format!(
                "别名目标不存在：{} -> {alias}",
                game.name
            ))
        })?;
    }
    Ok(game)
}

fn query_rules(connection: &Connection, game_id: i64) -> SteamDiscoveryResult<Vec<DbRule>> {
    let mut statement = connection
        .prepare(
            "SELECT id, path_template, tags
             FROM manifest_file_rules
             WHERE game_id = ?1
             ORDER BY path_template",
        )
        .map_err(database_error)?;
    let rows = statement
        .query_map(params![game_id], |row| {
            let tag_text: String = row.get(2)?;
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                tag_text
                    .split(',')
                    .filter(|tag| !tag.is_empty())
                    .map(str::to_string)
                    .collect::<BTreeSet<_>>(),
            ))
        })
        .map_err(database_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(database_error)?;

    let mut rules = Vec::with_capacity(rows.len());
    for (rule_id, path_template, tags) in rows {
        let mut constraints = connection
            .prepare(
                "SELECT os, store
                 FROM manifest_file_constraints
                 WHERE file_rule_id = ?1
                 ORDER BY ordinal",
            )
            .map_err(database_error)?;
        let constraints = constraints
            .query_map(params![rule_id], |row| {
                Ok(RuleConstraint {
                    os: row.get(0)?,
                    store: row.get(1)?,
                })
            })
            .map_err(database_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(database_error)?;
        rules.push(DbRule {
            path_template,
            tags,
            constraints,
        });
    }
    Ok(rules)
}

fn scan_rules(rules: &[DbRule], context: &ResolutionContext) -> SteamDiscoveryResult<RuleScan> {
    let mut result = RuleScan::default();
    for rule in rules {
        if compatibility(&rule.constraints, &context.os, &context.store)
            == Compatibility::Incompatible
        {
            result.other_environment_rule_count += 1;
            continue;
        }
        let expanded = expand(&rule.path_template, context);
        if !expanded.unresolved.is_empty() {
            result.unresolved_count += 1;
            continue;
        }
        for matched in match_pattern(&expanded.pattern)? {
            // `<storeUserId>` 代表用户目录。通配符还会匹配同级文件，但把这类
            // 文件提升为父目录会制造父子存档来源，例如 Elden Ring 的
            // GraphicsConfig.xml 与真实数字用户目录会重叠。
            if matched.is_file() && terminal_store_user_id_placeholder(&rule.path_template) {
                continue;
            }
            let protection_path = if matched.is_file() {
                matched.parent().map(Path::to_path_buf)
            } else if matched.is_dir() {
                Some(matched)
            } else {
                None
            };
            let Some(protection_path) = protection_path else {
                continue;
            };
            match RuleKind::from_tags(&rule.tags) {
                RuleKind::ConfigOnly => {
                    result.config_only.insert(protection_path);
                }
                RuleKind::Protected => {
                    result.recommended.insert(protection_path);
                }
            }
        }
    }
    result.recommended = collapse_overlapping_paths(result.recommended);
    result.config_only = collapse_overlapping_paths(result.config_only);
    Ok(result)
}

fn terminal_store_user_id_placeholder(template: &str) -> bool {
    template
        .replace('\\', "/")
        .trim_end_matches('/')
        .ends_with("<storeUserId>")
}

/// 多条 Manifest 规则可能命中同一目录树。SaveLink 的存档来源必须互相独立，
/// 因此保留最外层目录即可覆盖其子目录，避免后续快照和恢复出现父子来源。
fn collapse_overlapping_paths(paths: BTreeSet<PathBuf>) -> BTreeSet<PathBuf> {
    let mut candidates = paths.into_iter().collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        normalized_path_components(left)
            .len()
            .cmp(&normalized_path_components(right).len())
            .then_with(|| normalized_path(left).cmp(&normalized_path(right)))
    });

    let mut kept: Vec<PathBuf> = Vec::new();
    for candidate in candidates {
        if kept.iter().any(|parent| {
            let parent = normalized_path_components(parent);
            let candidate = normalized_path_components(&candidate);
            candidate.len() >= parent.len() && candidate.starts_with(&parent)
        }) {
            continue;
        }
        kept.push(candidate);
    }
    kept.into_iter().collect()
}

fn normalized_path(path: &Path) -> String {
    normalized_path_components(path).join("/")
}

fn normalized_path_components(path: &Path) -> Vec<String> {
    let mut components = Vec::new();
    for component in path.to_string_lossy().replace('\\', "/").split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            value => components.push(value.to_ascii_lowercase()),
        }
    }
    components
}

fn compatibility(constraints: &[RuleConstraint], os: &str, store: &str) -> Compatibility {
    if constraints.is_empty() {
        return Compatibility::Compatible;
    }
    if constraints.iter().any(|constraint| {
        constraint
            .os
            .as_deref()
            .is_none_or(|candidate| normalize(candidate) == os)
            && constraint
                .store
                .as_deref()
                .is_none_or(|candidate| normalize(candidate) == store)
    }) {
        Compatibility::Compatible
    } else {
        Compatibility::Incompatible
    }
}

#[derive(Debug)]
struct ExpandedPattern {
    pattern: String,
    unresolved: Vec<String>,
}

fn expand(template: &str, context: &ResolutionContext) -> ExpandedPattern {
    let mut pattern = template.replace('\\', "/");
    replace(&mut pattern, "<root>", Some(&context.root));
    replace(&mut pattern, "<base>", Some(&context.base));
    replace(&mut pattern, "<home>", context.home.as_deref());
    replace(&mut pattern, "<winAppData>", context.app_data.as_deref());
    replace(
        &mut pattern,
        "<winLocalAppData>",
        context.local_app_data.as_deref(),
    );
    let local_low = context
        .home
        .as_deref()
        .map(|home| format!("{home}/AppData/LocalLow"));
    replace(&mut pattern, "<winLocalAppDataLow>", local_low.as_deref());
    let documents = context
        .home
        .as_deref()
        .map(|home| format!("{home}/Documents"));
    replace(&mut pattern, "<winDocuments>", documents.as_deref());
    replace(&mut pattern, "<winPublic>", env_path("PUBLIC").as_deref());
    replace(
        &mut pattern,
        "<winProgramData>",
        env_path("PROGRAMDATA").as_deref(),
    );
    replace(&mut pattern, "<winDir>", env_path("WINDIR").as_deref());
    replace(&mut pattern, "<osUserName>", context.user_name.as_deref());
    replace(&mut pattern, "<storeGameId>", Some(&context.store_game_id));
    pattern = pattern.replace("<storeUserId>", "*");
    pattern = pattern.replace("<game>", &context.game);
    ExpandedPattern {
        unresolved: find_placeholders(&pattern),
        pattern,
    }
}

fn replace(target: &mut String, placeholder: &str, value: Option<&str>) {
    if let Some(value) = value {
        *target = target.replace(placeholder, value);
    }
}

fn find_placeholders(value: &str) -> Vec<String> {
    let mut found = BTreeSet::new();
    let mut rest = value;
    while let Some(start) = rest.find('<') {
        let after = &rest[start..];
        let Some(end) = after.find('>') else {
            break;
        };
        found.insert(after[..=end].to_string());
        rest = &after[end + 1..];
    }
    found.into_iter().collect()
}

fn match_pattern(pattern: &str) -> SteamDiscoveryResult<Vec<PathBuf>> {
    let options = MatchOptions {
        case_sensitive: false,
        require_literal_separator: false,
        require_literal_leading_dot: false,
    };
    let entries = glob_with(pattern, options)
        .map_err(|error| SteamDiscoveryError::Pattern(error.to_string()))?;
    let mut matches = Vec::new();
    for entry in entries {
        match entry {
            Ok(path) if path.exists() => matches.push(path),
            Ok(_) => {}
            Err(error) => return Err(SteamDiscoveryError::Pattern(error.to_string())),
        }
    }
    matches.sort();
    matches.dedup();
    Ok(matches)
}

fn env_path(name: &str) -> Option<String> {
    env::var(name).ok().map(|value| value.replace('\\', "/"))
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn database_error(error: rusqlite::Error) -> SteamDiscoveryError {
    SteamDiscoveryError::ManifestDatabaseInvalid(error.to_string())
}
