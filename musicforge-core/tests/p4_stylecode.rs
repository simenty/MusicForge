//! P4.4：风格代码解析 + genre 写入 + 相似封面 aHash 分组。
//!
//! 验收口径：
//! - `[Y23-S01-E01-C01-C02-V00]` → year/style/mood/scenes/version 结构化
//!   （蓝图 v3.0 权威语义：年份/风格/情绪/场景/版本）；
//! - genre = codebook 翻译（查不到回退原始码，绝不编造）；
//! - FillMissingOnly：已有 genre 绝不覆盖（--replace-all 需显式）；
//! - similar_cover：同内嵌封面 → 同组；不同封面 → 不同组；仅报告。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use musicforge_core::dedupe::similar_cover_scan;
use musicforge_core::stylecode::{
    apply_genre_writes, parse_style_code, plan_genre_writes, GenreDecision,
};

fn uniq_root(tag: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("mf-sc-{tag}-{n}"))
}

fn wav_bytes() -> Vec<u8> {
    let data = vec![0u8; 88200];
    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    v.extend_from_slice(b"WAVE");
    v.extend_from_slice(b"fmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&44100u32.to_le_bytes());
    v.extend_from_slice(&88200u32.to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&(data.len() as u32).to_le_bytes());
    v.extend_from_slice(&data);
    v
}

#[test]
fn parse_style_code_structured() {
    let sc = parse_style_code(Path::new("[Y23-S01-E01-C01-C02-V00] 晴天.flac")).unwrap();
    assert_eq!(sc.year, Some(2023));
    assert_eq!(sc.style.as_deref(), Some("S01"));
    assert_eq!(sc.mood.as_deref(), Some("E01"));
    assert_eq!(sc.scenes, vec!["C01", "C02"]);
    assert_eq!(sc.version.as_deref(), Some("V00"));

    let mut map = BTreeMap::new();
    map.insert("S01".to_string(), "流行".to_string());
    map.insert("C01".to_string(), "学习".to_string());
    assert_eq!(sc.genre_label(&map).as_deref(), Some("流行 / 学习 / C02"));
    let cn = sc.display_cn(&map);
    assert!(
        cn.contains("年份: 2023") && cn.contains("风格: 流行") && cn.contains("场景: 学习、C02"),
        "{cn}"
    );
    // 查不到的码回退原始码，绝不编造
    assert!(cn.contains("C02"), "{cn}");
}

#[test]
fn parse_ignores_non_leading_brackets_and_year_window() {
    assert!(parse_style_code(Path::new("song [Live] mix.flac")).is_none());
    assert!(parse_style_code(Path::new("普通歌名.flac")).is_none());
    assert_eq!(
        parse_style_code(Path::new("[Y79] a.flac")).unwrap().year,
        Some(2079)
    );
    assert_eq!(
        parse_style_code(Path::new("[Y80] a.flac")).unwrap().year,
        Some(1980)
    );
}

#[test]
fn no_label_when_only_version() {
    let sc = parse_style_code(Path::new("[V00] x.flac")).unwrap();
    assert!(sc.genre_label(&BTreeMap::new()).is_none());
}

fn map_fixture() -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert("S01".to_string(), "流行".to_string());
    m
}

#[test]
fn genre_write_fill_missing_only_roundtrip() {
    let root = uniq_root("genre");
    let lib = root.join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("[S01] t.wav"), wav_bytes()).unwrap();

    let map = map_fixture();
    // dry-run 规划
    let plan = plan_genre_writes(&lib, &map, false).unwrap();
    assert_eq!(plan.items.len(), 1);
    assert_eq!(
        plan.items[0].1,
        GenreDecision::WillWrite {
            genre: "流行".to_string()
        }
    );

    // apply → 标签真实落盘
    let (written, failed) = apply_genre_writes(&plan);
    assert_eq!((written, failed), (1, 0));

    // 回读验证（不信任写入方自证）
    use lofty::prelude::*;
    let tagged = lofty::read_from_path(lib.join("[S01] t.wav")).unwrap();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag()).unwrap();
    assert_eq!(tag.get_string(lofty::tag::ItemKey::Genre), Some("流行"));

    // 二次规划：FillMissingOnly → HasGenre（绝不覆盖）
    let plan2 = plan_genre_writes(&lib, &map, false).unwrap();
    assert_eq!(plan2.items[0].1, GenreDecision::HasGenre);
    std::fs::remove_dir_all(&root).ok();
}

/// 生成指定灰度图案的 PNG（用于内嵌封面 aHash 测试）。
fn png_bytes(pattern: u8) -> Vec<u8> {
    let img = image::GrayImage::from_fn(16, 16, |x, y| {
        image::Luma([((x * 13 + y * 7 + pattern as u32) % 256) as u8])
    });
    let mut cursor = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(img)
        .write_to(&mut cursor, image::ImageFormat::Png)
        .unwrap();
    cursor.into_inner()
}

fn wav_with_cover(wav: &[u8], cover: &[u8]) -> Vec<u8> {
    // 写入临时文件再由 lofty 嵌封面（WAV 容器）
    let tmp = std::env::temp_dir().join(format!(
        "mf-sc-cover-{}.wav",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&tmp, wav).unwrap();
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::prelude::*;
    use lofty::tag::TagType;
    let mut tagged = lofty::read_from_path(&tmp).unwrap();
    // WAV 的 RiffInfo 不支持图片块（save 会静默跳过）——封面必须走 ID3v2 块
    let mut tag = lofty::tag::Tag::new(TagType::Id3v2);
    let pic = Picture::unchecked(cover.to_vec())
        .mime_type(MimeType::Png)
        .pic_type(PictureType::CoverFront)
        .build();
    tag.push_picture(pic);
    tagged.insert_tag(tag);
    tagged
        .save_to_path(&tmp, lofty::config::WriteOptions::default())
        .unwrap();
    let bytes = std::fs::read(&tmp).unwrap();
    std::fs::remove_file(&tmp).ok();
    bytes
}

#[test]
fn similar_cover_groups_identical_and_separates_different() {
    let root = uniq_root("cover");
    let lib = root.join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    let png_a = png_bytes(0);
    let png_b = png_bytes(100);
    assert_ne!(
        cover_ahash_dummy(&png_a),
        cover_ahash_dummy(&png_b),
        "图案不同 → aHash 应不同"
    );

    std::fs::write(lib.join("one.wav"), wav_with_cover(&wav_bytes(), &png_a)).unwrap();
    std::fs::write(lib.join("two.wav"), wav_with_cover(&wav_bytes(), &png_a)).unwrap();
    std::fs::write(lib.join("three.wav"), wav_with_cover(&wav_bytes(), &png_b)).unwrap();

    let groups = similar_cover_scan(&lib).unwrap();
    assert_eq!(
        groups.len(),
        1,
        "同封面 two 文件成组，异封面 three 独立: {groups:?}"
    );
    let g = &groups[0];
    assert_eq!(g.members.len(), 2);
    assert!(g.members.iter().all(|(_, _, d)| *d <= 8));
    std::fs::remove_dir_all(&root).ok();
}

/// 直接对 PNG 字节算 aHash（绕过 lofty，用于断言图案差异）。
fn cover_ahash_dummy(png: &[u8]) -> u64 {
    let img = image::load_from_memory(png).unwrap();
    // 复用 dedupe 内部算法不可见，这里用等价手写：8×8 均值阈值
    let gray = img.to_luma8();
    let small = image::imageops::resize(&gray, 8, 8, image::imageops::FilterType::Lanczos3);
    let mean = small.pixels().map(|p| p.0[0] as u64).sum::<u64>() / 64;
    small.pixels().enumerate().fold(0u64, |acc, (i, p)| {
        if p.0[0] as u64 > mean {
            acc | (1u64 << i)
        } else {
            acc
        }
    })
}
