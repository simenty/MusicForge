//! P2/C3 稳定性回归：孤儿歌词/封面判定必须就地改写原条目，不得 append 新条目
//! （原方案让原 Lyrics/Cover 条目与新 Junk 条目同路径并存 → items 双计、
//! 分类计数虚高）。修复后同路径仅一条且分类计数自洽。

use musicforge_core::scan::{scan_library, Category, ScanOptions, ScanReport};
use std::fs;
use tempfile::tempdir;

#[test]
fn orphan_lyrics_and_covers_reclassified_in_place() {
    let dir = tempdir().unwrap();
    // 孤儿歌词：同目录无同名音频 → 应重分类为 Junk
    fs::write(dir.path().join("song.lrc"), b"[ti:test]").unwrap();
    // 孤儿封面：文件名 cover.jpg，所在目录无音频 → 应重分类为 Junk
    fs::write(dir.path().join("cover.jpg"), b"\xff\xd8\xff\xd9").unwrap();

    let report: ScanReport =
        scan_library(dir.path(), &ScanOptions::default()).expect("scan 应成功");

    // 同路径仅一条，且被重分类为 Junk（修复前会同时存在原 Lyrics/Cover + 新 Junk）
    let lrc_items: Vec<_> = report
        .items
        .iter()
        .filter(|i| i.path.to_string_lossy().ends_with("song.lrc"))
        .collect();
    assert_eq!(lrc_items.len(), 1, "C3：孤儿歌词同路径不得双计");
    assert_eq!(
        lrc_items[0].category,
        Category::Junk,
        "C3：孤儿歌词应重分类为 Junk"
    );

    let cover_items: Vec<_> = report
        .items
        .iter()
        .filter(|i| i.path.to_string_lossy().ends_with("cover.jpg"))
        .collect();
    assert_eq!(cover_items.len(), 1, "C3：孤儿封面同路径不得双计");
    assert_eq!(
        cover_items[0].category,
        Category::Junk,
        "C3：孤儿封面应重分类为 Junk"
    );

    // 分类计数不自洽：孤儿被移出 lyrics/covers，归入 junk
    assert_eq!(report.lyrics, 0, "C3：孤儿歌词不应计入 lyrics");
    assert_eq!(report.covers, 0, "C3：孤儿封面不应计入 covers");
    assert_eq!(report.junk, 2, "C3：两个孤儿应计入 junk");
}
