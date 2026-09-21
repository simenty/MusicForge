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
/// - v3（P2 播放行为）：likes / play_history / playlists / playlist_items。
///   与 v1 同库共存：v1 表管"转换/缓存"，v2 表管"曲库视图"，v3 表管"用户行为"，
///   全部遵循同一铁律——**db 是可再生的，真相在文件系统**
///   （例外：likes 与 play_history 是**用户产生的一次性数据**，
///   删库即丢失，属于「本地偏好」而非缓存——这也是它留在本地库的理由）。
pub const SCHEMA_VERSION: u32 = 3;

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

/// v3（P2 播放行为层）建表 SQL。
///
/// - `likes`：以 track_id 为主键（天然去重，"喜欢"是布尔状态的物化）；
/// - `play_history`：**只增不改**的追加日志（清空 = DELETE 全表，
///   不做软删除——历史的意义就是"发生过"）；`played_at` 为 UNIX 秒；
/// - `playlists` / `playlist_items`：顺序由 `position` 显式维护
///   （不依赖 rowid——重排/插入不产生歧义）。
const V3_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS likes (
    track_id   INTEGER PRIMARY KEY REFERENCES tracks(id),
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS play_history (
    id         INTEGER PRIMARY KEY,
    track_id   INTEGER NOT NULL REFERENCES tracks(id),
    played_at  INTEGER NOT NULL,
    ms_played  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_history_time ON play_history(played_at DESC);
CREATE TABLE IF NOT EXISTS playlists (
    id         INTEGER PRIMARY KEY,
    name       TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS playlist_items (
    playlist_id INTEGER NOT NULL REFERENCES playlists(id),
    track_id    INTEGER NOT NULL REFERENCES tracks(id),
    position    INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, position)
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
    /// 本地封面缓存路径（在线元数据补全；None = 尚无封面）。
    /// core 只存路径不做 IO——文件落盘与抓取都在 shell 层。
    pub cover_path: Option<String>,
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

/// 播放历史行（v3）：曲目信息 + 播放时刻。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRow {
    pub track: TrackRow,
    /// 播放发生时刻（UNIX 秒）
    pub played_at: i64,
    /// 实际播放毫秒（可为 0——起播即记）
    pub ms_played: i64,
}

/// 最常播放行（v3）：曲目信息 + 历史累计播放次数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopTrackRow {
    pub track: TrackRow,
    pub play_count: i64,
}

/// 全局搜索命中（P6.14）：一次查询返回四组结果（每组各 limit 条）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchHits {
    pub tracks: Vec<TrackRow>,
    pub albums: Vec<AlbumRow>,
    pub artists: Vec<ArtistRow>,
    pub playlists: Vec<PlaylistRow>,
}

/// 歌单行（v3）：id + 名称 + 曲目数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistRow {
    pub id: i64,
    pub name: String,
    pub track_count: i64,
}

/// 曲目列表排序键（P6.21）。
///
/// `Default` 由各调用方回退到其自然序（库 = path 稳定序；喜欢 = 收藏时间倒序；
/// 历史 = 播放时间倒序）。所有变体映射到**固定 SQL 片段**（白名单，绝不拼接用户
/// 输入，杜绝注入）。`Title/Artist/Album` 用 `COLLATE NOCASE` 做大小写不敏感排序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrackSort {
    #[default]
    Default,
    Title,
    Artist,
    Album,
    Duration,
    PlayedAt,
    LikedAt,
    PlayCount,
}

impl TrackSort {
    /// 返回 `(extra_join, order_by)`：
    /// - `extra_join` 空串 = 无需额外 JOIN；`PlayCount` 需 LEFT JOIN 播放次数子查询；
    /// - `order_by` 空串 = 调用方自行决定自然序（见各 `list_*` 方法）。
    fn clause(self) -> (&'static str, &'static str) {
        match self {
            TrackSort::Default => ("", ""),
            TrackSort::Title => ("", "t.title COLLATE NOCASE"),
            TrackSort::Artist => ("", "ar.name COLLATE NOCASE"),
            TrackSort::Album => ("", "al.title COLLATE NOCASE"),
            TrackSort::Duration => ("", "t.duration_ms"),
            TrackSort::PlayedAt => ("", "h.played_at DESC"),
            TrackSort::LikedAt => ("", "lk.created_at DESC"),
            TrackSort::PlayCount => (
                "LEFT JOIN (SELECT track_id, COUNT(*) AS pc FROM play_history GROUP BY track_id) pc ON pc.track_id = t.id",
                "COALESCE(pc.pc, 0) DESC, t.path",
            ),
        }
    }
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
        if version < 3 {
            conn.execute_batch(V3_SCHEMA_SQL)
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

    /// 移除媒体源并连带清理其曲目与行为行；返回被清理的曲目数。
    ///
    /// **FK 是活的**（P2 实测纠正）：rusqlite bundled 编译默认带
    /// `SQLITE_DEFAULT_FOREIGN_KEYS=1`——历史行存在时直接 DELETE tracks
    /// 会报 `FOREIGN KEY constraint failed`。因此清理必须按依赖顺序在
    /// **单事务**内完成：play_history / likes / playlist_items → tracks → sources。
    ///
    /// 语义：曲目行消失即行为行消失（重扫同一目录会产生新 track id，
    /// 旧行为本就无法再关联；留着只会成为孤儿）。
    pub fn remove_source(&self, id: i64) -> Result<usize, NcmError> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        for sql in [
            "DELETE FROM play_history WHERE track_id IN \
             (SELECT id FROM tracks WHERE source_id = ?1)",
            "DELETE FROM likes WHERE track_id IN \
             (SELECT id FROM tracks WHERE source_id = ?1)",
            "DELETE FROM playlist_items WHERE track_id IN \
             (SELECT id FROM tracks WHERE source_id = ?1)",
        ] {
            tx.execute(sql, [id])
                .map_err(|e| NcmError::Db(e.to_string()))?;
        }
        let n = tx
            .execute("DELETE FROM tracks WHERE source_id = ?1", [id])
            .map_err(|e| NcmError::Db(e.to_string()))?;
        tx.execute("DELETE FROM sources WHERE id = ?1", [id])
            .map_err(|e| NcmError::Db(e.to_string()))?;
        tx.commit().map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(n)
    }

    /// 从资料库移除指定曲目（仅删索引行，不动文件）。FK 是活的（P2 实测），
    /// 必须由近及远在单事务内连带清理行为行，否则 `FOREIGN KEY constraint failed`。
    ///
    /// 返回被删除的曲目行数；空切片直接返回 0（不开启事务）。
    pub fn remove_tracks(&self, ids: &[i64]) -> Result<usize, NcmError> {
        if ids.is_empty() {
            return Ok(0);
        }
        let placeholders = vec!["?"; ids.len()].join(",");
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        for table in ["play_history", "likes", "playlist_items"] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE track_id IN ({placeholders})"),
                rusqlite::params_from_iter(ids.iter()),
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        }
        let n = tx
            .execute(
                &format!("DELETE FROM tracks WHERE id IN ({placeholders})"),
                rusqlite::params_from_iter(ids.iter()),
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        tx.commit().map_err(|e| NcmError::Db(e.to_string()))?;
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

    /// 清理某媒体源下**不在本次索引 run 中**的曲目行（文件已删除/移走），
    /// 连带清理其行为行（FK 依赖顺序与 [`Db::remove_source`] 相同）。
    ///
    /// 依赖 `indexed_at < run_id` 判定，而不是把本次全部 path 传进 NOT IN——
    /// 十万级 path 列表会撞 SQLite 变量上限（默认 999），也白白放大 SQL。
    pub fn remove_stale_tracks(&self, source_id: i64, run_id: i64) -> Result<usize, NcmError> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        for sql in [
            "DELETE FROM play_history WHERE track_id IN \
             (SELECT id FROM tracks WHERE source_id = ?1 AND indexed_at < ?2)",
            "DELETE FROM likes WHERE track_id IN \
             (SELECT id FROM tracks WHERE source_id = ?1 AND indexed_at < ?2)",
            "DELETE FROM playlist_items WHERE track_id IN \
             (SELECT id FROM tracks WHERE source_id = ?1 AND indexed_at < ?2)",
        ] {
            tx.execute(sql, params![source_id, run_id])
                .map_err(|e| NcmError::Db(e.to_string()))?;
        }
        let n = tx
            .execute(
                "DELETE FROM tracks WHERE source_id = ?1 AND indexed_at < ?2",
                params![source_id, run_id],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        tx.commit().map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(n)
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

    /// 分页读取曲目（P6.21：支持排序；`Default` = path 稳定序）。
    ///
    /// `limit` 硬上限 500：IPC 层禁止全量序列化，分页是契约而非建议。
    pub fn list_tracks_sorted(
        &self,
        sort: TrackSort,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TrackRow>, NcmError> {
        let limit = limit.clamp(1, 500);
        let offset = offset.max(0);
        let (join, order_raw) = sort.clause();
        let order = if order_raw.is_empty() { "t.path" } else { order_raw };
        let mut stmt = self
            .conn
            .prepare(&format!("{TRACK_SELECT} {join} ORDER BY {order} LIMIT ?1 OFFSET ?2"))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map(params![limit, offset], map_track_row)
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }

    /// 分页读取曲目（按 path 稳定排序；P6.21 委托给 [`list_tracks_sorted`]）。
    pub fn list_tracks(&self, limit: i64, offset: i64) -> Result<Vec<TrackRow>, NcmError> {
        self.list_tracks_sorted(TrackSort::Default, limit, offset)
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
                "SELECT al.id, al.title, ar.name, al.year, COUNT(t.id) AS n, al.cover_cache
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
                    cover_path: r.get(5)?,
                })
            })
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }

    /// 写入专辑封面缓存路径（文件由调用方负责落盘）。
    pub fn set_album_cover(&self, album_id: i64, cover_path: &str) -> Result<(), NcmError> {
        self.conn
            .execute(
                "UPDATE albums SET cover_cache = ?1 WHERE id = ?2",
                params![cover_path, album_id],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }

    /// 回填专辑年份（在线元数据：MusicBrainz first-release-date）。
    /// **本地已有年份时不覆盖**（`year IS NULL` 守卫——用户标签优先）。
    pub fn set_album_year(&self, album_id: i64, year: i64) -> Result<(), NcmError> {
        self.conn
            .execute(
                "UPDATE albums SET year = ?1 WHERE id = ?2 AND year IS NULL",
                params![year, album_id],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }

    /// 某艺术家的全部曲目（按专辑名 / 碟号 / 轨号排序——详情页播放序）。
    pub fn tracks_by_artist(&self, artist_id: i64) -> Result<Vec<TrackRow>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "{TRACK_SELECT}
                 WHERE t.artist_id = ?1
                 ORDER BY al.title, t.disc_no, t.track_no, t.title"
            ))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([artist_id], map_track_row)
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }

    /// 某专辑的曲目（按碟号 / 轨号排序——详情页播放序）。
    pub fn tracks_by_album(&self, album_id: i64) -> Result<Vec<TrackRow>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "{TRACK_SELECT}
                 WHERE t.album_id = ?1
                 ORDER BY t.disc_no, t.track_no, t.title"
            ))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([album_id], map_track_row)
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }

    /// 按 id 取单曲（显示名已解析；不存在 → None）。
    pub fn get_track(&self, track_id: i64) -> Result<Option<TrackRow>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(&format!("{TRACK_SELECT} WHERE t.id = ?1"))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([track_id], map_track_row)
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows.into_iter().next())
    }

    /// 某曲目所在专辑的封面路径（底栏封面用；无专辑 / 无封面 → None）。
    pub fn track_cover_path(&self, track_id: i64) -> Result<Option<String>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT al.cover_cache
                 FROM tracks t JOIN albums al ON al.id = t.album_id
                 WHERE t.id = ?1",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([track_id], |r| r.get::<_, Option<String>>(0))
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows.into_iter().next().flatten())
    }

    /// 某艺术家曲目数最多的专辑（id + 封面路径）——艺术家图片的**降级来源**：
    /// 没有免 key 的艺术家图片源时，用其最热门专辑的封面代表（P6 决策，见记忆）。
    pub fn artist_top_album(
        &self,
        artist_id: i64,
    ) -> Result<Option<(i64, Option<String>)>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT al.id, al.cover_cache
                 FROM tracks t JOIN albums al ON al.id = t.album_id
                 WHERE t.artist_id = ?1
                 GROUP BY al.id
                 ORDER BY COUNT(t.id) DESC
                 LIMIT 1",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([artist_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows.into_iter().next())
    }

    /// 尚无封面的专辑（id、标题、专辑艺人名）——在线封面补全的输入，
    /// 按曲目数降序（优先补最可见的专辑）。
    pub fn albums_missing_cover(
        &self,
        limit: i64,
    ) -> Result<Vec<(i64, String, Option<String>)>, NcmError> {
        let limit = limit.clamp(1, 500);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT al.id, al.title, ar.name
                 FROM albums al
                 JOIN tracks t ON t.album_id = al.id
                 LEFT JOIN artists ar ON ar.id = al.album_artist_id
                 WHERE al.cover_cache IS NULL
                 GROUP BY al.id
                 ORDER BY COUNT(t.id) DESC, al.title
                 LIMIT ?1",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
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

    /// 全局搜索（P6.14）：曲目 + 专辑 + 艺术家 + 歌单，每组各 `limit` 条。
    ///
    /// 曲目谓词与 `search_tracks` 完全一致（标题/艺术家/专辑/路径；通配符按字面
    /// 转义）；空查询 → 四组全空。一次查询四组——搜索面板开一次只发一次 IPC。
    pub fn search_all(&self, query: &str, limit: i64) -> Result<SearchHits, NcmError> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(SearchHits::default());
        }
        let pattern = format!("%{}%", escape_like(q));
        let lim = limit.clamp(1, 50);

        let tracks = self.search_tracks(q, lim)?;

        // 专辑：标题或专辑艺人名命中（只含 ≥1 首曲目的专辑）
        let mut stmt = self
            .conn
            .prepare(
                "SELECT al.id, al.title, ar.name, al.year, COUNT(t.id) AS n, al.cover_cache
                 FROM albums al
                 JOIN tracks t ON t.album_id = al.id
                 LEFT JOIN artists ar ON ar.id = al.album_artist_id
                 WHERE al.title LIKE ?1 ESCAPE '\\' OR ar.name LIKE ?1 ESCAPE '\\'
                 GROUP BY al.id
                 ORDER BY n DESC, al.title
                 LIMIT ?2",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let albums = stmt
            .query_map(params![pattern, lim], |r| {
                Ok(AlbumRow {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    artist: r.get(2)?,
                    year: r.get(3)?,
                    track_count: r.get(4)?,
                    cover_path: r.get(5)?,
                })
            })
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;

        // 艺术家（只含有曲目的）
        let mut stmt = self
            .conn
            .prepare(
                "SELECT ar.id, ar.name, COUNT(t.id) AS n
                 FROM artists ar JOIN tracks t ON t.artist_id = ar.id
                 WHERE ar.name LIKE ?1 ESCAPE '\\'
                 GROUP BY ar.id, ar.name
                 ORDER BY n DESC, ar.name
                 LIMIT ?2",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let artists = stmt
            .query_map(params![pattern, lim], |r| {
                Ok(ArtistRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    track_count: r.get(2)?,
                })
            })
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;

        // 歌单（按名；空歌单也可见——它是用户资产）
        let mut stmt = self
            .conn
            .prepare(
                "SELECT p.id, p.name, COUNT(pi.track_id) AS n
                 FROM playlists p LEFT JOIN playlist_items pi ON pi.playlist_id = p.id
                 WHERE p.name LIKE ?1 ESCAPE '\\'
                 GROUP BY p.id, p.name
                 ORDER BY p.created_at DESC
                 LIMIT ?2",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let playlists = stmt
            .query_map(params![pattern, lim], |r| {
                Ok(PlaylistRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    track_count: r.get(2)?,
                })
            })
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;

        Ok(SearchHits {
            tracks,
            albums,
            artists,
            playlists,
        })
    }

    // ------------------------------------------------------------ v3 行为 --

    /// 设置/取消「喜欢」（幂等——重复设置同状态不产生副作用）。
    pub fn set_like(&self, track_id: i64, liked: bool) -> Result<(), NcmError> {
        if liked {
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO likes (track_id, created_at) VALUES (?1, unixepoch())",
                    [track_id],
                )
                .map_err(|e| NcmError::Db(e.to_string()))?;
        } else {
            self.conn
                .execute("DELETE FROM likes WHERE track_id = ?1", [track_id])
                .map_err(|e| NcmError::Db(e.to_string()))?;
        }
        Ok(())
    }

    /// 切换「喜欢」，返回切换后的状态（true = 已喜欢）。
    pub fn toggle_like(&self, track_id: i64) -> Result<bool, NcmError> {
        let liked = self.is_liked(track_id)?;
        self.set_like(track_id, !liked)?;
        Ok(!liked)
    }

    /// 批量取消喜欢（仅删 `likes` 行；不动曲目/文件）。返回被取消的条数；
    /// 空切片直接返回 0（不开启事务）。P6.23 收藏页批量清理用。
    pub fn unlike_tracks(&self, ids: &[i64]) -> Result<usize, NcmError> {
        if ids.is_empty() {
            return Ok(0);
        }
        let placeholders = vec!["?"; ids.len()].join(",");
        self.conn
            .execute(
                &format!("DELETE FROM likes WHERE track_id IN ({placeholders})"),
                rusqlite::params_from_iter(ids.iter()),
            )
            .map_err(|e| NcmError::Db(e.to_string()))
    }

    /// 是否已喜欢。
    pub fn is_liked(&self, track_id: i64) -> Result<bool, NcmError> {
        let n: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(1) FROM likes WHERE track_id = ?1",
                [track_id],
                |r| r.get(0),
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(n > 0)
    }

    /// 喜欢列表（P6.21：支持排序；`Default` = 收藏时间倒序）。
    /// 曲目已被移除的行自动消失——INNER JOIN。
    pub fn list_liked_sorted(
        &self,
        sort: TrackSort,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TrackRow>, NcmError> {
        let limit = limit.clamp(1, 500);
        let offset = offset.max(0);
        let (join, order_raw) = sort.clause();
        // `PlayedAt` 需要 history JOIN（本查询未 JOIN），回退默认序。
        let order = match (sort, order_raw.is_empty()) {
            (TrackSort::PlayedAt, _) | (_, true) => "lk.created_at DESC, t.id DESC",
            _ => order_raw,
        };
        let mut stmt = self
            .conn
            .prepare(&format!(
                "{TRACK_SELECT}
                 JOIN likes lk ON lk.track_id = t.id
                 {join}
                 ORDER BY {order}
                 LIMIT ?1 OFFSET ?2"
            ))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map(params![limit, offset], map_track_row)
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }

    /// 喜欢列表（按收藏时间倒序；P6.21 委托给 [`list_liked_sorted`]）。
    pub fn list_liked(&self, limit: i64, offset: i64) -> Result<Vec<TrackRow>, NcmError> {
        self.list_liked_sorted(TrackSort::Default, limit, offset)
    }

    /// 喜欢总数。
    pub fn liked_count(&self) -> Result<i64, NcmError> {
        self.conn
            .query_row("SELECT COUNT(1) FROM likes", [], |r| r.get(0))
            .map_err(|e| NcmError::Db(e.to_string()))
    }

    /// 全部已喜欢的曲目 id（前端一次性拉取做行状态判定——避免逐行查询）。
    ///
    /// 刻意不分页：likes 是用户显式行为，量级远小于曲库本身；即便上万条，
    /// 一个 i64 数组的传输也远小于分页往返的成本。
    pub fn all_liked_ids(&self) -> Result<Vec<i64>, NcmError> {
        let mut stmt = self
            .conn
            .prepare("SELECT track_id FROM likes ORDER BY created_at DESC, track_id DESC")
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| r.get(0))
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<i64>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }

    /// 追加一条播放记录（**追加日志**，不做去重——同一曲目多次播放就是多行）。
    pub fn record_play(
        &self,
        track_id: i64,
        played_at: i64,
        ms_played: i64,
    ) -> Result<(), NcmError> {
        self.conn
            .execute(
                "INSERT INTO play_history (track_id, played_at, ms_played) VALUES (?1, ?2, ?3)",
                params![track_id, played_at, ms_played],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }

    /// 播放历史（P6.21：支持排序；`Default` = 播放时间倒序；含曲目信息）。
    pub fn list_history_sorted(&self, sort: TrackSort, limit: i64) -> Result<Vec<HistoryRow>, NcmError> {
        let limit = limit.clamp(1, 500);
        let (join, order_raw) = sort.clause();
        // `LikedAt` 需要 likes JOIN（本查询未 JOIN），回退默认序。
        let order = match (sort, order_raw.is_empty()) {
            (TrackSort::LikedAt, _) | (_, true) => "h.played_at DESC, h.id DESC",
            _ => order_raw,
        };
        // 列序与 map_track_row 保持一一对应（0..13），14/15 为历史列。
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT t.id, t.source_id, t.path, t.size, t.title, ar.name, al.title, \
                 t.track_no, t.duration_ms, t.format, t.sample_rate, t.bit_depth, t.channels, \
                 t.is_lossless, h.played_at, h.ms_played \
                 FROM play_history h \
                 JOIN tracks t ON t.id = h.track_id \
                 LEFT JOIN artists ar ON ar.id = t.artist_id \
                 LEFT JOIN albums  al ON al.id = t.album_id \
                 {join} ORDER BY {order} LIMIT ?1"
            ))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([limit], |r| {
                Ok(HistoryRow {
                    track: map_track_row(r)?,
                    played_at: r.get(14)?,
                    ms_played: r.get(15)?,
                })
            })
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }

    /// 播放历史（按时间倒序；P6.21 委托给 [`list_history_sorted`]）。
    pub fn list_history(&self, limit: i64) -> Result<Vec<HistoryRow>, NcmError> {
        self.list_history_sorted(TrackSort::Default, limit)
    }

    /// 清空播放历史；返回被清空的行数。
    pub fn clear_history(&self) -> Result<usize, NcmError> {
        self.conn
            .execute("DELETE FROM play_history", [])
            .map_err(|e| NcmError::Db(e.to_string()))
    }

    // ------------------------------------------------------ v3 统计视图 --

    /// 最常播放的曲目（按播放次数降序；同次数按标题稳定排序）。
    pub fn top_tracks(&self, limit: i64) -> Result<Vec<TopTrackRow>, NcmError> {
        let limit = limit.clamp(1, 100);
        // 列序 0..13 与 map_track_row 对齐，14 为聚合列
        let mut stmt = self
            .conn
            .prepare(
                "SELECT t.id, t.source_id, t.path, t.size, t.title, ar.name, al.title, \
                 t.track_no, t.duration_ms, t.format, t.sample_rate, t.bit_depth, t.channels, \
                 t.is_lossless, COUNT(h.id) AS plays \
                 FROM play_history h \
                 JOIN tracks t ON t.id = h.track_id \
                 LEFT JOIN artists ar ON ar.id = t.artist_id \
                 LEFT JOIN albums  al ON al.id = t.album_id \
                 GROUP BY t.id \
                 ORDER BY plays DESC, t.title \
                 LIMIT ?1",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([limit], |r| {
                Ok(TopTrackRow {
                    track: map_track_row(r)?,
                    play_count: r.get(14)?,
                })
            })
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }

    /// 播放总览：`(总播放次数, 去重曲目数)`。
    pub fn history_totals(&self) -> Result<(i64, i64), NcmError> {
        self.conn
            .query_row(
                "SELECT COUNT(1), COUNT(DISTINCT track_id) FROM play_history",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| NcmError::Db(e.to_string()))
    }

    /// 按天播放计数（`since_secs` 之后；`day` 为**本地时区** `YYYY-MM-DD`，按日升序）。
    ///
    /// 时区交给 SQLite 的 `localtime` 修饰符——与前端分组（HistoryPage 的
    /// `new Date()` 本地时区）保持同一口径，避免跨 midnight 的双标。
    pub fn daily_play_counts(&self, since_secs: i64) -> Result<Vec<(String, i64)>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT date(played_at, 'unixepoch', 'localtime') AS day, COUNT(1) \
                 FROM play_history WHERE played_at >= ?1 GROUP BY day ORDER BY day",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([since_secs], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }

    /// 最近播放（**按曲目去重**，取每首曲目的最近一次；首页「继续聆听」）。
    pub fn recent_tracks(&self, limit: i64) -> Result<Vec<TrackRow>, NcmError> {
        let limit = limit.clamp(1, 100);
        let mut stmt = self
            .conn
            .prepare(&format!(
                "{TRACK_SELECT}
                 JOIN (SELECT track_id, MAX(played_at) AS last_at
                       FROM play_history GROUP BY track_id) h
                   ON h.track_id = t.id
                 ORDER BY h.last_at DESC
                 LIMIT ?1"
            ))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([limit], map_track_row)
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }

    // -------------------------------------------------------- 歌单（P6.4）--

    /// 创建歌单；名称 trim 后不得为空。返回新 id。
    pub fn create_playlist(&self, name: &str) -> Result<i64, NcmError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(NcmError::Db("歌单名称不能为空".to_string()));
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        self.conn
            .execute(
                "INSERT INTO playlists(name, created_at) VALUES (?1, ?2)",
                params![name, now],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 歌单列表（创建序；含曲目数）。
    pub fn list_playlists(&self) -> Result<Vec<PlaylistRow>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT p.id, p.name, COUNT(i.track_id) AS n
                 FROM playlists p
                 LEFT JOIN playlist_items i ON i.playlist_id = p.id
                 GROUP BY p.id
                 ORDER BY p.created_at, p.id",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(PlaylistRow {
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

    /// 歌单内曲目（按 position 顺序；显示名已解析）。
    /// INNER JOIN 语义：曲目行被移除（源删除）后条目自动隐藏。
    pub fn playlist_tracks(&self, playlist_id: i64) -> Result<Vec<TrackRow>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "{TRACK_SELECT}
                 JOIN playlist_items pi ON pi.track_id = t.id AND pi.playlist_id = ?1
                 ORDER BY pi.position"
            ))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([playlist_id], map_track_row)
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }

    /// 追加曲目到歌单：跳过已在歌单中的、跳过不存在的 track id；
    /// position 从尾部续接。返回实际追加数（单事务）。
    pub fn playlist_add_tracks(
        &self,
        playlist_id: i64,
        track_ids: &[i64],
    ) -> Result<usize, NcmError> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let exists: i64 = tx
            .query_row(
                "SELECT COUNT(1) FROM playlists WHERE id = ?1",
                [playlist_id],
                |r| r.get(0),
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        if exists == 0 {
            return Err(NcmError::Db(format!("歌单 {playlist_id} 不存在")));
        }
        let mut have: std::collections::HashSet<i64> = {
            let mut stmt = tx
                .prepare("SELECT track_id FROM playlist_items WHERE playlist_id = ?1")
                .map_err(|e| NcmError::Db(e.to_string()))?;
            let rows = stmt
                .query_map([playlist_id], |r| r.get::<_, i64>(0))
                .map_err(|e| NcmError::Db(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| NcmError::Db(e.to_string()))?;
            rows.into_iter().collect()
        };
        let mut pos: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(position), -1) + 1 FROM playlist_items WHERE playlist_id = ?1",
                [playlist_id],
                |r| r.get(0),
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let mut added = 0usize;
        for &id in track_ids {
            if have.contains(&id) {
                continue;
            }
            // 曲目不存在则跳过（FK 是活的——静默跳过比报 FK 错更符合"批量加"语义）
            let ok: i64 = tx
                .query_row("SELECT COUNT(1) FROM tracks WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .map_err(|e| NcmError::Db(e.to_string()))?;
            if ok == 0 {
                continue;
            }
            tx.execute(
                "INSERT INTO playlist_items(playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
                params![playlist_id, id, pos],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
            have.insert(id);
            pos += 1;
            added += 1;
        }
        tx.commit().map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(added)
    }

    /// 从歌单移除曲目（后续 position 前移，保持连续；单事务）。
    pub fn playlist_remove_track(&self, playlist_id: i64, track_id: i64) -> Result<(), NcmError> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let pos: Option<i64> = tx
            .query_row(
                "SELECT position FROM playlist_items WHERE playlist_id = ?1 AND track_id = ?2",
                params![playlist_id, track_id],
                |r| r.get(0),
            )
            .ok();
        if let Some(p) = pos {
            tx.execute(
                "DELETE FROM playlist_items WHERE playlist_id = ?1 AND position = ?2",
                params![playlist_id, p],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
            tx.execute(
                "UPDATE playlist_items SET position = position - 1 \
                 WHERE playlist_id = ?1 AND position > ?2",
                params![playlist_id, p],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        }
        tx.commit().map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }

    /// 歌单内移动曲目到目标下标（拖拽排序；position 重排后保持连续 0..n-1）。
    /// 越界下标 clamp；曲目不在歌单 → 空操作。
    ///
    /// ⚠️ `playlist_items` 的 PK 是 `(playlist_id, position)`：逐行 ±1 的
    /// "中间段平移"会中途撞唯一约束（SQLite 逐行更新，顺序不保证）。
    /// 实现走**两阶段**：先把全部 position 抬到过渡区间（+100000），
    /// 再按新顺序逐行写回——无约束技巧依赖，长度上限远低于偏移量。
    pub fn playlist_move_track(
        &self,
        playlist_id: i64,
        track_id: i64,
        to_index: i64,
    ) -> Result<(), NcmError> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        // ① 读全序（按 position）
        let mut ids: Vec<i64> = {
            let mut stmt = tx
                .prepare(
                    "SELECT track_id FROM playlist_items WHERE playlist_id = ?1 ORDER BY position",
                )
                .map_err(|e| NcmError::Db(e.to_string()))?;
            let rows = stmt
                .query_map([playlist_id], |r| r.get::<_, i64>(0))
                .map_err(|e| NcmError::Db(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| NcmError::Db(e.to_string()))?;
            rows
        };
        let Some(pos) = ids.iter().position(|&x| x == track_id) else {
            return Ok(()); // 不在歌单 → 空操作
        };
        let to = to_index.clamp(0, (ids.len() as i64 - 1).max(0)) as usize;
        if pos == to {
            return Ok(());
        }
        // ② 新顺序
        let moved = ids.remove(pos);
        ids.insert(to, moved);
        // ③ 抬到过渡区间（脱离 0..n-1，消除约束冲突）
        tx.execute(
            "UPDATE playlist_items SET position = position + 100000 WHERE playlist_id = ?1",
            [playlist_id],
        )
        .map_err(|e| NcmError::Db(e.to_string()))?;
        // ④ 按新顺序写回
        for (i, id) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE playlist_items SET position = ?1 WHERE playlist_id = ?2 AND track_id = ?3",
                params![i as i64, playlist_id, id],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        }
        tx.commit().map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }

    /// 重命名歌单（名称 trim 后不得为空）。
    pub fn playlist_rename(&self, playlist_id: i64, name: &str) -> Result<(), NcmError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(NcmError::Db("歌单名称不能为空".to_string()));
        }
        self.conn
            .execute(
                "UPDATE playlists SET name = ?1 WHERE id = ?2",
                params![name, playlist_id],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }

    /// 删除歌单及其条目（FK 无级联——手动两条 DELETE，单事务；**不动曲目行**）。
    pub fn playlist_delete(&self, playlist_id: i64) -> Result<(), NcmError> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        tx.execute(
            "DELETE FROM playlist_items WHERE playlist_id = ?1",
            [playlist_id],
        )
        .map_err(|e| NcmError::Db(e.to_string()))?;
        tx.execute("DELETE FROM playlists WHERE id = ?1", [playlist_id])
            .map_err(|e| NcmError::Db(e.to_string()))?;
        tx.commit().map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }

    /// 歌单封面候选（P6.12）：`(playlist_id, cover_path)`——按歌单内曲序，
    /// 每个歌单至多 `per` 条且路径去重（拼贴四格不应重复同图）。
    pub fn playlist_covers(&self, per: i64) -> Result<Vec<(i64, String)>, NcmError> {
        let per = per.clamp(1, 8) as usize;
        let mut stmt = self
            .conn
            .prepare(
                "SELECT pi.playlist_id, al.cover_cache
                 FROM playlist_items pi
                 JOIN tracks t ON t.id = pi.track_id
                 JOIN albums al ON al.id = t.album_id
                 WHERE al.cover_cache IS NOT NULL
                 ORDER BY pi.playlist_id, pi.position",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        // 每单去重 + 截断（候选行本身很小，内存侧处理；SQL 窗口函数不值得）
        let mut out: Vec<(i64, String)> = Vec::new();
        let mut seen: std::collections::HashMap<i64, std::collections::HashSet<String>> =
            std::collections::HashMap::new();
        for (pid, path) in rows {
            let set = seen.entry(pid).or_default();
            if set.len() < per && set.insert(path.clone()) {
                out.push((pid, path));
            }
        }
        Ok(out)
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
