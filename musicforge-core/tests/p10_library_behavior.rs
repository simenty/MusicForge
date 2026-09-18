//! P2 播放行为层（db v3）测试。
//!
//! 覆盖：v2→v3 迁移（数据保留）/ likes 生命周期与幂等 /
//! 播放历史追加-倒序-清空 / 曲目移除后历史行自动隐藏。

use musicforge_core::db::{Db, TrackInput};

fn track(path: &str, title: &str) -> TrackInput {
    TrackInput {
        source_id: 1,
        path: path.to_string(),
        size: 1024,
        title: Some(title.to_string()),
        artist: Some("A".to_string()),
        ..Default::default()
    }
}

/// 模拟 v2 状态的库（无 v3 表）升级：曲目数据保留 + 行为表可用。
#[test]
fn migrate_v2_to_v3_preserves_tracks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");

    // 先让当前版本建好 v3 库并写入曲目
    let track_id;
    {
        let db = Db::open(&path).unwrap();
        let sid = db.upsert_source("/m", None).unwrap();
        let mut t = track("/m/a.flac", "a");
        t.source_id = sid;
        db.upsert_tracks_batch(&[t], 1).unwrap();
        track_id = db.list_tracks(1, 0).unwrap()[0].id;
    }
    // 手动降级为 v2 状态（删 v3 表 + 版本回退）——模拟旧程序写过的库
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "DROP TABLE IF EXISTS playlist_items;
             DROP TABLE IF EXISTS playlists;
             DROP TABLE IF EXISTS play_history;
             DROP TABLE IF EXISTS likes;
             PRAGMA user_version = 2;",
        )
        .unwrap();
    }

    // 重新打开：应完成 v2→v3 迁移且曲目保留
    let db = Db::open(&path).unwrap();
    assert_eq!(db.count_tracks().unwrap(), 1, "曲目数据必须保留");
    assert_eq!(db.list_tracks(1, 0).unwrap()[0].id, track_id);
    // v3 表可用
    assert!(db.toggle_like(track_id).unwrap());
    assert_eq!(db.liked_count().unwrap(), 1);
}

/// likes：切换 / 幂等 / 列表 / 计数。
#[test]
fn likes_lifecycle_and_idempotency() {
    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source("/m", None).unwrap();
    let mut t = track("/m/a.flac", "a");
    t.source_id = sid;
    db.upsert_tracks_batch(&[t], 1).unwrap();
    let id = db.list_tracks(1, 0).unwrap()[0].id;

    assert!(!db.is_liked(id).unwrap());
    assert!(db.toggle_like(id).unwrap(), "首次切换 → 已喜欢");
    assert!(db.is_liked(id).unwrap());
    assert_eq!(db.liked_count().unwrap(), 1);
    assert!(!db.toggle_like(id).unwrap(), "再次切换 → 取消喜欢");
    assert_eq!(db.liked_count().unwrap(), 0);

    // set_like 幂等：重复设置同状态不产生重复行
    db.set_like(id, true).unwrap();
    db.set_like(id, true).unwrap();
    assert_eq!(db.liked_count().unwrap(), 1);
    assert_eq!(db.list_liked(10, 0).unwrap().len(), 1);
    assert_eq!(db.list_liked(10, 0).unwrap()[0].id, id);
}

/// 历史：追加（不去重）/ 倒序 / 清空。
#[test]
fn history_append_desc_and_clear() {
    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source("/m", None).unwrap();
    let mut a = track("/m/a.flac", "a");
    a.source_id = sid;
    let mut b = track("/m/b.flac", "b");
    b.source_id = sid;
    db.upsert_tracks_batch(&[a, b], 1).unwrap();
    let tracks = db.list_tracks(10, 0).unwrap();
    let (id_a, id_b) = (tracks[0].id, tracks[1].id);

    db.record_play(id_a, 100, 30_000).unwrap();
    db.record_play(id_b, 200, 0).unwrap();
    db.record_play(id_a, 300, 5_000).unwrap(); // 同一曲目再次播放 → 新行

    let h = db.list_history(10).unwrap();
    assert_eq!(h.len(), 3, "追加日志不去重");
    assert_eq!(h[0].played_at, 300, "按时间倒序");
    assert_eq!(h[0].track.id, id_a);
    assert_eq!(h[0].ms_played, 5_000);
    assert_eq!(h[2].played_at, 100);

    assert_eq!(db.clear_history().unwrap(), 3);
    assert!(db.list_history(10).unwrap().is_empty());
}

/// 曲目被移除（源删除）后：历史行被**连带清理**（FK 强制的一致语义——
/// 否则 DELETE tracks 会因 play_history 的引用而失败）。
#[test]
fn history_cleaned_when_tracks_removed() {
    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source("/m", None).unwrap();
    let mut t = track("/m/a.flac", "a");
    t.source_id = sid;
    db.upsert_tracks_batch(&[t], 1).unwrap();
    let id = db.list_tracks(1, 0).unwrap()[0].id;
    db.record_play(id, 100, 0).unwrap();
    assert_eq!(db.list_history(10).unwrap().len(), 1);

    db.remove_source(sid).unwrap();
    assert!(
        db.list_history(10).unwrap().is_empty(),
        "曲目移除后历史不可见（INNER JOIN 隐藏，而非显示无数据行）"
    );
}
