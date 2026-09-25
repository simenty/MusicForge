//! P1 曲库维度层（db v2）测试。
//!
//! 覆盖：v1→v2 迁移（不丢数据）/ 媒体源生命周期 / 批量入库与聚合 /
//! LIKE 通配符转义 / 索引 run 后的陈旧行清理 / 专辑年份回填。

use musicforge_core::db::{Db, TrackInput};

/// 构造测试曲目（source_id 由调用方覆盖）。
fn track(path: &str, artist: Option<&str>, album: Option<&str>, title: &str) -> TrackInput {
    TrackInput {
        source_id: 1,
        path: path.to_string(),
        size: 1024,
        title: Some(title.to_string()),
        artist: artist.map(str::to_string),
        album: album.map(str::to_string),
        ..Default::default()
    }
}

/// v1 库（旧程序写过的）升级到 v2：files 数据保留 + v2 表可用。
#[test]
fn migrate_v1_db_preserves_files_and_creates_v2() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    {
        // 模拟 schema v1 的既有库（结构与该版本的 SCHEMA_SQL 一致）
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE files (
                 path TEXT PRIMARY KEY, size INTEGER NOT NULL, mtime INTEGER,
                 format TEXT, sha256 TEXT, updated_at TEXT NOT NULL);
             CREATE TABLE tasks (
                 id TEXT PRIMARY KEY, command TEXT NOT NULL, started_at TEXT NOT NULL,
                 finished_at TEXT, ok INTEGER NOT NULL DEFAULT 0, failed INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE ack (id TEXT PRIMARY KEY, at TEXT NOT NULL);
             INSERT INTO files VALUES ('/a.flac', 100, 1, 'flac', NULL, datetime('now'));
             PRAGMA user_version = 1;",
        )
        .unwrap();
    }

    let db = Db::open(&path).unwrap();

    // v1 数据保留（迁移是增量建表，不重建旧表）
    assert_eq!(db.stats().unwrap(), (1, 0));
    assert!(db.get_file("/a.flac").unwrap().is_some());

    // v2 表可用
    let sid = db.upsert_source("/music", Some("主曲库")).unwrap();
    assert!(sid > 0);
    let srcs = db.list_sources().unwrap();
    assert_eq!(srcs.len(), 1);
    assert_eq!(srcs[0].label.as_deref(), Some("主曲库"));
}

/// 媒体源：幂等登记（label 不被 None 清掉）+ 移除时连带清理曲目。
#[test]
fn source_upsert_idempotent_and_remove_cascades() {
    let db = Db::open_in_memory().unwrap();
    let id1 = db.upsert_source("/music", Some("曲库")).unwrap();
    let id2 = db.upsert_source("/music", None).unwrap();
    assert_eq!(id1, id2, "同路径重复登记必须返回同一 id");
    assert_eq!(
        db.list_sources().unwrap()[0].label.as_deref(),
        Some("曲库"),
        "label 传 None 时应保留已有值"
    );

    let mut t = track("/music/a.flac", Some("A"), None, "a");
    t.source_id = id1;
    db.upsert_tracks_batch(&[t], 1).unwrap();
    assert_eq!(db.count_tracks().unwrap(), 1);

    let removed = db.remove_source(id1, false).unwrap();
    assert_eq!(removed, 1, "remove_source 应报告连带清理的曲目数");
    assert_eq!(db.count_tracks().unwrap(), 0);
    assert!(db.list_sources().unwrap().is_empty());
}

/// 批量入库 + 聚合统计 + 分页 + limit 上限。
#[test]
fn upsert_and_aggregate() {
    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source("/music", None).unwrap();

    let mk = |p: &str, ar: &str, al: &str, t: &str, y: i64| {
        let mut x = track(p, Some(ar), Some(al), t);
        x.source_id = sid;
        x.year = Some(y);
        x.duration_ms = Some(180_000);
        x
    };
    db.upsert_tracks_batch(
        &[
            mk("/m/1.flac", "Beyond", "乐与怒", "海阔天空", 1993),
            mk("/m/2.flac", "Beyond", "乐与怒", "光辉岁月", 1993),
            mk("/m/3.flac", "陈奕迅", "黑白灰", "十年", 2003),
        ],
        1,
    )
    .unwrap();

    let st = db.library_stats().unwrap();
    assert_eq!(st.tracks, 3);
    assert_eq!(st.artists, 2);
    assert_eq!(st.albums, 2);
    assert_eq!(st.total_duration_ms, 540_000);

    let artists = db.list_artists().unwrap();
    assert_eq!(artists[0].name, "Beyond");
    assert_eq!(artists[0].track_count, 2, "按曲目数降序");

    let albums = db.list_albums().unwrap();
    assert_eq!(albums[0].title, "乐与怒");
    assert_eq!(albums[0].artist.as_deref(), Some("Beyond"));
    assert_eq!(albums[0].year, Some(1993));

    // 分页（按 path 稳定排序）
    assert_eq!(db.list_tracks(2, 0).unwrap().len(), 2);
    assert_eq!(db.list_tracks(2, 2).unwrap().len(), 1);
    // limit 硬上限 500（此处库小，验行为等价）
    assert_eq!(db.list_tracks(9999, 0).unwrap().len(), 3);
}

/// 搜索：LIKE 通配符必须按字面转义（输入 `%` 不得命中全库）。
#[test]
fn search_escapes_like_wildcards() {
    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source("/music", None).unwrap();

    let mut t1 = track("/music/100percent.flac", Some("X"), None, "100% Love");
    t1.source_id = sid;
    let mut t2 = track("/music/normal.flac", Some("Y"), None, "Normal Song");
    t2.source_id = sid;
    db.upsert_tracks_batch(&[t1, t2], 1).unwrap();

    let hits = db.search_tracks("%", 50).unwrap();
    assert_eq!(hits.len(), 1, "未转义时 `%` 会命中全部曲目");
    assert_eq!(hits[0].title.as_deref(), Some("100% Love"));

    assert_eq!(db.search_tracks("normal", 50).unwrap().len(), 1);
    assert_eq!(db.search_tracks("   ", 50).unwrap().len(), 0);
}

/// 索引 run 结束后清理陈旧行（文件已从磁盘移除）。
#[test]
fn stale_tracks_removed_after_rescan() {
    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source("/music", None).unwrap();

    let mut a = track("/music/a.flac", Some("A"), Some("Al"), "a");
    a.source_id = sid;
    let mut b = track("/music/b.flac", Some("B"), Some("Bl"), "b");
    b.source_id = sid;
    db.upsert_tracks_batch(&[a.clone(), b], 1000).unwrap();
    assert_eq!(db.count_tracks().unwrap(), 2);

    // 第二次索引：只剩 a（b 已从磁盘移走）
    db.upsert_tracks_batch(&[a], 2000).unwrap();
    let removed = db.remove_stale_tracks(sid, 2000, false).unwrap();
    assert_eq!(removed, 1);
    assert_eq!(db.count_tracks().unwrap(), 1);
    assert_eq!(db.list_tracks(10, 0).unwrap()[0].path, "/music/a.flac");
}

/// P9 审计回归：陈旧清理的保留策略。
///
/// 原实现**无条件**删除 `likes` / `play_history`，与 `retain_likes_history`
/// 默认 `true` 直接冲突——重扫索引会把用户不可再生数据「静默清理」掉。
/// 语义对齐 [`Db::remove_source`] / [`Db::remove_tracks`]：
/// `retain = true` 保留二者（FK 临时关闭以允许「留行为行 + 删曲目」），
/// 仅结构性关联 `playlist_items` 两种模式都清。
#[test]
fn remove_stale_tracks_retain_keeps_behavior_rows() {
    fn seed() -> (Db, i64, i64) {
        let db = Db::open_in_memory().unwrap();
        let sid = db.upsert_source("/music", None).unwrap();
        let mut a = track("/music/a.flac", Some("A"), Some("Al"), "a");
        a.source_id = sid;
        let mut b = track("/music/b.flac", Some("B"), Some("Bl"), "b");
        b.source_id = sid;
        db.upsert_tracks_batch(&[a.clone(), b], 1000).unwrap();
        let b_id = db
            .list_tracks(100, 0)
            .unwrap()
            .into_iter()
            .find(|t| t.path == "/music/b.flac")
            .expect("b 应已入库")
            .id;
        db.toggle_like(b_id).unwrap();
        db.record_play(b_id, 1_700_000_000, 30_000).unwrap();
        (db, sid, b_id)
    }

    // retain = true：曲目行照删，但 likes / play_history 保留
    let (db, sid, b_id) = seed();
    assert_eq!(db.all_liked_ids().unwrap(), vec![b_id]);
    let mut a = track("/music/a.flac", Some("A"), Some("Al"), "a");
    a.source_id = sid;
    db.upsert_tracks_batch(&[a], 2000).unwrap();
    let removed = db.remove_stale_tracks(sid, 2000, true).unwrap();
    assert_eq!(removed, 1, "陈旧曲目行仍要清掉（保留策略不豁免曲目本身）");
    assert_eq!(db.count_tracks().unwrap(), 1);
    assert_eq!(
        db.all_liked_ids().unwrap(),
        vec![b_id],
        "retain=true：likes 不得被删（用户不可再生数据）"
    );

    // 对照 retain = false：likes 随曲目级联清理（原语义不变）
    let (db2, sid2, _) = seed();
    let mut a2 = track("/music/a.flac", Some("A"), Some("Al"), "a");
    a2.source_id = sid2;
    db2.upsert_tracks_batch(&[a2], 2000).unwrap();
    db2.remove_stale_tracks(sid2, 2000, false).unwrap();
    assert!(
        db2.all_liked_ids().unwrap().is_empty(),
        "retain=false：likes 随曲目级联清理"
    );
}

/// P1-11：daily_play_counts 必须排除孤儿播放（曲目已删、保留历史策略下残留）。
#[test]
fn daily_play_counts_excludes_orphans_p1() {
    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source("/m", None).unwrap();
    let mut t = track("/m/a.flac", Some("A"), Some("Al"), "a");
    t.source_id = sid;
    db.upsert_tracks_batch(&[t], 1000).unwrap();
    let tid = db.list_tracks(10, 0).unwrap().remove(0).id;
    db.record_play(tid, 1_700_000_000, 10_000).unwrap();
    // 让 a.flac 变陈旧：再索引别的文件 → a.flac 被删但 history 保留（retain=true）
    let mut other = track("/m/b.flac", Some("B"), Some("Bl"), "b");
    other.source_id = sid;
    db.upsert_tracks_batch(&[other], 2000).unwrap();
    db.remove_stale_tracks(sid, 2000, true).unwrap();
    assert_eq!(db.count_tracks().unwrap(), 1);
    let days = db.daily_play_counts(0).unwrap();
    assert!(
        days.is_empty(),
        "孤儿播放不得计入每日计数（否则统计数虚高、与可导航历史脱节）"
    );
}

/// P1-11：list_playlists 的曲目数必须排除孤儿条目（曲目已删、保留策略下残留）。
#[test]
fn list_playlists_excludes_orphan_items_p1() {
    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source("/m", None).unwrap();
    let mut t = track("/m/a.flac", Some("A"), Some("Al"), "a");
    t.source_id = sid;
    db.upsert_tracks_batch(&[t], 1000).unwrap();
    let tid = db.list_tracks(10, 0).unwrap().remove(0).id;
    let pid = db.create_playlist("p1").unwrap();
    db.playlist_add_tracks(pid, &[tid]).unwrap();
    assert_eq!(db.list_playlists().unwrap()[0].track_count, 1);
    // 让 a.flac 变陈旧 → 残留孤儿 playlist_items
    let mut other = track("/m/b.flac", Some("B"), Some("Bl"), "b");
    other.source_id = sid;
    db.upsert_tracks_batch(&[other], 2000).unwrap();
    db.remove_stale_tracks(sid, 2000, true).unwrap();
    assert_eq!(db.count_tracks().unwrap(), 1);
    assert_eq!(
        db.list_playlists().unwrap()[0].track_count,
        0,
        "孤儿条目不得计入歌单曲目数"
    );
}

/// P1-12：count_history_filtered 与 list_history_with 必须同口径
/// （虚拟化历史列表行数不再错位出现越界占位行）。
#[test]
fn history_count_matches_list_p1() {
    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source("/m", None).unwrap();
    for i in 0..3 {
        let mut t = track(&format!("/m/{i}.flac"), Some("A"), Some("Al"), "a");
        t.source_id = sid;
        db.upsert_tracks_batch(&[t], 1000).unwrap();
    }
    for t in db.list_tracks(10, 0).unwrap() {
        db.record_play(t.id, 1_700_000_000 + t.id, 10_000).unwrap();
    }
    let list = db
        .list_history_with(musicforge_core::db::TrackSort::Default, 500, None)
        .unwrap();
    let n = db.count_history_filtered(None).unwrap();
    assert_eq!(
        n as usize,
        list.len(),
        "历史计数必须与列表自洽（否则虚拟化越界占位）"
    );
}

/// 专辑年份：先入库无年份、后续索引补齐（补空不覆盖已有值）。
#[test]
fn album_year_backfilled_when_missing() {
    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source("/m", None).unwrap();

    let mut t1 = track("/m/1.flac", Some("A"), Some("Al"), "one");
    t1.source_id = sid; // 无 year
    db.upsert_tracks_batch(&[t1], 1).unwrap();

    let mut t2 = track("/m/2.flac", Some("A"), Some("Al"), "two");
    t2.source_id = sid;
    t2.year = Some(2001);
    db.upsert_tracks_batch(&[t2], 2).unwrap();

    let albums = db.list_albums().unwrap();
    assert_eq!(albums.len(), 1, "同艺术家同名专辑不得重复建行");
    assert_eq!(albums[0].year, Some(2001));
}

/// 批量移除曲目：FK 是活的，须连带清理行为行（否则 FOREIGN KEY constraint failed）。
#[test]
fn remove_tracks_cascades_behavior_rows() {
    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source("/music", None).unwrap();
    let mk = |p: &str, t: &str| {
        let mut x = track(p, Some("A"), Some("Al"), t);
        x.source_id = sid;
        x
    };
    db.upsert_tracks_batch(&[mk("/m/1.flac", "one"), mk("/m/2.flac", "two")], 1)
        .unwrap();
    let tracks = db.list_tracks(10, 0).unwrap();
    let id1 = tracks[0].id;
    // 制造行为行（FK 依赖），验证级联清理不报约束错误
    db.record_play(id1, 1_700_000_000_000, 120_000).unwrap();
    db.toggle_like(id1).unwrap();

    let removed = db.remove_tracks(&[id1], false).unwrap();
    assert_eq!(removed, 1);
    assert_eq!(db.count_tracks().unwrap(), 1);
    assert_eq!(db.list_history(10).unwrap().len(), 0, "播放历史须级联清理");
    assert_eq!(db.liked_count().unwrap(), 0, "喜欢须级联清理");
}
