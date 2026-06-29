//! SQLite 实现的 `Repository`。
//!
//! 与 `InMemoryRepo` 实现同一个 trait（同一个插槽），因此上层逻辑与全部 31 个测试
//! 无需改动即可在真数据库上运行——这是存储抽象的验证点。
//!
//! 一个 `savelink.db` 文件即整个元数据库，无服务器、零安装，贴合"单机自包含"。

use crate::error::{Result, SaveLinkError};
use crate::model::{Game, Reason, Snapshot, SnapshotStatus};
use crate::repo::Repository;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 把 rusqlite 错误归一为 SaveLinkError::Io。
fn map_err(e: rusqlite::Error) -> SaveLinkError {
    SaveLinkError::Io(e.to_string())
}

fn reason_to_str(r: Reason) -> &'static str {
    match r {
        Reason::Manual => "manual",
        Reason::BeforeRestore => "before_restore",
        Reason::Auto => "auto",
    }
}
fn reason_from_str(s: &str) -> Reason {
    match s {
        "before_restore" => Reason::BeforeRestore,
        "auto" => Reason::Auto,
        _ => Reason::Manual,
    }
}
fn status_to_str(s: SnapshotStatus) -> &'static str {
    match s {
        SnapshotStatus::Writing => "writing",
        SnapshotStatus::Complete => "complete",
        SnapshotStatus::Corrupt => "corrupt",
    }
}
fn status_from_str(s: &str) -> SnapshotStatus {
    match s {
        "writing" => SnapshotStatus::Writing,
        "corrupt" => SnapshotStatus::Corrupt,
        _ => SnapshotStatus::Complete,
    }
}

pub struct SqliteRepo {
    conn: Mutex<Connection>,
}

impl SqliteRepo {
    /// 打开（或创建）数据库文件并建表。
    pub fn open(db_path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(db_path.as_ref()).map_err(map_err)?;
        Self::init(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// 内存数据库（测试用，等价 InMemoryRepo 但走真 SQL 路径）。
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(map_err)?;
        Self::init(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn init(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS games (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                icon TEXT,
                repo_path TEXT NOT NULL,
                save_paths TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS snapshots (
                id TEXT PRIMARY KEY,
                game_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                note TEXT,
                reason TEXT NOT NULL,
                locked INTEGER NOT NULL DEFAULT 0,
                file_count INTEGER NOT NULL,
                total_size INTEGER NOT NULL,
                content_hash TEXT NOT NULL,
                storage_key TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'complete'
             );",
        )
        .map_err(map_err)
    }
}

/// save_paths 序列化：换行分隔（路径不含换行符）。
fn paths_to_str(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}
fn paths_from_str(s: &str) -> Vec<PathBuf> {
    if s.is_empty() {
        return Vec::new();
    }
    s.split('\n').map(PathBuf::from).collect()
}

/// 从一行查询结果构造 Snapshot。列顺序见各查询的 SELECT。
fn row_to_snapshot(row: &rusqlite::Row<'_>) -> rusqlite::Result<Snapshot> {
    let reason: String = row.get(4)?;
    let locked_i: i64 = row.get(5)?;
    let status: String = row.get(10)?;
    Ok(Snapshot {
        id: row.get(0)?,
        game_id: row.get(1)?,
        created_at: row.get(2)?,
        note: row.get(3)?,
        reason: reason_from_str(&reason),
        locked: locked_i != 0,
        file_count: row.get::<_, i64>(6)? as u64,
        total_size: row.get::<_, i64>(7)? as u64,
        content_hash: row.get(8)?,
        storage_key: row.get(9)?,
        status: status_from_str(&status),
    })
}

const SNAP_COLS: &str =
    "id, game_id, created_at, note, reason, locked, file_count, total_size, content_hash, storage_key, status";

impl Repository for SqliteRepo {
    fn insert_game(&self, game: Game) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO games (id, name, icon, repo_path, save_paths, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                game.id,
                game.name,
                game.icon,
                game.repo_path.to_string_lossy(),
                paths_to_str(&game.save_paths),
                game.created_at,
                game.updated_at,
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    fn get_game(&self, game_id: &str) -> Result<Option<Game>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, name, icon, repo_path, save_paths, created_at, updated_at FROM games WHERE id = ?1")
            .map_err(map_err)?;
        let mut rows = stmt
            .query_map(params![game_id], |row| {
                let repo_path: String = row.get(3)?;
                let save_paths: String = row.get(4)?;
                Ok(Game {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    icon: row.get(2)?,
                    repo_path: PathBuf::from(repo_path),
                    save_paths: paths_from_str(&save_paths),
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .map_err(map_err)?;
        match rows.next() {
            Some(r) => Ok(Some(r.map_err(map_err)?)),
            None => Ok(None),
        }
    }

    fn list_games(&self) -> Result<Vec<Game>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, name, icon, repo_path, save_paths, created_at, updated_at FROM games")
            .map_err(map_err)?;
        let rows = stmt
            .query_map([], |row| {
                let repo_path: String = row.get(3)?;
                let save_paths: String = row.get(4)?;
                Ok(Game {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    icon: row.get(2)?,
                    repo_path: PathBuf::from(repo_path),
                    save_paths: paths_from_str(&save_paths),
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .map_err(map_err)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(map_err)?);
        }
        Ok(out)
    }

    fn update_game(&self, game: Game) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE games SET name=?2, icon=?3, repo_path=?4, save_paths=?5, created_at=?6, updated_at=?7
             WHERE id=?1",
            params![
                game.id,
                game.name,
                game.icon,
                game.repo_path.to_string_lossy(),
                paths_to_str(&game.save_paths),
                game.created_at,
                game.updated_at,
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    fn insert_snapshot(&self, s: Snapshot) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO snapshots (id, game_id, created_at, note, reason, locked, file_count, total_size, content_hash, storage_key, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                s.id,
                s.game_id,
                s.created_at,
                s.note,
                reason_to_str(s.reason),
                s.locked as i64,
                s.file_count as i64,
                s.total_size as i64,
                s.content_hash,
                s.storage_key,
                status_to_str(s.status),
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    fn get_snapshot(&self, snapshot_id: &str) -> Result<Option<Snapshot>> {
        let conn = self.conn.lock().unwrap();
        let sql = format!("SELECT {SNAP_COLS} FROM snapshots WHERE id = ?1");
        let mut stmt = conn.prepare(&sql).map_err(map_err)?;
        let mut rows = stmt.query_map(params![snapshot_id], row_to_snapshot).map_err(map_err)?;
        match rows.next() {
            Some(r) => Ok(Some(r.map_err(map_err)?)),
            None => Ok(None),
        }
    }

    fn list_snapshots(&self, game_id: &str) -> Result<Vec<Snapshot>> {
        let conn = self.conn.lock().unwrap();
        // 时间线倒序（新在前）。created_at 字符串可比，同值时用 rowid 兜底稳定排序。
        let sql = format!(
            "SELECT {SNAP_COLS} FROM snapshots WHERE game_id = ?1 ORDER BY created_at DESC, rowid DESC"
        );
        let mut stmt = conn.prepare(&sql).map_err(map_err)?;
        let rows = stmt.query_map(params![game_id], row_to_snapshot).map_err(map_err)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(map_err)?);
        }
        Ok(out)
    }

    fn update_snapshot(&self, s: Snapshot) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE snapshots SET game_id=?2, created_at=?3, note=?4, reason=?5, locked=?6,
                 file_count=?7, total_size=?8, content_hash=?9, storage_key=?10, status=?11
             WHERE id=?1",
            params![
                s.id,
                s.game_id,
                s.created_at,
                s.note,
                reason_to_str(s.reason),
                s.locked as i64,
                s.file_count as i64,
                s.total_size as i64,
                s.content_hash,
                s.storage_key,
                status_to_str(s.status),
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    fn delete_snapshot(&self, snapshot_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM snapshots WHERE id = ?1", params![snapshot_id])
            .map_err(map_err)?;
        Ok(())
    }

    fn list_writing(&self) -> Result<Vec<Snapshot>> {
        let conn = self.conn.lock().unwrap();
        let sql = format!("SELECT {SNAP_COLS} FROM snapshots WHERE status = 'writing'");
        let mut stmt = conn.prepare(&sql).map_err(map_err)?;
        let rows = stmt.query_map([], row_to_snapshot).map_err(map_err)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(map_err)?);
        }
        Ok(out)
    }
}
