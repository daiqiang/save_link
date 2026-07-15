//! SQLite 实现的 `Repository`。
//!
//! 与 `InMemoryRepo` 实现同一个 trait（同一个插槽），因此上层逻辑与全部 31 个测试
//! 无需改动即可在真数据库上运行——这是存储抽象的验证点。
//!
//! 一个 `savelink.db` 文件即整个元数据库，无服务器、零安装，贴合"单机自包含"。

use crate::cloud_model::{CloudAccount, CloudGameBinding, CloudSnapshotRecord, CloudSyncStatus};
use crate::cloud_repo::CloudStateRepository;
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
             );
             CREATE TABLE IF NOT EXISTS app_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS cloud_accounts (
                id TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                account_identity TEXT,
                display_name TEXT,
                token_ref TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                UNIQUE(provider, account_identity)
             );
             CREATE TABLE IF NOT EXISTS cloud_game_bindings (
                account_id TEXT NOT NULL,
                cloud_game_id TEXT NOT NULL,
                local_game_id TEXT NOT NULL,
                remote_revision INTEGER NOT NULL DEFAULT 0,
                sync_enabled INTEGER NOT NULL DEFAULT 1,
                last_scanned_at TEXT,
                PRIMARY KEY (account_id, cloud_game_id),
                UNIQUE(account_id, local_game_id)
             );
             CREATE TABLE IF NOT EXISTS cloud_snapshot_sync (
                account_id TEXT NOT NULL,
                cloud_game_id TEXT NOT NULL,
                snapshot_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                reason TEXT NOT NULL,
                note TEXT,
                locked INTEGER NOT NULL DEFAULT 0,
                file_count INTEGER NOT NULL,
                total_size INTEGER NOT NULL,
                content_hash TEXT NOT NULL,
                archive_size INTEGER NOT NULL,
                archive_sha256 TEXT NOT NULL,
                published_at TEXT NOT NULL,
                created_by_device_id TEXT NOT NULL,
                sync_status TEXT NOT NULL CHECK (
                    sync_status IN (
                        'uploading', 'uploaded', 'remote_only', 'downloading',
                        'downloaded', 'ignored', 'error'
                    )
                ),
                last_synced_at TEXT,
                last_error_code TEXT,
                PRIMARY KEY (account_id, snapshot_id)
             );
             CREATE INDEX IF NOT EXISTS idx_cloud_snapshot_game
                ON cloud_snapshot_sync(account_id, cloud_game_id, created_at DESC);",
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

const CLOUD_SNAPSHOT_COLS: &str = "account_id, cloud_game_id, snapshot_id, created_at, reason, note, locked, file_count, total_size, content_hash, archive_size, archive_sha256, published_at, created_by_device_id, sync_status, last_synced_at, last_error_code";

fn row_to_cloud_snapshot(row: &rusqlite::Row<'_>) -> rusqlite::Result<CloudSnapshotRecord> {
    let reason: String = row.get(4)?;
    let locked: i64 = row.get(6)?;
    let status: String = row.get(14)?;
    Ok(CloudSnapshotRecord {
        account_id: row.get(0)?,
        cloud_game_id: row.get(1)?,
        snapshot_id: row.get(2)?,
        created_at: row.get(3)?,
        reason: reason_from_str(&reason),
        note: row.get(5)?,
        locked: locked != 0,
        file_count: row.get::<_, i64>(7)? as u64,
        total_size: row.get::<_, i64>(8)? as u64,
        content_hash: row.get(9)?,
        archive_size: row.get::<_, i64>(10)? as u64,
        archive_sha256: row.get(11)?,
        published_at: row.get(12)?,
        created_by_device_id: row.get(13)?,
        sync_status: CloudSyncStatus::from_str(&status),
        last_synced_at: row.get(15)?,
        last_error_code: row.get(16)?,
    })
}

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

    fn delete_game(&self, game_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction().map_err(map_err)?;
        tx.execute("DELETE FROM snapshots WHERE game_id = ?1", params![game_id])
            .map_err(map_err)?;
        tx.execute("DELETE FROM games WHERE id = ?1", params![game_id])
            .map_err(map_err)?;
        tx.commit().map_err(map_err)?;
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

impl CloudStateRepository for SqliteRepo {
    fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )
        .map_err(map_err)?;
        Ok(())
    }

    fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT value FROM app_settings WHERE key = ?1")
            .map_err(map_err)?;
        let mut rows = stmt
            .query_map(params![key], |row| row.get(0))
            .map_err(map_err)?;
        match rows.next() {
            Some(value) => Ok(Some(value.map_err(map_err)?)),
            None => Ok(None),
        }
    }

    fn upsert_cloud_account(&self, account: CloudAccount) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO cloud_accounts
                (id, provider, account_identity, display_name, token_ref, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                provider=excluded.provider,
                account_identity=excluded.account_identity,
                display_name=excluded.display_name,
                token_ref=excluded.token_ref,
                updated_at=excluded.updated_at",
            params![
                account.id,
                account.provider,
                account.account_identity,
                account.display_name,
                account.token_ref,
                account.created_at,
                account.updated_at,
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    fn get_cloud_account(&self, account_id: &str) -> Result<Option<CloudAccount>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, provider, account_identity, display_name, token_ref, created_at, updated_at
                 FROM cloud_accounts WHERE id = ?1",
            )
            .map_err(map_err)?;
        let mut rows = stmt
            .query_map(params![account_id], |row| {
                Ok(CloudAccount {
                    id: row.get(0)?,
                    provider: row.get(1)?,
                    account_identity: row.get(2)?,
                    display_name: row.get(3)?,
                    token_ref: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .map_err(map_err)?;
        match rows.next() {
            Some(account) => Ok(Some(account.map_err(map_err)?)),
            None => Ok(None),
        }
    }

    fn list_cloud_accounts(&self) -> Result<Vec<CloudAccount>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, provider, account_identity, display_name, token_ref, created_at, updated_at
                 FROM cloud_accounts ORDER BY created_at, id",
            )
            .map_err(map_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(CloudAccount {
                    id: row.get(0)?,
                    provider: row.get(1)?,
                    account_identity: row.get(2)?,
                    display_name: row.get(3)?,
                    token_ref: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    fn upsert_cloud_game_binding(&self, binding: CloudGameBinding) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO cloud_game_bindings
                (account_id, cloud_game_id, local_game_id, remote_revision, sync_enabled, last_scanned_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(account_id, cloud_game_id) DO UPDATE SET
                local_game_id=excluded.local_game_id,
                remote_revision=excluded.remote_revision,
                sync_enabled=excluded.sync_enabled,
                last_scanned_at=excluded.last_scanned_at",
            params![
                binding.account_id,
                binding.cloud_game_id,
                binding.local_game_id,
                binding.remote_revision as i64,
                binding.sync_enabled as i64,
                binding.last_scanned_at,
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    fn get_cloud_game_binding(
        &self,
        account_id: &str,
        cloud_game_id: &str,
    ) -> Result<Option<CloudGameBinding>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT account_id, cloud_game_id, local_game_id, remote_revision, sync_enabled, last_scanned_at
                 FROM cloud_game_bindings WHERE account_id = ?1 AND cloud_game_id = ?2",
            )
            .map_err(map_err)?;
        let mut rows = stmt
            .query_map(params![account_id, cloud_game_id], |row| {
                Ok(CloudGameBinding {
                    account_id: row.get(0)?,
                    cloud_game_id: row.get(1)?,
                    local_game_id: row.get(2)?,
                    remote_revision: row.get::<_, i64>(3)? as u64,
                    sync_enabled: row.get::<_, i64>(4)? != 0,
                    last_scanned_at: row.get(5)?,
                })
            })
            .map_err(map_err)?;
        match rows.next() {
            Some(binding) => Ok(Some(binding.map_err(map_err)?)),
            None => Ok(None),
        }
    }

    fn list_cloud_game_bindings(&self, account_id: &str) -> Result<Vec<CloudGameBinding>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT account_id, cloud_game_id, local_game_id, remote_revision, sync_enabled, last_scanned_at
                 FROM cloud_game_bindings WHERE account_id = ?1 ORDER BY cloud_game_id",
            )
            .map_err(map_err)?;
        let rows = stmt
            .query_map(params![account_id], |row| {
                Ok(CloudGameBinding {
                    account_id: row.get(0)?,
                    cloud_game_id: row.get(1)?,
                    local_game_id: row.get(2)?,
                    remote_revision: row.get::<_, i64>(3)? as u64,
                    sync_enabled: row.get::<_, i64>(4)? != 0,
                    last_scanned_at: row.get(5)?,
                })
            })
            .map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    fn upsert_cloud_snapshot(&self, snapshot: CloudSnapshotRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO cloud_snapshot_sync
                (account_id, cloud_game_id, snapshot_id, created_at, reason, note, locked,
                 file_count, total_size, content_hash, archive_size, archive_sha256,
                 published_at, created_by_device_id, sync_status, last_synced_at, last_error_code)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
             ON CONFLICT(account_id, snapshot_id) DO UPDATE SET
                cloud_game_id=excluded.cloud_game_id,
                created_at=excluded.created_at,
                reason=excluded.reason,
                note=excluded.note,
                locked=excluded.locked,
                file_count=excluded.file_count,
                total_size=excluded.total_size,
                content_hash=excluded.content_hash,
                archive_size=excluded.archive_size,
                archive_sha256=excluded.archive_sha256,
                published_at=excluded.published_at,
                created_by_device_id=excluded.created_by_device_id,
                sync_status=excluded.sync_status,
                last_synced_at=excluded.last_synced_at,
                last_error_code=excluded.last_error_code",
            params![
                snapshot.account_id,
                snapshot.cloud_game_id,
                snapshot.snapshot_id,
                snapshot.created_at,
                reason_to_str(snapshot.reason),
                snapshot.note,
                snapshot.locked as i64,
                snapshot.file_count as i64,
                snapshot.total_size as i64,
                snapshot.content_hash,
                snapshot.archive_size as i64,
                snapshot.archive_sha256,
                snapshot.published_at,
                snapshot.created_by_device_id,
                snapshot.sync_status.as_str(),
                snapshot.last_synced_at,
                snapshot.last_error_code,
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    fn get_cloud_snapshot(
        &self,
        account_id: &str,
        snapshot_id: &str,
    ) -> Result<Option<CloudSnapshotRecord>> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {CLOUD_SNAPSHOT_COLS} FROM cloud_snapshot_sync
             WHERE account_id = ?1 AND snapshot_id = ?2"
        );
        let mut stmt = conn.prepare(&sql).map_err(map_err)?;
        let mut rows = stmt
            .query_map(params![account_id, snapshot_id], row_to_cloud_snapshot)
            .map_err(map_err)?;
        match rows.next() {
            Some(snapshot) => Ok(Some(snapshot.map_err(map_err)?)),
            None => Ok(None),
        }
    }

    fn list_cloud_snapshots(
        &self,
        account_id: &str,
        cloud_game_id: &str,
    ) -> Result<Vec<CloudSnapshotRecord>> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {CLOUD_SNAPSHOT_COLS} FROM cloud_snapshot_sync
             WHERE account_id = ?1 AND cloud_game_id = ?2
             ORDER BY created_at DESC, snapshot_id DESC"
        );
        let mut stmt = conn.prepare(&sql).map_err(map_err)?;
        let rows = stmt
            .query_map(params![account_id, cloud_game_id], row_to_cloud_snapshot)
            .map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    fn list_cloud_snapshots_by_status(
        &self,
        account_id: &str,
        status: CloudSyncStatus,
    ) -> Result<Vec<CloudSnapshotRecord>> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {CLOUD_SNAPSHOT_COLS} FROM cloud_snapshot_sync
             WHERE account_id = ?1 AND sync_status = ?2
             ORDER BY created_at DESC, snapshot_id DESC"
        );
        let mut stmt = conn.prepare(&sql).map_err(map_err)?;
        let rows = stmt
            .query_map(params![account_id, status.as_str()], row_to_cloud_snapshot)
            .map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    fn update_cloud_snapshot_status(
        &self,
        account_id: &str,
        snapshot_id: &str,
        status: CloudSyncStatus,
        last_synced_at: Option<&str>,
        last_error_code: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let changed = conn
            .execute(
                "UPDATE cloud_snapshot_sync
                 SET sync_status = ?3, last_synced_at = ?4, last_error_code = ?5
                 WHERE account_id = ?1 AND snapshot_id = ?2",
                params![
                    account_id,
                    snapshot_id,
                    status.as_str(),
                    last_synced_at,
                    last_error_code,
                ],
            )
            .map_err(map_err)?;
        if changed == 0 {
            return Err(SaveLinkError::Io(format!(
                "云端快照状态不存在: {account_id}/{snapshot_id}"
            )));
        }
        Ok(())
    }
}
