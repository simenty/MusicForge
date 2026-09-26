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
//!
//! P3-24：本文件原为 `db.rs`（2666 行）。按领域把 `impl Db` 的方法体拆分到
//! `db/db_impl/*.rs`（逐字 `include!` 拼回，公开 API / 行为不变），本文件保留：
//! 类型定义、`Db` 构造器（open/open_in_memory/migrate）、跨域私有 helper
//! （`TRACK_SELECT`/`map_track_row`/`track_filter_pred`/`escape_like`/`non_empty`）、
//! 配置目录工具函数、单元测试。拆分仅为可维护性，无行为变更。

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

/// SQLite 单语句 host 参数上限（rusqlite bundled 默认 `SQLITE_MAX_VARIABLE_NUMBER`
/// = 999；见 2026-09-12 设计备忘：十万 path 直接撞上限）。`IN (?,?,...)` 由用户
/// 传入的 `Vec<i64>` 拼占位符时，**每批不得超过此值**——留 99 余量防边界波动。
/// 超过即在 [`Db::remove_tracks`] / [`Db::unlike_tracks`] 内拆批执行。
const MAX_SQL_VARS: usize = 900;

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

/// 文件索引批量写入行：`(path, size, mtime, format, sha256)`——`upsert_files_batch` 的形参。
type FileIndexRow = (String, i64, Option<i64>, Option<String>, Option<String>);

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
        // P1-9：忙等待而非立即失败。原实现每请求 `Db::open` 并发读写，遇锁直接
        // `SQLITE_BUSY` 失败；设 5s busy_timeout 后 SQLite 自行等待，调用方无需
        // 处理瞬时锁争用。（server 连接的共享复用属更大重构，留待单独评估。）
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Self::migrate(&conn)?;
        // P1-7：补两个高频查询索引。幂等——已建库不重建；须在 migrate（建表）之后。
        // 覆盖：最近播放 / 播放排行（GROUP BY track_id）/ 批量删曲（WHERE track_id IN）。
        let _ = conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_history_track ON play_history(track_id);
             CREATE INDEX IF NOT EXISTS idx_playlist_items_track ON playlist_items(track_id);",
        );
        Ok(Self { conn })
    }

    /// 内存库（测试用）。
    pub fn open_in_memory() -> Result<Self, NcmError> {
        let conn = Connection::open_in_memory().map_err(|e| NcmError::Db(e.to_string()))?;
        // 与 `open()` 同源：busy_timeout + 两个高频查询索引（P1-9 / P1-7）。
        // 注意：索引必须在 `migrate`（建表）之后创建，否则表尚不存在被静默跳过。
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Self::migrate(&conn)?;
        let _ = conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_history_track ON play_history(track_id);
             CREATE INDEX IF NOT EXISTS idx_playlist_items_track ON playlist_items(track_id);",
        );
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
        if version == SCHEMA_VERSION {
            return Ok(()); // 已是目标版本，无需迁移（避免无谓事务）
        }
        // P1-8：迁移包进**单个事务**。原实现三段 execute_batch + user_version
        // 各自独立——跨级失败会留下**半套 schema** 且 user_version 不推进，库永久
        // 卡在中间态、无法自愈（审计 Top5 数据·不可恢复）。包事务后，任何一级失败
        // 整体回滚，下次打开可安全重试。
        conn.execute("BEGIN", [])
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let outcome: Result<(), NcmError> = (|| {
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
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(|e| NcmError::Db(e.to_string()))?;
            Ok(())
        })();
        match outcome {
            Ok(()) => {
                conn.execute("COMMIT", [])
                    .map_err(|e| NcmError::Db(e.to_string()))?;
            }
            Err(e) => {
                let _ = conn.execute("ROLLBACK", []);
                return Err(e);
            }
        }
        Ok(())
    }
}

// ---- v1 文件索引 / 任务 / 确认（impl Db 方法体，逐字 include! 拼回，模块级合法位置）----
include!("db_impl/files.rs");
// ---- v2 曲库（源 / 曲目写入 / 清理）----
include!("db_impl/sources.rs");
// ---- v2 曲目读 / 搜索 ----
include!("db_impl/tracks.rs");
// ---- v3 喜欢 ----
include!("db_impl/liked.rs");
// ---- v3 历史 ----
include!("db_impl/history.rs");
// ---- v3 统计视图 ----
include!("db_impl/stats.rs");
// ---- 歌单（P6.4）----
include!("db_impl/playlists.rs");

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

/// P6.25 列表文本过滤谓词：匹配 **标题 / 艺术家 / 专辑 / 路径**。
///
/// 返回 `Some((sql_fragment, pattern))`：
/// - `sql_fragment` 可直接拼在 `WHERE` 之后，占位符统一用调用方给定的 `idx`
///   （SQLite 允许同一个 `?N` 在一句中重复出现，四处只绑一次）；
/// - `pattern` **必须以绑定参数传入**——`escape_like` 只处理 LIKE 元字符，
///   **不做 SQL 字符串转义**；若把用户输入拼进 SQL 字面量会留下注入口子
///   （单引号不受 `escape_like` 约束）。
///
/// 空的/纯空白查询 → `None`（调用方据此省略 WHERE，行为与不过滤一致）。
fn track_filter_pred(query: &str, idx: usize) -> Option<(String, String)> {
    let raw = query.trim();
    if raw.is_empty() {
        return None;
    }
    let pat = format!("%{}%", escape_like(raw));
    let p = format!("?{idx}");
    Some((
        format!(
            "(t.title LIKE {p} ESCAPE '\\' OR ar.name LIKE {p} ESCAPE '\\' \
             OR al.title LIKE {p} ESCAPE '\\' OR t.path LIKE {p} ESCAPE '\\')"
        ),
        pat,
    ))
}

#[cfg(test)]
// 单元测试模块置于文件中部（紧邻 playlist 清理实现），用 allow 关闭
// `items_after_test_module` 风格 lint——本库此前无单元测，集中放尾部会割裂
// 与被测函数的对应关系。
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    /// P1-7：open 必须为高频查询建立两个索引（最近播放/播放排行/批量删曲）。
    #[test]
    fn indexes_created_on_open_p1() {
        let db = Db::open_in_memory().unwrap();
        let names: Vec<String> = db
            .conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='index' \
                 AND name IN ('idx_history_track','idx_playlist_items_track')",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(
            names.len(),
            2,
            "两个索引都应存在（覆盖 play_history / playlist_items 的 track_id 查询）"
        );
    }

    /// 造一个干净的 3 曲歌单（a/b/c 各一次）。
    fn seed_clean() -> (Db, i64, i64, i64, i64) {
        let db = Db::open_in_memory().unwrap();
        let sid = db.upsert_source("/m", None).unwrap();
        let mk = |path: &str, title: &str| TrackInput {
            source_id: sid,
            path: path.to_string(),
            size: 1024,
            title: Some(title.to_string()),
            artist: Some("A".to_string()),
            ..Default::default()
        };
        db.upsert_tracks_batch(
            &[
                mk("/m/a.flac", "a"),
                mk("/m/b.flac", "b"),
                mk("/m/c.flac", "c"),
            ],
            1,
        )
        .unwrap();
        let tracks = db.list_tracks(10, 0).unwrap();
        let (id_a, id_b, id_c) = (tracks[0].id, tracks[1].id, tracks[2].id);
        let pid = db.create_playlist("clean").unwrap();
        db.playlist_add_tracks(pid, &[id_a, id_b, id_c]).unwrap();
        (db, pid, id_a, id_b, id_c)
    }

    /// 智能清理：移除重复副本（保留首条）+ 失效条目，并连续重排位置。
    #[test]
    fn playlist_smart_cleanup_removes_duplicates_and_orphans() {
        let (db, pid, id_a, _b, _c) = seed_clean();
        // 注入：a 的重复副本（position 3）
        db.conn
            .execute(
                "INSERT INTO playlist_items(playlist_id, track_id, position) VALUES (?1, ?2, 3)",
                rusqlite::params![pid, id_a],
            )
            .unwrap();
        // 注入失效条目（track_id 不存在）：活 FK 会拦截，故临时关闭以构造孤儿场景
        // （真实成因：重新索引源产生新 track id；此处仅验证清理分支能正确移除）
        db.conn.execute("PRAGMA foreign_keys = OFF", []).unwrap();
        db.conn
            .execute(
                "INSERT INTO playlist_items(playlist_id, track_id, position) VALUES (?1, ?2, 4)",
                rusqlite::params![pid, 999_999],
            )
            .unwrap();
        db.conn.execute("PRAGMA foreign_keys = ON", []).unwrap();

        assert_eq!(
            db.playlist_cleanup_preview(pid).unwrap(),
            (1, 1),
            "预览：1 重复副本 + 1 失效条目"
        );

        let (dups, orphans) = db.playlist_smart_cleanup(pid).unwrap();
        assert_eq!((dups, orphans), (1, 1), "执行：移除 1 重复 + 1 失效");

        let items = db.playlist_tracks(pid).unwrap();
        assert_eq!(items.len(), 3, "清理后仅剩 3 条有效曲目");
        assert_eq!(items[0].id, id_a, "重复副本已去；首条保留");

        // 位置重排为连续 0..n-1
        let positions: Vec<i64> = {
            let mut stmt = db
                .conn
                .prepare(
                    "SELECT position FROM playlist_items WHERE playlist_id = ?1 ORDER BY position",
                )
                .unwrap();
            stmt.query_map([pid], |r| r.get::<_, i64>(0))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        assert_eq!(positions, vec![0, 1, 2], "位置重排连续");
        assert_eq!(db.list_playlists().unwrap()[0].track_count, 3, "计数同步");
    }

    /// P6.25 文本过滤：按 标题 / 艺术家 / 专辑 / 路径 匹配，且行数随之收敛。
    #[test]
    fn tracks_filter_matches_title_artist_album_and_path() {
        let db = Db::open_in_memory().unwrap();
        let sid = db.upsert_source("/m", None).unwrap();
        let mk = |path: &str, title: &str, artist: &str, album: &str| TrackInput {
            source_id: sid,
            path: path.to_string(),
            size: 1024,
            title: Some(title.to_string()),
            artist: Some(artist.to_string()),
            album: Some(album.to_string()),
            ..Default::default()
        };
        db.upsert_tracks_batch(
            &[
                mk("/m/a.flac", "夜空中最亮的星", "逃跑计划", "世界"),
                mk("/m/b.flac", "晴天", "周杰伦", "叶惠美"),
                mk("/m/live/c.mp3", "晴天(Live)", "周杰伦", "演唱会"),
            ],
            1,
        )
        .unwrap();

        assert_eq!(db.count_tracks_filtered(None).unwrap(), 3);

        // 标题
        assert_eq!(
            db.list_tracks_with(TrackSort::Default, 10, 0, Some("晴天"))
                .unwrap()
                .len(),
            2
        );
        // 艺术家
        let by_artist = db
            .list_tracks_with(TrackSort::Default, 10, 0, Some("逃跑计划"))
            .unwrap();
        assert_eq!(by_artist.len(), 1);
        assert_eq!(by_artist[0].title.as_deref(), Some("夜空中最亮的星"));
        // 专辑
        assert_eq!(
            db.list_tracks_with(TrackSort::Default, 10, 0, Some("叶惠美"))
                .unwrap()
                .len(),
            1
        );
        // 路径（目录名）
        assert_eq!(
            db.list_tracks_with(TrackSort::Default, 10, 0, Some("/m/live/"))
                .unwrap()
                .len(),
            1
        );

        // 计数与结果集必须一致——虚拟化列表的行数依赖它
        assert_eq!(db.count_tracks_filtered(Some("晴天")).unwrap(), 2);
        assert_eq!(db.count_tracks_filtered(Some("不存在")).unwrap(), 0);

        // `%` 是字面量而非通配符（`escape_like` 保证）
        assert_eq!(
            db.list_tracks_with(TrackSort::Default, 10, 0, Some("%"))
                .unwrap()
                .len(),
            0
        );
        // 空/空白 = 不过滤（与 None 等价）
        assert_eq!(
            db.list_tracks_with(TrackSort::Default, 10, 0, Some("  "))
                .unwrap()
                .len(),
            3
        );
        // 过滤 + 排序 + 分页 可组合
        let paged = db
            .list_tracks_with(TrackSort::Title, 1, 1, Some("晴天"))
            .unwrap();
        assert_eq!(paged.len(), 1, "第二页仍有一条");
    }

    /// 回归：`PlayedAt`/`LikedAt` 的 ORDER BY 引用 `h.`/`lk.` 别名，而
    /// `list_tracks_with` 的 TRACK_SELECT 并未 JOIN 这两张表 → 不回退就会
    /// prepare 失败（no such column），整条查询硬错。
    #[test]
    fn list_tracks_with_played_at_and_liked_at_do_not_break_query() {
        let (db, _pid, _a, _b, _c) = seed_clean();
        for s in [
            TrackSort::PlayedAt,
            TrackSort::LikedAt,
            TrackSort::PlayCount,
            TrackSort::Title,
            TrackSort::Default,
        ] {
            let rows = db.list_tracks_with(s, 10, 0, None).unwrap();
            assert_eq!(rows.len(), 3, "排序 {s:?} 不应让查询失败");
        }
    }

    /// 干净歌单：预览与执行均为零影响（幂等、不破坏顺序）。
    #[test]
    fn playlist_smart_cleanup_noop_on_clean_playlist() {
        let (db, pid, _a, _b, _c) = seed_clean();
        assert_eq!(db.playlist_cleanup_preview(pid).unwrap(), (0, 0));
        assert_eq!(db.playlist_smart_cleanup(pid).unwrap(), (0, 0));
        let items = db.playlist_tracks(pid).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].id, _a);
        assert_eq!(items[1].id, _b);
        assert_eq!(items[2].id, _c);
    }

    // ---- db.rs 审计修复回归（占位符上限 + 计数口径）----

    /// 造 `n` 首曲目并返回其 id 列表（分批插入，避开 upsert 内部潜在大语句）。
    fn seed_n_tracks(n: usize) -> (Db, Vec<i64>) {
        let db = Db::open_in_memory().unwrap();
        let sid = db.upsert_source("/m", None).unwrap();
        for batch in (0..n).collect::<Vec<_>>().chunks(200) {
            let inputs: Vec<TrackInput> = batch
                .iter()
                .map(|i| TrackInput {
                    source_id: sid,
                    path: format!("/m/track_{i}.flac"),
                    size: 1024,
                    title: Some(format!("t{i}")),
                    artist: Some("A".to_string()),
                    ..Default::default()
                })
                .collect();
            db.upsert_tracks_batch(&inputs, 1).unwrap();
        }
        let ids = {
            let mut stmt = db
                .conn
                .prepare("SELECT id FROM tracks ORDER BY id")
                .unwrap();
            stmt.query_map([], |r| r.get::<_, i64>(0))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        (db, ids)
    }

    /// 审计修复：用户批量勾选上千首时 `IN (?,..)` 占位符数 = id 数会撞 SQLite
    /// 变量上限（999）。`remove_tracks` 必须按 [`MAX_SQL_VARS`] 拆批，绝不
    /// `too many SQL variables`。
    #[test]
    fn remove_tracks_with_more_than_999_ids_succeeds() {
        let (db, ids) = seed_n_tracks(1200);
        assert_eq!(ids.len(), 1200);
        let n = db.remove_tracks(&ids, false).unwrap();
        assert_eq!(n, 1200, "拆批后整批删除必须完整生效");
        assert_eq!(db.count_tracks_filtered(None).unwrap(), 0);
    }

    /// 同上，`unlike_tracks` 也必须拆批。
    #[test]
    fn unlike_tracks_with_more_than_999_ids_succeeds() {
        let (db, ids) = seed_n_tracks(1200);
        for &id in &ids {
            db.set_like(id, true).unwrap();
        }
        assert_eq!(db.liked_count().unwrap(), 1200);
        let n = db.unlike_tracks(&ids).unwrap();
        assert_eq!(n, 1200, "拆批后整批取消喜欢必须完整生效");
        assert_eq!(db.liked_count().unwrap(), 0);
    }

    /// 审计修复：孤儿喜欢（曲目已删 / FK 关闭残留）不得计入 `liked_count`——
    /// 必须与 `list_liked_with`（INNER JOIN tracks）口径自洽。
    #[test]
    fn liked_count_excludes_orphan_rows() {
        let db = Db::open_in_memory().unwrap();
        let sid = db.upsert_source("/m", None).unwrap();
        db.upsert_tracks_batch(
            &[TrackInput {
                source_id: sid,
                path: "/m/x.flac".into(),
                size: 1024,
                title: Some("x".into()),
                artist: Some("A".into()),
                ..Default::default()
            }],
            1,
        )
        .unwrap();
        let tid = db.list_tracks(10, 0).unwrap()[0].id;
        db.set_like(tid, true).unwrap();
        // 注入孤儿喜欢（track_id 不存在）：活 FK 会拦截，临时关闭以构造孤儿场景
        db.conn.execute("PRAGMA foreign_keys = OFF", []).unwrap();
        db.conn
            .execute(
                "INSERT INTO likes(track_id, created_at) VALUES (?1, 0)",
                rusqlite::params![999_999],
            )
            .unwrap();
        db.conn.execute("PRAGMA foreign_keys = ON", []).unwrap();

        assert_eq!(db.liked_count().unwrap(), 1, "孤儿喜欢不得计入总数");
        db.set_like(tid, false).unwrap();
        assert_eq!(
            db.liked_count().unwrap(),
            0,
            "真实喜欢取消后归零；孤儿仍不计入"
        );
    }

    /// 审计修复：孤儿播放记录不得计入 `history_totals`（与可导航历史视图口径自洽）。
    #[test]
    fn history_totals_excludes_orphan_rows() {
        let db = Db::open_in_memory().unwrap();
        let sid = db.upsert_source("/m", None).unwrap();
        db.upsert_tracks_batch(
            &[TrackInput {
                source_id: sid,
                path: "/m/x.flac".into(),
                size: 1024,
                title: Some("x".into()),
                artist: Some("A".into()),
                ..Default::default()
            }],
            1,
        )
        .unwrap();
        let tid = db.list_tracks(10, 0).unwrap()[0].id;
        db.record_play(tid, 1000, 100).unwrap();
        // 注入孤儿播放记录（track_id 不存在）
        db.conn.execute("PRAGMA foreign_keys = OFF", []).unwrap();
        db.conn
            .execute(
                "INSERT INTO play_history(track_id, played_at) VALUES (?1, 0)",
                rusqlite::params![999_999],
            )
            .unwrap();
        db.conn.execute("PRAGMA foreign_keys = ON", []).unwrap();

        let (plays, played_tracks) = db.history_totals().unwrap();
        assert_eq!(plays, 1, "孤儿播放不计入总次数");
        assert_eq!(played_tracks, 1, "孤儿播放不计入去重曲目数");
    }

    /// 保留策略（retain=true）：`remove_tracks` 物理删除曲目行，但**保留** likes /
    /// play_history 行为行；`liked_count`/`history_totals` 经 `EXISTS` 过滤孤儿，
    /// 计数口径仍与可导航视图一致。
    #[test]
    fn retain_likes_history_keeps_behavior_rows() {
        let db = Db::open_in_memory().unwrap();
        let sid = db.upsert_source("/m", None).unwrap();
        db.upsert_tracks_batch(
            &[TrackInput {
                source_id: sid,
                path: "/m/x.flac".into(),
                size: 1024,
                title: Some("x".into()),
                artist: Some("A".into()),
                ..Default::default()
            }],
            1,
        )
        .unwrap();
        let tid = db.list_tracks(10, 0).unwrap()[0].id;
        db.set_like(tid, true).unwrap();
        db.record_play(tid, 1000, 100).unwrap();

        let n = db.remove_tracks(&[tid], true).unwrap();
        assert_eq!(n, 1, "曲目行仍被删");
        // 计数视图过滤孤儿 → 0（与可导航视图一致）
        assert_eq!(db.liked_count().unwrap(), 0);
        assert_eq!(db.history_totals().unwrap(), (0, 0));
        // 但原始行为行物理保留（保留策略生效）
        let raw_likes: i64 = db
            .conn
            .query_row("SELECT COUNT(1) FROM likes", [], |r| r.get(0))
            .unwrap();
        let raw_hist: i64 = db
            .conn
            .query_row("SELECT COUNT(1) FROM play_history", [], |r| r.get(0))
            .unwrap();
        assert_eq!(raw_likes, 1, "retain 下 likes 行物理保留");
        assert_eq!(raw_hist, 1, "retain 下 play_history 行物理保留");
    }

    /// 保留策略（retain=true）：`remove_source` 同理保留 likes / play_history。
    #[test]
    fn retain_on_remove_source_keeps_behavior_rows() {
        let db = Db::open_in_memory().unwrap();
        let sid = db.upsert_source("/m", None).unwrap();
        db.upsert_tracks_batch(
            &[TrackInput {
                source_id: sid,
                path: "/m/x.flac".into(),
                size: 1024,
                title: Some("x".into()),
                artist: Some("A".into()),
                ..Default::default()
            }],
            1,
        )
        .unwrap();
        let tid = db.list_tracks(10, 0).unwrap()[0].id;
        db.set_like(tid, true).unwrap();
        db.record_play(tid, 1000, 100).unwrap();

        db.remove_source(sid, true).unwrap();
        let raw_likes: i64 = db
            .conn
            .query_row("SELECT COUNT(1) FROM likes", [], |r| r.get(0))
            .unwrap();
        let raw_hist: i64 = db
            .conn
            .query_row("SELECT COUNT(1) FROM play_history", [], |r| r.get(0))
            .unwrap();
        assert_eq!(raw_likes, 1, "retain 下 remove_source 保留 likes");
        assert_eq!(raw_hist, 1, "retain 下 remove_source 保留 play_history");
        // playlist_items 与曲目/音源仍被清
        let raw_tracks: i64 = db
            .conn
            .query_row("SELECT COUNT(1) FROM tracks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(raw_tracks, 0);
    }

    /// B2 回归：remove_tracks(retain=true) 操作后必须恢复 `foreign_keys = ON`。
    /// 此前 `let _ = PRAGMA foreign_keys = ON` 吞掉恢复失败，且 SQLite 在活动事务内
    /// 该 PRAGMA 是静默 no-op，连接可能永久停在 FK=OFF（后续删除不级联 → 孤儿行）。
    #[test]
    fn remove_tracks_retain_restores_foreign_keys() {
        let db = Db::open_in_memory().unwrap();
        let sid = db.upsert_source("/m", None).unwrap();
        db.upsert_tracks_batch(
            &[TrackInput {
                source_id: sid,
                path: "/m/a.wav".into(),
                size: 1024,
                title: Some("A".into()),
                artist: Some("X".into()),
                ..Default::default()
            }],
            1,
        )
        .unwrap();
        let tid = db.list_tracks(10, 0).unwrap()[0].id;
        db.set_like(tid, true).unwrap();

        let removed = db.remove_tracks(&[tid], true).unwrap();
        assert_eq!(removed, 1);

        let fk: i64 = db
            .conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1, "B2：操作后 foreign_keys 必须恢复为 ON");
        // retain 语义：孤儿 like 应保留
        assert!(db.is_liked(tid).unwrap());
    }

    /// B1 回归守护：锁中毒时 `into_inner` 必须取回已收集的数据，而非丢弃为默认空。
    /// 等价于 library.rs `parallel_parse` 修复（unwrap_or_default →
    /// unwrap_or_else(|p| p.into_inner())）：中毒丢失已解析数据会因 indexed=0 而
    /// 叠加 A1 的空 run 守卫触发整源清空。钉住此行为防止回退。
    #[test]
    fn mutex_poison_preserves_collected_data() {
        use std::sync::{Arc, Mutex};
        use std::thread;
        let m = Arc::new(Mutex::new(vec![1i64, 2, 3]));
        let m2 = m.clone();
        let h = thread::spawn(move || {
            let _g = m2.lock().unwrap();
            panic!("worker boom");
        });
        let _ = h.join();
        // 等价于修复后的 parallel_parse 末行：中毒 Mutex 取回内部数据
        let mutex = Arc::into_inner(m).expect("arc 强引用应为 1");
        let recovered = mutex.into_inner().unwrap_or_else(|p| p.into_inner());
        assert_eq!(recovered, vec![1, 2, 3]);
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
