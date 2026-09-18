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

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::NcmError;

/// 当前 schema 版本（`PRAGMA user_version`）。
///
/// - v1（历史）：files / tasks / ack —— 转换状态与哈希缓存层。
/// - v2（P1 曲库）：sources / artists / albums / tracks —— 曲库浏览维度层。
///   与 v1 同库共存：v1 表管"转换/缓存"，v2 表管"曲库视图"，
///   两者都遵循同一铁律——**db 是可再生的，真相在文件系统**。
pub const SCHEMA_VERSION: u32 = 2;

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

/// v2（P1 曲库维度层）建表 SQL。
///
/// 设计要点：
/// - `sources` 是媒体源（用户授权的曲库目录）；`tracks.source_id` 关联之。
/// - `albums` 用**表达式唯一索引** `(title, IFNULL(album_artist_id, 0))`——
///   SQLite 的普通 UNIQUE 对 NULL 不去重（NULL != NULL），
///   未知艺术家（NULL）的同名专辑会无限重复插入。
/// - `indexed_at` 记录该行最后一次索引的时间戳（秒）：
///   一次索引 run 结束后用 `indexed_at < run_id` 找出并删除**已被移除**的文件行——
///   增量清理不依赖全量 path 比对（十万级 path 列表传进 SQL 会撞变量上限）。
/// - 外键用 `REFERENCES` 声明（文档价值）；级联删除不依赖 FK pragma
///   （`remove_source` 手动两条 DELETE 兜底，见该方法注释）。
const V2_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS sources (
    id        INTEGER PRIMARY KEY,
    path      TEXT NOT NULL UNIQUE,
    label     TEXT,
    enabled   INTEGER NOT NULL DEFAULT 1,
    added_at  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS artists (
    id    INTEGER PRIMARY KEY,
    name  TEXT NOT NULL UNIQUE
);
CREATE TABLE IF NOT EXISTS albums (
    id              INTEGER PRIMARY KEY,
    title           TEXT NOT NULL,
    album_artist_id INTEGER REFERENCES artists(id),
    year            INTEGER,
    cover_cache     TEXT,
    cover_hash      TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_albums_key
    ON albums(title, IFNULL(album_artist_id, 0));
CREATE TABLE IF NOT EXISTS tracks (
    id            INTEGER PRIMARY KEY,
    source_id     INTEGER NOT NULL REFERENCES sources(id),
    path          TEXT NOT NULL UNIQUE,
    size          INTEGER NOT NULL,
    mtime         INTEGER,
    title         TEXT,
    artist_id     INTEGER REFERENCES artists(id),
    album_id      INTEGER REFERENCES albums(id),
    track_no      INTEGER,
    disc_no       INTEGER,
    duration_ms   INTEGER,
    format        TEXT,
    sample_rate   INTEGER,
    bit_depth     INTEGER,
    channels      INTEGER,
    is_lossless   INTEGER NOT NULL DEFAULT 0,
    indexed_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tracks_source ON tracks(source_id);
CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(artist_id);
CREATE INDEX IF NOT EXISTS idx_tracks_album  ON tracks(album_id);
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

// ---------------------------------------------------------------- v2 类型 --

/// 媒体源行（v2）：用户授权进曲库的根目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRow {
    pub id: i64,
    pub path: String,
    pub label: Option<String>,
    pub enabled: bool,
    pub added_at: i64,
}

/// 曲目入库输入（v2）：索引构建层产出的扁平结构。
///
/// `artist` / `album` / `album_artist` 是**名字**而非 id——写入时由
/// [`Db::upsert_tracks_batch`] 统一解析（对不存在的艺术家/专辑先建后引），
/// 索引层不必感知 id 分配。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrackInput {
    pub source_id: i64,
    pub path: String,
    pub size: i64,
    pub mtime: Option<i64>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub year: Option<i64>,
    pub track_no: Option<i64>,
    pub disc_no: Option<i64>,
    pub duration_ms: Option<i64>,
    pub format: Option<String>,
    pub sample_rate: Option<i64>,
    pub bit_depth: Option<i64>,
    pub channels: Option<i64>,
    pub is_lossless: bool,
}

/// 曲目行（v2 读路径）：`artist` / `album` 已解析为显示名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackRow {
    pub id: i64,
    pub source_id: i64,
    pub path: String,
    pub size: i64,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_no: Option<i64>,
    pub duration_ms: Option<i64>,
    pub format: Option<String>,
    pub sample_rate: Option<i64>,
    pub bit_depth: Option<i64>,
    pub channels: Option<i64>,
    pub is_lossless: bool,
}

/// 艺术家聚合行（列表视图）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistRow {
    pub id: i64,
    pub name: String,
    pub track_count: i64,
}

/// 专辑聚合行（列表视图）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumRow {
    pub id: i64,
    pub title: String,
    pub artist: Option<String>,
    pub year: Option<i64>,
    pub track_count: i64,
}

/// 曲库总览统计。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LibraryStats {
    pub tracks: i64,
    pub artists: i64,
    pub albums: i64,
    /// 所有曲目字节数之和
    pub total_size: i64,
    /// 所有曲目时长之和（毫秒；未知时长按 0 计）
    pub total_duration_ms: i64,
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
        // P2-1：WAL 提升读写并发（扫描写 + 查询读不再互相阻塞）。
        // 铁律不变——db 只是**可再生缓存**：WAL 设置失败仅降级，绝不中止流程。
        // （二级索引暂不加：当前查询模式全部按 path 命中 PRIMARY KEY，加索引无收益。）
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
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
        // 逐级迁移：`version < N` 时执行第 N 级建表 SQL。全部语句幂等
        // （CREATE IF NOT EXISTS / 表达式索引），中途失败可安全重试。
        // 历史库（v1）只补 v2 表；全新库（v0）两级都执行。
        if version < 1 {
            conn.execute_batch(SCHEMA_SQL)
                .map_err(|e| NcmError::Db(e.to_string()))?;
        }
        if version < 2 {
            conn.execute_batch(V2_SCHEMA_SQL)
                .map_err(|e| NcmError::Db(e.to_string()))?;
        }
        if version != SCHEMA_VERSION {
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

    /// D17 哈希缓存查询（L1+L2 语义）：
    ///
    /// 仅当缓存行的 `size` 与 `mtime` 都与当前一致时才返回哈希——
    /// 任一不一致（或行内无哈希）都返回 `None`，调用方需重算并回写。
    /// `mtime` 不可得的文件永远 miss（宁可重算，不返回过期哈希）。
    pub fn cached_hash(
        &self,
        path: &str,
        size: i64,
        mtime: i64,
    ) -> Result<Option<String>, NcmError> {
        let h: Option<String> = self
            .conn
            .query_row(
                "SELECT sha256 FROM files
                 WHERE path = ?1 AND size = ?2 AND mtime = ?3 AND sha256 IS NOT NULL",
                params![path, size, mtime],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(h)
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

    // ------------------------------------------------------------ v2 曲库 --

    /// 登记/更新媒体源，返回其 id。
    ///
    /// 幂等：同一路径重复登记不报错；`label` 为 `None` 时保留已有标签
    /// （重复扫描场景下不会把用户起的名字清掉）。
    pub fn upsert_source(&self, path: &str, label: Option<&str>) -> Result<i64, NcmError> {
        self.conn
            .execute(
                "INSERT INTO sources (path, label, enabled, added_at)
                 VALUES (?1, ?2, 1, unixepoch())
                 ON CONFLICT(path) DO UPDATE SET
                    label = COALESCE(excluded.label, sources.label)",
                params![path, label],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        self.conn
            .query_row("SELECT id FROM sources WHERE path = ?1", [path], |r| {
                r.get(0)
            })
            .map_err(|e| NcmError::Db(e.to_string()))
    }

    /// 列出全部媒体源（按添加时间序）。
    pub fn list_sources(&self) -> Result<Vec<SourceRow>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, path, label, enabled, added_at FROM sources ORDER BY added_at, id",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SourceRow {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    label: r.get(2)?,
                    enabled: r.get::<_, i64>(3)? != 0,
                    added_at: r.get(4)?,
                })
            })
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }

    /// 移除媒体源并连带清理其曲目行；返回被清理的曲目数。
    ///
    /// 手动两条 DELETE（tracks → source）而非依赖 FK 级联：SQLite 级联
    /// 需要 per-connection `PRAGMA foreign_keys = ON`，本库刻意不开该
    /// pragma（不给既有连接引入隐性行为），`REFERENCES` 声明只作文档。
    pub fn remove_source(&self, id: i64) -> Result<usize, NcmError> {
        let n = self
            .conn
            .execute("DELETE FROM tracks WHERE source_id = ?1", [id])
            .map_err(|e| NcmError::Db(e.to_string()))?;
        self.conn
            .execute("DELETE FROM sources WHERE id = ?1", [id])
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(n)
    }

    /// 批量写入曲目（单事务）。
    ///
    /// - 对不存在的 artist/album 先建后引（`INSERT OR IGNORE` + 回读 id），
    ///   索引层只提供名字，不感知 id 分配；
    /// - 专辑年份为空时用本次输入补齐（`year IS NULL` 守卫，不覆盖已有值）；
    /// - 已存在的 path 全量 UPDATE（重扫时元数据变化即时反映）；
    /// - `indexed_at` 标记本次索引 run——run 结束后用
    ///   [`Db::remove_stale_tracks`] 清掉不在本次索引里的行。
    pub fn upsert_tracks_batch(
        &self,
        tracks: &[TrackInput],
        indexed_at: i64,
    ) -> Result<usize, NcmError> {
        if tracks.is_empty() {
            return Ok(0);
        }
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        {
            let mut st_artist = tx
                .prepare("INSERT OR IGNORE INTO artists (name) VALUES (?1)")
                .map_err(|e| NcmError::Db(e.to_string()))?;
            let mut st_artist_id = tx
                .prepare("SELECT id FROM artists WHERE name = ?1")
                .map_err(|e| NcmError::Db(e.to_string()))?;
            let mut st_album = tx
                .prepare(
                    "INSERT OR IGNORE INTO albums (title, album_artist_id, year)
                     VALUES (?1, ?2, ?3)",
                )
                .map_err(|e| NcmError::Db(e.to_string()))?;
            let mut st_album_id = tx
                .prepare(
                    "SELECT id FROM albums
                     WHERE title = ?1 AND IFNULL(album_artist_id, 0) = IFNULL(?2, 0)",
                )
                .map_err(|e| NcmError::Db(e.to_string()))?;
            let mut st_album_year = tx
                .prepare("UPDATE albums SET year = ?1 WHERE id = ?2 AND year IS NULL")
                .map_err(|e| NcmError::Db(e.to_string()))?;
            let mut st_track = tx
                .prepare(
                    "INSERT INTO tracks (source_id, path, size, mtime, title, artist_id,
                                         album_id, track_no, disc_no, duration_ms, format,
                                         sample_rate, bit_depth, channels, is_lossless, indexed_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                     ON CONFLICT(path) DO UPDATE SET
                        source_id=excluded.source_id, size=excluded.size, mtime=excluded.mtime,
                        title=excluded.title, artist_id=excluded.artist_id,
                        album_id=excluded.album_id, track_no=excluded.track_no,
                        disc_no=excluded.disc_no, duration_ms=excluded.duration_ms,
                        format=excluded.format, sample_rate=excluded.sample_rate,
                        bit_depth=excluded.bit_depth, channels=excluded.channels,
                        is_lossless=excluded.is_lossless, indexed_at=excluded.indexed_at",
                )
                .map_err(|e| NcmError::Db(e.to_string()))?;

            for t in tracks {
                let artist_id = match non_empty(t.artist.as_deref()) {
                    Some(n) => {
                        st_artist
                            .execute(params![n])
                            .map_err(|e| NcmError::Db(e.to_string()))?;
                        let id: i64 = st_artist_id
                            .query_row(params![n], |r| r.get(0))
                            .map_err(|e| NcmError::Db(e.to_string()))?;
                        Some(id)
                    }
                    None => None,
                };
                // album_artist 缺省时回退到 track artist（专辑归属的最合理猜测）
                let album_artist_id = match non_empty(t.album_artist.as_deref()) {
                    Some(n) => {
                        st_artist
                            .execute(params![n])
                            .map_err(|e| NcmError::Db(e.to_string()))?;
                        let id: i64 = st_artist_id
                            .query_row(params![n], |r| r.get(0))
                            .map_err(|e| NcmError::Db(e.to_string()))?;
                        Some(id)
                    }
                    None => artist_id,
                };
                let album_id = match non_empty(t.album.as_deref()) {
                    Some(n) => {
                        st_album
                            .execute(params![n, album_artist_id, t.year])
                            .map_err(|e| NcmError::Db(e.to_string()))?;
                        let id: i64 = st_album_id
                            .query_row(params![n, album_artist_id], |r| r.get(0))
                            .map_err(|e| NcmError::Db(e.to_string()))?;
                        if let Some(y) = t.year {
                            // 补空不覆盖；失败忽略（缓存层降级哲学）
                            let _ = st_album_year.execute(params![y, id]);
                        }
                        Some(id)
                    }
                    None => None,
                };
                st_track
                    .execute(params![
                        t.source_id,
                        &t.path,
                        t.size,
                        t.mtime,
                        &t.title,
                        artist_id,
                        album_id,
                        t.track_no,
                        t.disc_no,
                        t.duration_ms,
                        &t.format,
                        t.sample_rate,
                        t.bit_depth,
                        t.channels,
                        t.is_lossless as i64,
                        indexed_at
                    ])
                    .map_err(|e| NcmError::Db(e.to_string()))?;
            }
        }
        tx.commit().map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(tracks.len())
    }

    /// 清理某媒体源下**不在本次索引 run 中**的曲目行（文件已删除/移走）。
    ///
    /// 依赖 `indexed_at < run_id` 判定，而不是把本次全部 path 传进 NOT IN——
    /// 十万级 path 列表会撞 SQLite 变量上限（默认 999），也白白放大 SQL。
    pub fn remove_stale_tracks(&self, source_id: i64, run_id: i64) -> Result<usize, NcmError> {
        self.conn
            .execute(
                "DELETE FROM tracks WHERE source_id = ?1 AND indexed_at < ?2",
                params![source_id, run_id],
            )
            .map_err(|e| NcmError::Db(e.to_string()))
    }

    /// 各媒体源的曲目计数（`source_id → count`；源管理页展示用）。
    pub fn source_track_counts(
        &self,
    ) -> Result<std::collections::HashMap<i64, i64>, NcmError> {
        let mut stmt = self
            .conn
            .prepare("SELECT source_id, COUNT(1) FROM tracks GROUP BY source_id")
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let map = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<std::collections::HashMap<i64, i64>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(map)
    }

    /// 曲目总数。
    pub fn count_tracks(&self) -> Result<i64, NcmError> {
        self.conn
            .query_row("SELECT COUNT(1) FROM tracks", [], |r| r.get(0))
            .map_err(|e| NcmError::Db(e.to_string()))
    }

    /// 分页读取曲目（按 path 稳定排序）。
    ///
    /// `limit` 硬上限 500：IPC 层禁止全量序列化，分页是契约而非建议。
    pub fn list_tracks(&self, limit: i64, offset: i64) -> Result<Vec<TrackRow>, NcmError> {
        let limit = limit.clamp(1, 500);
        let offset = offset.max(0);
        let mut stmt = self
            .conn
            .prepare(&format!("{TRACK_SELECT} ORDER BY t.path LIMIT ?1 OFFSET ?2"))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map(params![limit, offset], map_track_row)
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }

    /// 艺术家聚合列表（只含 ≥1 首曲目的艺术家；脏数据自愈）。
    pub fn list_artists(&self) -> Result<Vec<ArtistRow>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT ar.id, ar.name, COUNT(t.id) AS n
                 FROM artists ar JOIN tracks t ON t.artist_id = ar.id
                 GROUP BY ar.id, ar.name
                 ORDER BY n DESC, ar.name",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ArtistRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    track_count: r.get(2)?,
                })
            })
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }

    /// 专辑聚合列表（只含 ≥1 首曲目的专辑）。
    pub fn list_albums(&self) -> Result<Vec<AlbumRow>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT al.id, al.title, ar.name, al.year, COUNT(t.id) AS n
                 FROM albums al
                 JOIN tracks t ON t.album_id = al.id
                 LEFT JOIN artists ar ON ar.id = al.album_artist_id
                 GROUP BY al.id
                 ORDER BY n DESC, al.title",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(AlbumRow {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    artist: r.get(2)?,
                    year: r.get(3)?,
                    track_count: r.get(4)?,
                })
            })
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }

    /// 曲库总览统计（艺术家/专辑按**实际关联的**曲目去重计数）。
    pub fn library_stats(&self) -> Result<LibraryStats, NcmError> {
        let (tracks, total_size, total_duration_ms) = self
            .conn
            .query_row(
                "SELECT COUNT(1), COALESCE(SUM(size), 0), COALESCE(SUM(duration_ms), 0) FROM tracks",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let artists: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(DISTINCT artist_id) FROM tracks WHERE artist_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let albums: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(DISTINCT album_id) FROM tracks WHERE album_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(LibraryStats {
            tracks,
            artists,
            albums,
            total_size,
            total_duration_ms,
        })
    }

    /// 搜索曲目（标题 / 艺术家 / 专辑 / 路径，大小写不敏感 LIKE）。
    ///
    /// 通配符转义：用户输入的 `%` `_` `\` 按字面匹配（SQL 侧 `ESCAPE '\'`）——
    /// 否则输入一个 `%` 会命中全库。
    pub fn search_tracks(&self, query: &str, limit: i64) -> Result<Vec<TrackRow>, NcmError> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let pattern = format!("%{}%", escape_like(q));
        let limit = limit.clamp(1, 500);
        let mut stmt = self
            .conn
            .prepare(&format!(
                "{TRACK_SELECT}
                 WHERE t.title LIKE ?1 ESCAPE '\\'
                    OR ar.name LIKE ?1 ESCAPE '\\'
                    OR al.title LIKE ?1 ESCAPE '\\'
                    OR t.path LIKE ?1 ESCAPE '\\'
                 ORDER BY t.path LIMIT ?2"
            ))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map(params![pattern, limit], map_track_row)
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }
}

/// 曲目读取的公共 SELECT（`list_tracks` / `search_tracks` 共用，列序与
/// [`map_track_row`] 一一对应）。
const TRACK_SELECT: &str = "SELECT t.id, t.source_id, t.path, t.size, t.title, ar.name, al.title, \
     t.track_no, t.duration_ms, t.format, t.sample_rate, t.bit_depth, t.channels, t.is_lossless \
     FROM tracks t \
     LEFT JOIN artists ar ON ar.id = t.artist_id \
     LEFT JOIN albums  al ON al.id = t.album_id";

/// [`TRACK_SELECT`] 的行映射。
fn map_track_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<TrackRow> {
    Ok(TrackRow {
        id: r.get(0)?,
        source_id: r.get(1)?,
        path: r.get(2)?,
        size: r.get(3)?,
        title: r.get(4)?,
        artist: r.get(5)?,
        album: r.get(6)?,
        track_no: r.get(7)?,
        duration_ms: r.get(8)?,
        format: r.get(9)?,
        sample_rate: r.get(10)?,
        bit_depth: r.get(11)?,
        channels: r.get(12)?,
        is_lossless: r.get::<_, i64>(13)? != 0,
    })
}

/// 「trim 后非空」判定：空白字符串按缺失处理（对齐 tagger 的 `has_value` 语义）。
fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|v| !v.is_empty())
}

/// LIKE 模式转义（`\` 自身、`%`、`_`），配合 SQL 侧 `ESCAPE '\'` 使用。
fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
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
