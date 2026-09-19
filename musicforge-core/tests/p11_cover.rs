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
