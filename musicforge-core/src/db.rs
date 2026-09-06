//! 状态层（D16：SQLite 单文件 `library.db`）。
//!
//! 定位（务必牢记，否则会被滥用）：
//!
//! - db 只是**可再生缓存 + 历史**：扫描索引、哈希缓存、任务历史、ack 留痕。
//! - **真相永远在文件系统与 manifest**：db 丢失/损坏不影响已产出的文件，
//!   也不影响可审计性（manifest 才是「发生了什么」的留痕）。
//! - 因此 db 的一切写入失败都必须**降级而非中止**转换流程。
//!
//! 位置铁律（X16）：**只放本地配置目录，严禁网络挂载**——SQLite 在
//! SMB/NFS 上锁语义不可靠，长期会损坏。参见 [`default_db_path`] 与
//! [`ensure_local_db_path`]。

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

use crate::error::NcmError;

/// 当前 schema 版本（`PRAGMA user_version`）。
pub const SCHEMA_VERSION: u32 = 1;

/// 默认文件名。
pub const DB_FILE_NAME: &str = "library.db";

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS files (
    path       TEXT PRIMARY KEY,
    size       INTEGER NOT NULL,
    mtime      INTEGER,
    format     TEXT,
    sha256     TEXT,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS tasks (
    id          TEXT PRIMARY KEY,
    command     TEXT NOT NULL,
    started_at  TEXT NOT NULL,
    finished_at TEXT,
    ok          INTEGER NOT NULL DEFAULT 0,
    failed      INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS ack (
    id  TEXT PRIMARY KEY,
    at  TEXT NOT NULL
);
"#;

/// 状态库句柄。
pub struct Db {
    conn: Connection,
}

/// 文件索引行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub path: String,
    pub size: i64,
    pub mtime: Option<i64>,
    pub format: Option<String>,
    pub sha256: Option<String>,
}

impl Db {
    /// 打开（不存在则创建）状态库并完成迁移。
    ///
    /// - `user_version == 0`：建表并置为 [`SCHEMA_VERSION`]
    /// - 等于当前版本：直接使用
    /// - **高于当前版本：拒绝打开并报错**（降级不猜、不静默重建）
    pub fn open(path: &Path) -> Result<Self, NcmError> {
        ensure_local_db_path(path)?;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path).map_err(|e| NcmError::Db(e.to_string()))?;
        Self::migrate(&conn)?;
        Ok(Self { conn })
    }

    /// 内存库（测试用）。
    pub fn open_in_memory() -> Result<Self, NcmError> {
        let conn = Connection::open_in_memory().map_err(|e| NcmError::Db(e.to_string()))?;
        Self::migrate(&conn)?;
        Ok(Self { conn })
    }

    fn migrate(conn: &Connection) -> Result<(), NcmError> {
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        if version > SCHEMA_VERSION {
            return Err(NcmError::Db(format!(
                "状态库版本 {version} 高于本程序支持的 {SCHEMA_VERSION}：拒绝打开以免破坏数据（请升级 MusicForge 或迁移后重试）"
            )));
        }
        conn.execute_batch(SCHEMA_SQL)
            .map_err(|e| NcmError::Db(e.to_string()))?;
        if version == 0 {
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(|e| NcmError::Db(e.to_string()))?;
        }
        Ok(())
    }

    /// 写入/更新一条文件索引（增量扫描的缓存依据）。
    pub fn upsert_file(
        &self,
        path: &str,
        size: i64,
        mtime: Option<i64>,
        format: Option<&str>,
        sha256: Option<&str>,
    ) -> Result<(), NcmError> {
        self.conn
            .execute(
                "INSERT INTO files (path, size, mtime, format, sha256, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
                 ON CONFLICT(path) DO UPDATE SET
                    size=excluded.size, mtime=excluded.mtime,
                    format=excluded.format, sha256=excluded.sha256,
                    updated_at=excluded.updated_at",
                params![path, size, mtime, format, sha256],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }

    /// 读取一条文件索引。
    pub fn get_file(&self, path: &str) -> Result<Option<FileRow>, NcmError> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, size, mtime, format, sha256 FROM files WHERE path = ?1")
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let mut rows = stmt
            .query_map([path], |r| {
                Ok(FileRow {
                    path: r.get(0)?,
                    size: r.get(1)?,
                    mtime: r.get(2)?,
                    format: r.get(3)?,
                    sha256: r.get(4)?,
                })
            })
            .map_err(|e| NcmError::Db(e.to_string()))?;
        match rows.next() {
            Some(row) => Ok(Some(row.map_err(|e| NcmError::Db(e.to_string()))?)),
            None => Ok(None),
        }
    }

    /// 登记一次任务开始。
    pub fn start_task(&self, id: &str, command: &str, started_at: &str) -> Result<(), NcmError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO tasks (id, command, started_at, ok, failed)
                 VALUES (?1, ?2, ?3, 0, 0)",
                params![id, command, started_at],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }

    /// 标记一次任务结束。
    pub fn finish_task(
        &self,
        id: &str,
        finished_at: &str,
        ok: i64,
        failed: i64,
    ) -> Result<(), NcmError> {
        self.conn
            .execute(
                "UPDATE tasks SET finished_at=?2, ok=?3, failed=?4 WHERE id=?1",
                params![id, finished_at, ok, failed],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }

    /// 记录一次确认（如高风险插件的 acknowledge）。
    pub fn set_ack(&self, id: &str) -> Result<(), NcmError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO ack (id, at) VALUES (?1, datetime('now'))",
                params![id],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }

    /// 是否已确认。
    pub fn has_ack(&self, id: &str) -> Result<bool, NcmError> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(1) FROM ack WHERE id = ?1", [id], |r| r.get(0))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(n > 0)
    }

    /// 统计：`(files, tasks)`；用于诊断与测试。
    pub fn stats(&self) -> Result<(i64, i64), NcmError> {
        let files: i64 = self
            .conn
            .query_row("SELECT COUNT(1) FROM files", [], |r| r.get(0))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let tasks: i64 = self
            .conn
            .query_row("SELECT COUNT(1) FROM tasks", [], |r| r.get(0))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok((files, tasks))
    }
}

/// 本地配置目录（Windows `%LOCALAPPDATA%\MusicForge`，
/// unix `$XDG_CONFIG_HOME/musicforge` 或 `~/.config/musicforge`）。
/// 状态库与 GUI 的 manifests 都放这里——**绝不放音乐目录或网络挂载**。
pub fn local_config_dir() -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(|v| PathBuf::from(v).join("MusicForge"))
            .unwrap_or_else(|| PathBuf::from(".").join("MusicForge"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(|v| PathBuf::from(v).join("musicforge"))
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/musicforge"))
            })
            .unwrap_or_else(|| PathBuf::from(".").join("musicforge"))
    }
}

/// 默认状态库位置：本地配置目录下的 [`DB_FILE_NAME`]。
pub fn default_db_path() -> PathBuf {
    local_config_dir().join(DB_FILE_NAME)
}

/// 位置守卫：拒绝把状态库放到网络位置（UNC / `\\server\share`）。
///
/// SQLite 在 SMB/NFS 上的锁语义不可靠，长期运行会导致数据库损坏；
/// 宁可显式报错，也不要让用户把库放上去后静默烂掉。
pub fn ensure_local_db_path(path: &Path) -> Result<(), NcmError> {
    let s = path.to_string_lossy();
    let unc = s.starts_with(r"\\") || s.starts_with("//");
    if unc {
        return Err(NcmError::Db(format!(
            "状态库不能放在网络位置（{s}）：SQLite 在网络挂载上锁不可靠，请改用本地配置目录"
        )));
    }
    Ok(())
}
