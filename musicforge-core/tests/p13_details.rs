//! 详情页查询（P6.10）：按艺术家 / 专辑取曲目及排序。

use musicforge_core::db::{Db, TrackInput};

fn track(path: &str, title: &str, artist: &str, album: &str, track_no: i64) -> TrackInput {
    TrackInput {
        source_id: 1,
        path: path.to_string(),
        size: 1024,
        title: Some(title.to_string()),
        artist: Some(artist.to_string()),
        album: Some(album.to_string()),
        track_no: Some(track_no),
        ..Default::default()
    }
}

/// 艺术家/专辑详情：过滤正确 + 排序（专辑名 → 轨号）。
#[test]
fn detail_queries_filter_and_sort() {
    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source("/m", None).unwrap();
    let mut a = track("/m/1.flac", "甲", "Beyond", "乐与怒", 2);
    a.source_id = sid;
    let mut b = track("/m/2.flac", "乙", "Beyond", "乐与怒", 1);
    b.source_id = sid;
    let mut c = track("/m/3.flac", "丙", "Beyond", "命运派对", 1);
    c.source_id = sid;
    let mut d = track("/m/4.flac", "丁", "周杰伦", "叶惠美", 1);
    d.source_id = sid;
    db.upsert_tracks_batch(&[a, b, c, d], 1).unwrap();

    // 艺术家 id：取曲目数最多者不可靠（都 3/1）——按名字找
    let artists = db.list_artists().unwrap();
    let by = artists.iter().find(|x| x.name == "Beyond").unwrap().id;
    let jay = artists.iter().find(|x| x.name == "周杰伦").unwrap().id;

    let bd = db.tracks_by_artist(by).unwrap();
    assert_eq!(bd.len(), 3, "只含该艺术家的曲目");
    assert!(
        bd.iter().all(|t| t.artist.as_deref() == Some("Beyond")),
        "过滤正确"
    );
    // 排序：乐与怒（轨 1 乙、轨 2 甲）→ 命运派对（轨 1 丙）
    assert_eq!(bd[0].title.as_deref(), Some("乙"));
    assert_eq!(bd[1].title.as_deref(), Some("甲"));
    assert_eq!(bd[2].title.as_deref(), Some("丙"));

    let jd = db.tracks_by_artist(jay).unwrap();
    assert_eq!(jd.len(), 1);
    assert_eq!(jd[0].title.as_deref(), Some("丁"));

    // 专辑详情：乐与怒 2 首，按轨号
    let albums = db.list_albums().unwrap();
    let lyr = albums.iter().find(|x| x.title == "乐与怒").unwrap();
    let lt = db.tracks_by_album(lyr.id).unwrap();
    assert_eq!(lt.len(), 2);
    assert_eq!(lt[0].title.as_deref(), Some("乙"), "轨号 1 在前");
    assert_eq!(lt[1].title.as_deref(), Some("甲"));

    // 不存在的 id → 空（不报错）
    assert!(db.tracks_by_artist(99_999).unwrap().is_empty());
    assert!(db.tracks_by_album(99_999).unwrap().is_empty());
}
