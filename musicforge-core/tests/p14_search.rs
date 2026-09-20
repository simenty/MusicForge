//! 全局搜索（P6.14）：四组命中 / 空查询 / 通配符按字面转义。

use musicforge_core::db::{Db, TrackInput};

fn track(path: &str, title: &str, artist: &str, album: &str) -> TrackInput {
    TrackInput {
        source_id: 1,
        path: path.to_string(),
        size: 1024,
        title: Some(title.to_string()),
        artist: Some(artist.to_string()),
        album: Some(album.to_string()),
        ..Default::default()
    }
}

#[test]
fn search_all_covers_four_groups() {
    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source("/m", None).unwrap();
    let mut a = track("/m/a.flac", "海阔天空", "Beyond", "乐与怒");
    a.source_id = sid;
    let mut b = track("/m/b.flac", "真的爱你", "Beyond", "乐与怒");
    b.source_id = sid;
    db.upsert_tracks_batch(&[a, b], 1).unwrap();
    db.create_playlist("夜跑").unwrap();

    // 曲目：标题命中
    let hits = db.search_all("海阔", 8).unwrap();
    assert_eq!(hits.tracks.len(), 1);
    assert_eq!(hits.tracks[0].title.as_deref(), Some("海阔天空"));

    // 艺术家：艺人名命中（该艺术家两首曲目都算曲目命中）
    let hits = db.search_all("Beyond", 8).unwrap();
    assert_eq!(hits.tracks.len(), 2, "曲目按艺术家名命中");
    assert_eq!(hits.artists.len(), 1);
    assert_eq!(hits.artists[0].name, "Beyond");
    assert_eq!(hits.artists[0].track_count, 2);

    // 专辑：专辑名命中（两首同专辑 → 1 条，计数 2）
    let hits = db.search_all("乐与怒", 8).unwrap();
    assert_eq!(hits.albums.len(), 1);
    assert_eq!(hits.albums[0].track_count, 2);
    assert_eq!(hits.albums[0].artist.as_deref(), Some("Beyond"));

    // 歌单：名称命中（空歌单也可见——它是用户资产）
    let hits = db.search_all("夜跑", 8).unwrap();
    assert_eq!(hits.playlists.len(), 1);
    assert_eq!(hits.playlists[0].name, "夜跑");
    assert_eq!(hits.playlists[0].track_count, 0);

    // 空查询 → 四组全空（面板不应为空白输入发查询）
    let empty = db.search_all("   ", 8).unwrap();
    assert!(
        empty.tracks.is_empty()
            && empty.albums.is_empty()
            && empty.artists.is_empty()
            && empty.playlists.is_empty()
    );

    // 通配符按字面转义：`%` 不应退化成"匹配全部"
    let wild = db.search_all("%", 8).unwrap();
    assert!(
        wild.tracks.is_empty(),
        "LIKE 通配符须按字面处理（escape_like）"
    );
}
