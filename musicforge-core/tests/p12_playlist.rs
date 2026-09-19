//! 歌单（P6.4）：CRUD、去重、顺序维护与源删除联动。

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

/// 创建/列表/追加去重/顺序/移除前移/重命名/删除 全链路。
#[test]
fn playlist_crud_and_order() {
    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source("/m", None).unwrap();
    let mut a = track("/m/a.flac", "a");
    a.source_id = sid;
    let mut b = track("/m/b.flac", "b");
    b.source_id = sid;
    let mut c = track("/m/c.flac", "c");
    c.source_id = sid;
    db.upsert_tracks_batch(&[a, b, c], 1).unwrap();
    let tracks = db.list_tracks(10, 0).unwrap();
    let (id_a, id_b, id_c) = (tracks[0].id, tracks[1].id, tracks[2].id);

    // 创建（trim）+ 空名拒绝
    let pid = db.create_playlist("  夜跑  ").unwrap();
    assert!(db.create_playlist("   ").is_err(), "空名拒绝");
    let list = db.list_playlists().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "夜跑");
    assert_eq!(list[0].track_count, 0);

    // 追加：重复与不存在的 id 均跳过
    assert_eq!(
        db.playlist_add_tracks(pid, &[id_a, id_b, id_a, 999_999]).unwrap(),
        2
    );
    assert_eq!(db.playlist_add_tracks(pid, &[id_c]).unwrap(), 1);
    let items = db.playlist_tracks(pid).unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].id, id_a);
    assert_eq!(items[1].id, id_b);
    assert_eq!(items[2].id, id_c);

    // 移除中间项 → 后续前移
    db.playlist_remove_track(pid, id_b).unwrap();
    let items = db.playlist_tracks(pid).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, id_a);
    assert_eq!(items[1].id, id_c, "移除后顺序前移");

    // 移除后再加 → 追加尾部（position 续接）
    assert_eq!(db.playlist_add_tracks(pid, &[id_b]).unwrap(), 1);
    let items = db.playlist_tracks(pid).unwrap();
    assert_eq!(items[2].id, id_b);

    // 重命名 + 计数
    db.playlist_rename(pid, "晨跑").unwrap();
    assert_eq!(db.list_playlists().unwrap()[0].name, "晨跑");
    assert_eq!(db.list_playlists().unwrap()[0].track_count, 3);

    // 不存在的歌单 → 报错
    assert!(db.playlist_add_tracks(999, &[id_a]).is_err());

    // 删除歌单：条目清空，曲目行不受影响
    db.playlist_delete(pid).unwrap();
    assert!(db.list_playlists().unwrap().is_empty());
    assert!(db.playlist_tracks(pid).unwrap().is_empty());
    assert_eq!(db.list_tracks(10, 0).unwrap().len(), 3, "删歌单不动曲目");
}

/// 源删除（remove_source）连带清理歌单条目——P2 已有的级联语义回归。
#[test]
fn source_removal_cleans_playlist_items() {
    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source("/m", None).unwrap();
    let mut a = track("/m/a.flac", "a");
    a.source_id = sid;
    db.upsert_tracks_batch(&[a], 1).unwrap();
    let id_a = db.list_tracks(10, 0).unwrap()[0].id;
    let pid = db.create_playlist("测试").unwrap();
    db.playlist_add_tracks(pid, &[id_a]).unwrap();
    assert_eq!(db.playlist_tracks(pid).unwrap().len(), 1);

    db.remove_source(sid).unwrap();
    assert!(
        db.playlist_tracks(pid).unwrap().is_empty(),
        "源删除后歌单条目自动消失（不显示无数据行）"
    );
    // 歌单本身保留（空歌单仍是用户资产）
    assert_eq!(db.list_playlists().unwrap().len(), 1);
    assert_eq!(db.list_playlists().unwrap()[0].track_count, 0);
}
