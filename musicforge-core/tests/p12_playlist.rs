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

/// 拖拽排序：上移/下移/越界 clamp/不在歌单空操作。
#[test]
fn playlist_move_track_reorders() {
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
    let pid = db.create_playlist("排序").unwrap();
    db.playlist_add_tracks(pid, &[id_a, id_b, id_c]).unwrap();

    // 末位 c 拖到首位（to=0）
    db.playlist_move_track(pid, id_c, 0).unwrap();
    let items = db.playlist_tracks(pid).unwrap();
    assert_eq!(
        items.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![id_c, id_a, id_b]
    );

    // 首位 c 拖到中位（to=1）
    db.playlist_move_track(pid, id_c, 1).unwrap();
    let items = db.playlist_tracks(pid).unwrap();
    assert_eq!(
        items.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![id_a, id_c, id_b]
    );

    // 越界 clamp：to=99 → 末位
    db.playlist_move_track(pid, id_a, 99).unwrap();
    let items = db.playlist_tracks(pid).unwrap();
    assert_eq!(
        items.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![id_c, id_b, id_a]
    );

    // 不在歌单的曲目 → 空操作（不报错）
    db.playlist_move_track(pid, 999_999, 0).unwrap();
    assert_eq!(db.playlist_tracks(pid).unwrap().len(), 3);

    // position 仍连续（移除后不变空洞）
    db.playlist_remove_track(pid, id_b).unwrap();
    db.playlist_add_tracks(pid, &[id_b]).unwrap();
    let items = db.playlist_tracks(pid).unwrap();
    assert_eq!(
        items.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![id_c, id_a, id_b]
    );
}

/// 单歌单导出 M3U8：格式（#EXTM3U/#EXTINF/相对路径）与读回。
#[test]
fn export_one_m3u8_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("夜跑.m3u8");
    let entries = vec![
        (
            dir.path().join("a.flac"),
            "第一首".to_string(),
            251i64,
        ),
        (std::path::PathBuf::from("C:\\elsewhere\\b.flac"), "B".to_string(), -1),
    ];
    let n = musicforge_core::playlist::export_one_m3u8(&dst, &entries).unwrap();
    assert_eq!(n, 2);
    let body = std::fs::read_to_string(&dst).unwrap();
    assert!(body.starts_with("#EXTM3U\n"));
    assert!(body.contains("#EXTINF:251,第一首\n"));
    assert!(body.contains("a.flac\n"), "同目录内应为相对路径");
    assert!(body.contains("#EXTINF:-1,B\n"));
    assert!(body.contains("C:/elsewhere/b.flac"), "外部路径保持绝对（正斜杠）");
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

    db.remove_source(sid, false).unwrap();
    assert!(
        db.playlist_tracks(pid).unwrap().is_empty(),
        "源删除后歌单条目自动消失（不显示无数据行）"
    );
    // 歌单本身保留（空歌单仍是用户资产）
    assert_eq!(db.list_playlists().unwrap().len(), 1);
    assert_eq!(db.list_playlists().unwrap()[0].track_count, 0);
}

/// 封面拼贴（P6.12）：按曲序、路径去重、每单截断、无封面剔除、歌单间隔离。
#[test]
fn playlist_covers_ordered_deduped_and_capped() {
    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source("/m", None).unwrap();
    // a1/a2 同专辑 A（同封面）+ B/C/D/E 各一专辑 —— 5 张不同封面候选
    let mk = |path: &str, album: &str| {
        let mut x = track(path, path);
        x.source_id = sid;
        x.album = Some(album.to_string());
        x
    };
    db.upsert_tracks_batch(
        &[
            mk("/m/a1.flac", "A"),
            mk("/m/a2.flac", "A"),
            mk("/m/b.flac", "B"),
            mk("/m/c.flac", "C"),
            mk("/m/d.flac", "D"),
            mk("/m/e.flac", "E"),
        ],
        1,
    )
    .unwrap();
    let tracks = db.list_tracks(20, 0).unwrap();
    let find = |p: &str| tracks.iter().find(|t| t.path == p).unwrap().id;

    let by_title: std::collections::HashMap<String, i64> = db
        .list_albums()
        .unwrap()
        .into_iter()
        .map(|a| (a.title, a.id))
        .collect();
    for (title, n) in [("A", 1), ("B", 2), ("C", 3), ("D", 4), ("E", 5)] {
        db.set_album_cover(by_title[title], &format!("C:\\cov\\{n}.jpg"))
            .unwrap();
    }

    let pid = db.create_playlist("拼贴").unwrap();
    db.playlist_add_tracks(
        pid,
        &[
            find("/m/a1.flac"),
            find("/m/a2.flac"), // 与 a1 同专辑 → 封面重复，应被去重
            find("/m/b.flac"),
            find("/m/c.flac"),
            find("/m/d.flac"),
            find("/m/e.flac"),
        ],
    )
    .unwrap();

    // per=4：A→B→C→D（去重 a2、截断 E）
    let covs = db.playlist_covers(4).unwrap();
    assert!(covs.iter().all(|(id, _)| *id == pid));
    assert_eq!(
        covs.iter().map(|(_, p)| p.as_str()).collect::<Vec<_>>(),
        vec![
            "C:\\cov\\1.jpg",
            "C:\\cov\\2.jpg",
            "C:\\cov\\3.jpg",
            "C:\\cov\\4.jpg"
        ],
        "按曲序去重 + 每单截断"
    );

    // per 放宽 → 5 张全出（含 E）
    assert_eq!(db.playlist_covers(8).unwrap().len(), 5);

    // 歌单间隔离：新歌单只含自己的封面
    let pid2 = db.create_playlist("单曲").unwrap();
    db.playlist_add_tracks(pid2, &[find("/m/e.flac")]).unwrap();
    let covs2 = db.playlist_covers(4).unwrap();
    assert_eq!(
        covs2
            .iter()
            .filter(|(id, _)| *id == pid2)
            .map(|(_, p)| p.as_str())
            .collect::<Vec<_>>(),
        vec!["C:\\cov\\5.jpg"]
    );

    // 空歌单不出现
    let pid3 = db.create_playlist("空").unwrap();
    assert!(db
        .playlist_covers(8)
        .unwrap()
        .iter()
        .all(|(id, _)| *id != pid3));
}
