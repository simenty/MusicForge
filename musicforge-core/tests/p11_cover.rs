//! 封面元数据（core 侧）：专辑封面字段的读写与缺失谓词。
//!
//! core 只存**路径**不做 IO——抓取/落盘在 shell 层
//! （见 `musicforge-gui/src-tauri/src/commands/covers.rs`）。

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

/// 封面字段：写入 → 读取 → 缺失谓词（core 只存路径，不做 IO）。
#[test]
fn album_cover_roundtrip() {
    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source("/m", None).unwrap();
    let mut a = track("/m/a.flac", "a");
    a.source_id = sid;
    a.album = Some("乐与怒".to_string());
    a.artist = Some("Beyond".to_string());
    db.upsert_tracks_batch(&[a], 1).unwrap();

    let albums = db.list_albums().unwrap();
    assert_eq!(albums.len(), 1);
    let id = albums[0].id;
    assert!(albums[0].cover_path.is_none(), "初始无封面");

    // 缺失谓词包含它（id、标题、专辑艺人名）
    let miss = db.albums_missing_cover(10).unwrap();
    assert_eq!(miss.len(), 1);
    assert_eq!(miss[0].0, id);
    assert_eq!(miss[0].1, "乐与怒");
    assert_eq!(miss[0].2.as_deref(), Some("Beyond"));

    db.set_album_cover(id, "C:\\data\\covers\\1.jpg").unwrap();
    let albums = db.list_albums().unwrap();
    assert_eq!(
        albums[0].cover_path.as_deref(),
        Some("C:\\data\\covers\\1.jpg")
    );
    assert!(
        db.albums_missing_cover(10).unwrap().is_empty(),
        "已补封面的专辑不再出现在缺失列表"
    );
}

/// 年份回填守卫 + 曲目封面查找 + 艺术家最热专辑。
#[test]
fn meta_predicates_roundtrip() {
    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source("/m", None).unwrap();
    let mut a = track("/m/a.flac", "a");
    a.source_id = sid;
    a.album = Some("乐与怒".to_string());
    a.artist = Some("Beyond".to_string());
    a.year = None;
    db.upsert_tracks_batch(&[a], 1).unwrap();

    let album = db.list_albums().unwrap().remove(0);
    let album_id = album.id;
    let tracks = db.list_tracks(10, 0).unwrap();
    assert_eq!(tracks.len(), 1);

    // 年份回填：空 → 写入
    db.set_album_year(album_id, 1993).unwrap();
    assert_eq!(db.list_albums().unwrap()[0].year, Some(1993));
    // 已有年份 → 守卫拒绝覆盖
    db.set_album_year(album_id, 2020).unwrap();
    assert_eq!(db.list_albums().unwrap()[0].year, Some(1993), "不覆盖已有年份");

    // 曲目封面查找：无封面 → None；补上 → Some
    let track_id = tracks[0].id;
    assert_eq!(
        db.get_track(track_id).unwrap().unwrap().title.as_deref(),
        Some("a")
    );
    assert!(db.track_cover_path(track_id).unwrap().is_none());
    db.set_album_cover(album_id, "C:\\c\\1.jpg").unwrap();
    assert_eq!(
        db.track_cover_path(track_id).unwrap().as_deref(),
        Some("C:\\c\\1.jpg")
    );

    // 艺术家最热专辑：曲目数最多的那（此处唯一）
    let artists = db.list_artists().unwrap();
    let aid = artists
        .iter()
        .find(|x| x.track_count > 0)
        .map(|x| x.id)
        .unwrap();
    let top = db.artist_top_album(aid).unwrap();
    assert_eq!(top.unwrap().0, album_id);
}
