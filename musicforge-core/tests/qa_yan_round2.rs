//! QA(Yan) 第二轮**独立验证** —— B7/B7b/B7c（tagger 谎报）+ T3（模板回退段长）
//! + 为 Python 校验脚本 `scratch/check_id3v1_affected.py` 提供 lofty 真值。
//!
//! 独立性的含义：本文件**不 import、不复用**实现者 `qa_adversarial.rs` 里的任何
//! fixture 构造 helper。所有样本（MPEG 帧流 / ID3v1 / ID3v2.3 / ID3v2.4 / FLAC /
//! MP4 / 真 PNG）均逐字节手搓，参数取值刻意与实现者错开（不同码率/声道/填充字节/
//! 真实图片），避免「两人用同一份样本」的同源偏差。
//!
//! 核心不变量：**`write_tags` 报告的 `(written, embedded)` 必须与磁盘回读真值一致。**

use lofty::picture::MimeType;
use lofty::prelude::*;
use lofty::tag::{ItemKey, TagType};
use musicforge_core::format::Format;
use musicforge_core::metadata::Metadata;
use musicforge_core::tagger::write_tags;
use std::path::Path;

// ─────────────────────────────────────────────────────────────────────────────
// 1. 自造样本
// ─────────────────────────────────────────────────────────────────────────────

/// 一帧 MPEG-1 Layer III：128kbps / 44.1kHz / **单声道** / 无 CRC。
///
/// 帧头逐位手搓（与实现者用的 0x90 0x64「联合立体声」刻意错开为单声道）：
///   FF : 同步字 11111111
///   FB : 111 同步续 + 11=MPEG1 + 01=Layer III + 1=无 CRC
///   90 : 1001=码率索引9(128kbps) + 00=44100Hz + 0=无填充 + 0=私有
///   C4 : 11=单声道 + 00 + 0=无版权 + 1=原版 + 00=无强调
/// 帧长 = floor(144 * 128000 / 44100) = 417 字节（含 4 字节头）。
/// 填充字节用 0x5A 而非 0x00：非零负载能暴露「写回时被截断/错位」。
pub fn mpeg_frames(n: usize) -> Vec<u8> {
    const FRAME_LEN: usize = 417;
    let mut v = Vec::with_capacity(FRAME_LEN * n);
    for _ in 0..n {
        v.extend_from_slice(&[0xff, 0xfb, 0x90, 0xc4]);
        v.extend(std::iter::repeat_n(0x5a_u8, FRAME_LEN - 4));
    }
    v
}

/// 128 字节 ID3v1 尾标签。三个文本字段各 30 字节，空格右填充。
pub fn id3v1(title: &str, artist: &str, album: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(128);
    out.extend_from_slice(b"TAG");
    for f in [title, artist, album] {
        let mut buf = vec![b' '; 30];
        let b = f.as_bytes();
        let n = b.len().min(30);
        buf[..n].copy_from_slice(&b[..n]);
        out.extend_from_slice(&buf);
    }
    out.extend_from_slice(b"2024");
    out.extend(std::iter::repeat_n(b' ', 30));
    out.push(12); // genre: Other
    assert_eq!(out.len(), 128);
    out
}

fn syncsafe(n: u32) -> [u8; 4] {
    [
        ((n >> 21) & 0x7f) as u8,
        ((n >> 14) & 0x7f) as u8,
        ((n >> 7) & 0x7f) as u8,
        (n & 0x7f) as u8,
    ]
}

/// 手搓 ID3v2.3 文本帧（TIT2/TPE1/TALB）。
/// `value` 为空串 ⇒ 「帧存在但内容为空」，即 B7b 的触发条件。
pub fn id3v2_text_frame(id: &[u8; 4], value: &str) -> Vec<u8> {
    let mut data = vec![0x03u8]; // UTF-8
    data.extend_from_slice(value.as_bytes());
    let mut f = Vec::new();
    f.extend_from_slice(id);
    f.extend_from_slice(&(data.len() as u32).to_be_bytes()); // v2.3 帧长是普通大端
    f.extend_from_slice(&[0x00, 0x00]);
    f.extend_from_slice(&data);
    f
}

/// 手搓 ID3v2.3 APIC 封面帧。
pub fn id3v2_apic_frame(png: &[u8]) -> Vec<u8> {
    let mut data = vec![0x00u8];
    data.extend_from_slice(b"image/png\0");
    data.push(0x03); // CoverFront
    data.push(0x00);
    data.extend_from_slice(png);
    let mut f = Vec::new();
    f.extend_from_slice(b"APIC");
    f.extend_from_slice(&(data.len() as u32).to_be_bytes());
    f.extend_from_slice(&[0x00, 0x00]);
    f.extend_from_slice(&data);
    f
}

/// 组装完整 ID3v2.3 标签（含 10 字节头）。
pub fn id3v2_tag(frames: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = frames.iter().flatten().copied().collect();
    let mut out = Vec::new();
    out.extend_from_slice(b"ID3");
    out.extend_from_slice(&[0x03, 0x00]);
    out.push(0x00);
    out.extend_from_slice(&syncsafe(body.len() as u32));
    out.extend_from_slice(&body);
    out
}

/// QA(Yan) 自造：8x8 真彩色 PNG，结构完整（IHDR/IDAT/IEND + CRC 校验通过），165 字节。
/// 与实现者用的 12 字节伪 PNG 桩不同 —— 真实图片字节才能证明封面确实按原字节落盘。
pub const REAL_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x08, 0x02, 0x00, 0x00, 0x00, 0x4b, 0x6d, 0x29,
    0xdc, 0x00, 0x00, 0x00, 0x6c, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x15, 0xcd, 0x41, 0x15, 0x00,
    0x51, 0x08, 0x42, 0x51, 0xa3, 0x18, 0x85, 0x28, 0x46, 0x79, 0x51, 0x88, 0x42, 0x14, 0xa2, 0xcc,
    0x1f, 0x97, 0x5c, 0x0e, 0xce, 0x0c, 0x3b, 0x68, 0xb8, 0x81, 0xc1, 0x43, 0x86, 0x0e, 0x33, 0xcb,
    0x2e, 0x5a, 0x6e, 0x61, 0xf1, 0x92, 0xa5, 0xfb, 0x40, 0xac, 0x90, 0x38, 0x81, 0xb0, 0x88, 0xa8,
    0x1e, 0x1c, 0x7b, 0xe8, 0xb8, 0x83, 0xc3, 0x47, 0x8e, 0xde, 0x83, 0x7f, 0xe0, 0x55, 0x5f, 0xf8,
    0x9f, 0x21, 0xd0, 0xf7, 0x6e, 0xcc, 0x1a, 0x99, 0xf3, 0x1f, 0xdb, 0xc4, 0xd4, 0x0f, 0xc2, 0x06,
    0x85, 0xcb, 0x5f, 0x76, 0x48, 0x68, 0x1e, 0x94, 0x2d, 0x2a, 0xd7, 0x7f, 0xc2, 0x25, 0xa5, 0xe5,
    0x03, 0xc6, 0x7b, 0x58, 0x01, 0x57, 0x39, 0x36, 0xf2, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e,
    0x44, 0xae, 0x42, 0x60, 0x82,
];

pub fn meta() -> Metadata {
    Metadata {
        name: Some("验证标题".to_string()),
        artist: Some("验证歌手".to_string()),
        album: Some("验证专辑".to_string()),
        format: Some("mp3".to_string()),
        track: None,
        bitrate: None,
        duration: None,
        album_pic_url: None,
    }
}

/// 回读：整文件所有标签里的图片总数。
fn total_pictures(p: &Path) -> usize {
    lofty::read_from_path(p)
        .expect("产物必须能被 lofty 重新读回")
        .tags()
        .iter()
        .map(|t| t.pictures().len())
        .sum()
}

/// 回读：主标签的指定文本字段。
fn primary_string(p: &Path, key: ItemKey) -> Option<String> {
    lofty::read_from_path(p)
        .expect("产物必须能被 lofty 重新读回")
        .primary_tag()
        .and_then(|t| t.get_string(key))
        .map(|s| s.to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. 先验自检：我手搓的样本本身必须合法
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn selftest_my_fixtures_are_readable() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("bare.mp3");
    std::fs::write(&p, mpeg_frames(4)).unwrap();
    let tagged = lofty::read_from_path(&p).expect("我手搓的 MPEG 帧流必须能被 lofty 识别为 MP3");
    assert_eq!(tagged.file_type(), lofty::file::FileType::Mpeg);
    assert_eq!(tagged.primary_tag_type(), TagType::Id3v2);

    let p2 = dir.path().join("v1.mp3");
    let mut v = mpeg_frames(4);
    v.extend(id3v1("旧标题", "旧歌手", "旧专辑"));
    std::fs::write(&p2, &v).unwrap();
    let t2 = lofty::read_from_path(&p2).expect("ID3v1 样本必须可读");
    assert!(t2.tags().iter().any(|t| t.tag_type() == TagType::Id3v1));
    assert_eq!(t2.primary_tag_type(), TagType::Id3v2);
}

#[test]
fn selftest_real_png_is_a_valid_png() {
    assert_eq!(&REAL_PNG[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(&REAL_PNG[12..16], b"IHDR");
    assert!(REAL_PNG.windows(4).any(|w| w == b"IDAT"));
    assert!(REAL_PNG.windows(4).any(|w| w == b"IEND"));
}

#[test]
fn selftest_my_flac_is_readable() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("x.flac");
    std::fs::write(&p, minimal_flac(&[("TITLE", "旧标题")])).unwrap();
    let t = lofty::read_from_path(&p).expect("我手搓的 FLAC 必须能被 lofty 识别");
    assert_eq!(t.file_type(), lofty::file::FileType::Flac);
    assert_eq!(t.primary_tag_type(), TagType::VorbisComments);
    assert_eq!(
        t.primary_tag()
            .and_then(|x| x.get_string(ItemKey::TrackTitle)),
        Some("旧标题"),
        "手搓的 Vorbis Comment 必须能被读出"
    );
}

#[test]
fn selftest_my_m4a_is_readable() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("x.m4a");
    std::fs::write(&p, minimal_m4a()).unwrap();
    match lofty::read_from_path(&p) {
        Ok(t) => {
            assert_eq!(t.file_type(), lofty::file::FileType::Mp4);
            assert_eq!(t.primary_tag_type(), TagType::Mp4Ilst);
        }
        Err(e) => panic!(
            "我手搓的最小 M4A 未被 lofty 接受: {e} —— 若属实，MP4 路径须降级为「仅静态验证」"
        ),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. B7 ① ID3v1 三字段非空
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn b7_id3v1_nonempty_writes_into_id3v2_and_cover_lands() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("id3v1_full.mp3");

    // 三个字段取满 30 字节的边界值
    let mut payload = mpeg_frames(4);
    payload.extend(id3v1(&"T".repeat(30), &"A".repeat(30), &"L".repeat(30)));
    std::fs::write(&p, &payload).unwrap();

    let (written, embedded) =
        write_tags(&p, Format::Mp3, &meta(), REAL_PNG).expect("只带 ID3v1 的 MP3 必须能写标签");

    // 断言 1：embedded == true ⇒ 磁盘上必须真有图片（核心不变量）
    assert!(embedded, "无封面且传入非空封面 ⇒ 应报告已嵌入");
    assert!(
        total_pictures(&p) > 0,
        "报告 embedded=true，但磁盘回读图片数为 0 —— 这就是「谎报」"
    );

    // 断言 2：写入目标必须是容器主标签类型 Id3v2
    let tagged = lofty::read_from_path(&p).unwrap();
    let primary = tagged.primary_tag().expect("应有主标签");
    assert_eq!(
        primary.tag_type(),
        TagType::Id3v2,
        "写入目标必须是 Id3v2，不得落到 ID3v1（不支持图片、字段上限 30 字符）"
    );

    // 断言 3：三字段回读值与写入值精确相等（不得被静默截断）
    for (key, expect) in [
        (ItemKey::TrackTitle, "验证标题"),
        (ItemKey::TrackArtist, "验证歌手"),
        (ItemKey::AlbumTitle, "验证专辑"),
    ] {
        assert_eq!(
            primary_string(&p, key).as_deref(),
            Some(expect),
            "{key:?} 回读值必须与写入值精确相等"
        );
    }

    // 断言 4：written 与实际写入字段数一致
    assert_eq!(
        written, 4,
        "3 文本 + 1 封面 ⇒ written 应为 4，实际 {written}"
    );

    // 断言 5：原始 ID3v1 不应被破坏
    let v1 = lofty::read_from_path(&p)
        .unwrap()
        .tags()
        .iter()
        .find(|t| t.tag_type() == TagType::Id3v1)
        .and_then(|t| t.get_string(ItemKey::TrackTitle).map(|s| s.to_string()));
    assert_eq!(
        v1.as_deref(),
        Some("T".repeat(30).as_str()),
        "原有 ID3v1 不应被改写（语义：不覆盖已有值）"
    );
}

/// 加固：ID3v1 字段上限 30 字符。超长标题写入 ID3v1-only MP3 时
/// **不得被静默截断**（修复前落到 ID3v1 → lofty 静默截断到 30 字符）。
#[test]
fn b7_long_title_not_silently_truncated() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("id3v1_long.mp3");
    let mut payload = mpeg_frames(4);
    payload.extend(id3v1("Old", "Old", "Old"));
    std::fs::write(&p, &payload).unwrap();

    let long_title =
        "这是一个远超三十字符上限的超长标题内容用于验证静默截断是否已经彻底消除".to_string();
    assert!(long_title.chars().count() > 30);

    let mut m = meta();
    m.name = Some(long_title.clone());
    write_tags(&p, Format::Mp3, &m, REAL_PNG).expect("写入应成功");

    let tagged = lofty::read_from_path(&p).unwrap();
    assert_eq!(tagged.primary_tag().unwrap().tag_type(), TagType::Id3v2);
    assert_eq!(
        primary_string(&p, ItemKey::TrackTitle).as_deref(),
        Some(long_title.as_str()),
        "超长标题被截断（ID3v1 的 30 字符上限仍在生效）"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. B7 ② 三字段全空 —— 用真实存在的空 ID3v2.3 帧来打（命中 has_value 判空逻辑）
// ─────────────────────────────────────────────────────────────────────────────

/// 比「ID3v1 全空格」更严格：ID3v1 根本不是写入目标，那个用例其实被「容器切换」
/// 顺带修好了，没有真正打到 `has_value()`。这里改用**真实存在的 ID3v2.3 空文本帧**
/// （TIT2/TPE1/TALB 存在但内容为空），才是 `get_string()` 返回 `Some("")` 的场景。
#[test]
fn b7b_empty_id3v2_text_frames_count_as_missing() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("empty_frames.mp3");
    let mut payload = id3v2_tag(&[
        id3v2_text_frame(b"TIT2", ""),
        id3v2_text_frame(b"TPE1", ""),
        id3v2_text_frame(b"TALB", ""),
    ]);
    payload.extend(mpeg_frames(4));
    std::fs::write(&p, &payload).unwrap();

    // 先验：确认样本确实是「字段存在但为空」
    let before = lofty::read_from_path(&p).unwrap();
    let before_tag = before.primary_tag().expect("应读到我手搓的 ID3v2.3");
    assert_eq!(before_tag.tag_type(), TagType::Id3v2);
    assert_eq!(
        before_tag.get_string(ItemKey::TrackTitle),
        Some(""),
        "样本前提：TIT2 存在且为空串（若不是，本用例未命中 B7b）"
    );

    let (written, embedded) = write_tags(&p, Format::Mp3, &meta(), REAL_PNG).expect("写入应成功");

    assert!(
        written >= 3,
        "空串必须按缺失处理并写入，实际 written={written}"
    );
    assert!(embedded);
    assert!(total_pictures(&p) > 0, "报 embedded=true 却无图片落盘");

    for (key, expect) in [
        (ItemKey::TrackTitle, "验证标题"),
        (ItemKey::TrackArtist, "验证歌手"),
        (ItemKey::AlbumTitle, "验证专辑"),
    ] {
        assert_eq!(primary_string(&p, key).as_deref(), Some(expect), "{key:?}");
    }
}

/// 变体：纯空白（" " / "\t"）—— `has_value` 用 `trim()` 判空，必须同样按缺失处理。
#[test]
fn b7b_whitespace_only_frames_count_as_missing() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("ws_frames.mp3");
    let mut payload = id3v2_tag(&[
        id3v2_text_frame(b"TIT2", "   "),
        id3v2_text_frame(b"TPE1", "\t"),
        id3v2_text_frame(b"TALB", " "),
    ]);
    payload.extend(mpeg_frames(4));
    std::fs::write(&p, &payload).unwrap();

    let (written, _) = write_tags(&p, Format::Mp3, &meta(), REAL_PNG).expect("写入应成功");
    assert!(written >= 3, "纯空白必须按缺失处理，实际 written={written}");
    assert_eq!(
        primary_string(&p, ItemKey::TrackTitle).as_deref(),
        Some("验证标题")
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. B7 ③ ID3v1 + 有效封面字节
// ─────────────────────────────────────────────────────────────────────────────

/// 只带 ID3v1 的 MP3 + **真实**封面字节（165 字节合法 PNG）。
/// 断言图片按原字节落盘，且 MIME 被正确识别为 PNG。
#[test]
fn b7c_id3v1_with_real_png_cover_lands_byte_exact() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("id3v1_cover.mp3");
    let mut payload = mpeg_frames(4);
    payload.extend(id3v1("旧标题", "旧歌手", "旧专辑"));
    std::fs::write(&p, &payload).unwrap();

    let (written, embedded) = write_tags(&p, Format::Mp3, &meta(), REAL_PNG).expect("写入应成功");
    assert!(embedded, "应报告已嵌入封面");

    let tagged = lofty::read_from_path(&p).unwrap();
    let pics: Vec<_> = tagged
        .tags()
        .iter()
        .flat_map(|t| t.pictures().iter())
        .collect();
    assert_eq!(pics.len(), 1, "磁盘上应恰好 1 张图片，实际 {}", pics.len());
    assert_eq!(pics[0].data(), REAL_PNG, "封面必须**逐字节**落盘");
    assert_eq!(
        pics[0].mime_type(),
        Some(&MimeType::Png),
        "PNG magic 应识别为 image/png"
    );
    assert_eq!(written, 4, "3 文本 + 1 封面，实际 {written}");
}

/// JPEG 分支：非 PNG 魔数应落到 MimeType::Jpeg。
#[test]
fn b7c_jpeg_cover_gets_jpeg_mime() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("j.mp3");
    let mut payload = mpeg_frames(4);
    payload.extend(id3v1("T", "A", "AL"));
    std::fs::write(&p, &payload).unwrap();

    let jpeg: Vec<u8> = vec![
        0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00,
    ];
    let (_, embedded) = write_tags(&p, Format::Mp3, &meta(), &jpeg).expect("写入应成功");
    assert!(embedded);
    let tagged = lofty::read_from_path(&p).unwrap();
    let pic = &tagged.primary_tag().unwrap().pictures()[0];
    assert_eq!(pic.data(), jpeg.as_slice(), "JPEG 封面字节必须保真");
    assert_eq!(pic.mime_type(), Some(&MimeType::Jpeg));
}

/// 反向：目标**已有**封面 ⇒ 必须**不**覆盖（防止修复过头成「总是覆盖」）。
#[test]
fn b7c_existing_cover_is_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("has_cover.mp3");
    let mut payload = id3v2_tag(&[id3v2_apic_frame(REAL_PNG)]);
    payload.extend(mpeg_frames(4));
    std::fs::write(&p, &payload).unwrap();
    assert_eq!(total_pictures(&p), 1, "样本前提：已有 1 张封面");

    // 传入一张不同的图
    let mut other = REAL_PNG.to_vec();
    other.extend_from_slice(&[0x01, 0x02, 0x03]);
    let (written, embedded) = write_tags(&p, Format::Mp3, &meta(), &other).expect("写入应成功");

    assert!(!embedded, "已有封面 ⇒ 不得覆盖，embedded 必须为 false");
    assert_eq!(total_pictures(&p), 1, "图片数量应保持 1");
    let tagged = lofty::read_from_path(&p).unwrap();
    assert_eq!(
        tagged.primary_tag().unwrap().pictures()[0].data(),
        REAL_PNG,
        "原有封面字节必须原样保留"
    );
    assert_eq!(
        written, 3,
        "只应写入 3 个文本字段（无封面），实际 {written}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 6. 反向 / 回归：裸 MP3、FLAC、MP4 三条路径
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn regression_bare_mp3_path() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("bare.mp3");
    std::fs::write(&p, mpeg_frames(4)).unwrap();

    let (written, embedded) =
        write_tags(&p, Format::Mp3, &meta(), REAL_PNG).expect("裸 MP3 写入应成功");
    assert_eq!(written, 4);
    assert!(embedded);
    let tagged = lofty::read_from_path(&p).unwrap();
    assert_eq!(tagged.primary_tag().unwrap().tag_type(), TagType::Id3v2);
    assert_eq!(
        primary_string(&p, ItemKey::TrackTitle).as_deref(),
        Some("验证标题")
    );
    assert_eq!(total_pictures(&p), 1);
}

/// 空封面 ⇒ `embedded` 必须为 false，且不得谎报。
#[test]
fn regression_empty_cover_never_claims_embedded() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("bare.mp3");
    std::fs::write(&p, mpeg_frames(4)).unwrap();

    let (written, embedded) = write_tags(&p, Format::Mp3, &meta(), &[]).expect("写入应成功");
    assert!(!embedded, "传入空封面 ⇒ 绝不得报告 embedded=true");
    assert_eq!(written, 3, "空封面时 written 应为 3（仅文本）");
    assert_eq!(total_pictures(&p), 0, "不得凭空产生图片");
}

/// 自造最小 FLAC：`fLaC` + STREAMINFO + VORBIS_COMMENT。
pub fn minimal_flac(comments: &[(&str, &str)]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"fLaC");

    let mut si = Vec::new();
    si.extend_from_slice(&4096u16.to_be_bytes()); // min block size
    si.extend_from_slice(&4096u16.to_be_bytes()); // max block size
    si.extend_from_slice(&[0u8; 3]); // min frame size (24bit)
    si.extend_from_slice(&[0u8; 3]); // max frame size (24bit)
                                     // sample_rate(20) | (channels-1)(3) | (bps-1)(5) | total_samples(36)
    let packed: u64 = ((44100u64 & 0xfffff) << 44) | (1u64 << 41) | (15u64 << 36);
    si.extend_from_slice(&packed.to_be_bytes()); // 8 字节
    si.extend(std::iter::repeat_n(0u8, 16)); // MD5
    assert_eq!(si.len(), 34, "STREAMINFO 必须 34 字节");
    v.push(0x00); // 非最后一块, type 0
    v.extend_from_slice(&(si.len() as u32).to_be_bytes()[1..]);
    v.extend_from_slice(&si);

    let mut vc = Vec::new();
    let vendor = b"musicforge-qa";
    vc.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    vc.extend_from_slice(vendor);
    vc.extend_from_slice(&(comments.len() as u32).to_le_bytes());
    for (k, val) in comments {
        let entry = format!("{k}={val}");
        vc.extend_from_slice(&(entry.len() as u32).to_le_bytes());
        vc.extend_from_slice(entry.as_bytes());
    }
    v.push(0x80 | 0x04); // 最后一块, type 4
    v.extend_from_slice(&(vc.len() as u32).to_be_bytes()[1..]);
    v.extend_from_slice(&vc);
    v
}

#[test]
fn regression_flac_path_writes_vorbis_comments() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("x.flac");
    std::fs::write(&p, minimal_flac(&[])).unwrap();

    let (written, embedded) =
        write_tags(&p, Format::Flac, &meta(), REAL_PNG).expect("FLAC 写入应成功");
    assert_eq!(written, 4, "FLAC 也应写 3 文本 + 1 封面，实际 {written}");
    assert!(embedded);
    let tagged = lofty::read_from_path(&p).unwrap();
    assert_eq!(
        tagged.primary_tag().unwrap().tag_type(),
        TagType::VorbisComments
    );
    assert_eq!(
        primary_string(&p, ItemKey::TrackTitle).as_deref(),
        Some("验证标题")
    );
    assert_eq!(total_pictures(&p), 1, "FLAC 封面必须真的落盘");
}

/// FLAC 上同样验证 B7b：Vorbis Comment 里 `TITLE=` 空值必须按缺失处理。
#[test]
fn b7b_flac_empty_vorbis_value_counts_as_missing() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("x.flac");
    std::fs::write(
        &p,
        minimal_flac(&[("TITLE", ""), ("ARTIST", ""), ("ALBUM", "")]),
    )
    .unwrap();

    let before = lofty::read_from_path(&p).unwrap();
    assert_eq!(
        before
            .primary_tag()
            .and_then(|t| t.get_string(ItemKey::TrackTitle)),
        Some(""),
        "样本前提：TITLE 存在且为空（若不是，本用例未命中 B7b）"
    );

    let (written, _) = write_tags(&p, Format::Flac, &meta(), REAL_PNG).expect("写入应成功");
    assert!(
        written >= 3,
        "FLAC 空值也必须按缺失写入，实际 written={written}"
    );
    assert_eq!(
        primary_string(&p, ItemKey::TrackTitle).as_deref(),
        Some("验证标题")
    );
    assert_eq!(
        primary_string(&p, ItemKey::TrackArtist).as_deref(),
        Some("验证歌手")
    );
}

pub fn mp4_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
    v.extend_from_slice(kind);
    v.extend_from_slice(payload);
    v
}

fn be32(v: u32) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

fn be16(v: u16) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

/// unity matrix（9 × 32bit）
fn unity_matrix() -> Vec<u8> {
    let mut v = Vec::new();
    for m in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        v.extend(be32(m));
    }
    assert_eq!(v.len(), 36);
    v
}

/// 自造最小 M4A：`ftyp` + `moov(mvhd, trak(tkhd, mdia(...)))` + `mdat`。
///
/// ⚠ 只给 `ftyp + moov(mvhd)` 是不够的：lofty 会报 "failed to parse Mp4 file"，
/// 因为 mp4ameta 需要一个带音频轨道的完整 box 树才会接受该文件。
pub fn minimal_m4a() -> Vec<u8> {
    let ftyp = {
        let mut p = b"M4A ".to_vec();
        p.extend(be32(0)); // minor version
        p.extend_from_slice(b"M4A");
        p.extend_from_slice(b"mp42");
        p.extend_from_slice(b"isom");
        mp4_box(b"ftyp", &p)
    };

    let mvhd = {
        let mut p = be32(0); // version+flags
        p.extend(be32(0)); // creation
        p.extend(be32(0)); // modification
        p.extend(be32(1000)); // timescale
        p.extend(be32(1000)); // duration
        p.extend(be32(0x0001_0000)); // rate 1.0
        p.extend(be16(0x0100)); // volume 1.0
        p.extend(be16(0)); // reserved
        p.extend(std::iter::repeat_n(0u8, 8)); // reserved
        p.extend(unity_matrix());
        p.extend(std::iter::repeat_n(0u8, 24)); // pre_defined
        p.extend(be32(2)); // next_track_ID
        assert_eq!(p.len(), 100, "mvhd v0 payload 必须 100 字节");
        mp4_box(b"mvhd", &p)
    };

    // ---- stbl ----
    let mut stsd_entry = std::iter::repeat_n(0u8, 6).collect::<Vec<u8>>(); // reserved
    stsd_entry.extend(be16(1)); // data_reference_index
    stsd_entry.extend(be16(0)); // version
    stsd_entry.extend(be16(0)); // revision
    stsd_entry.extend(be32(0)); // vendor
    stsd_entry.extend(be16(2)); // channel_count
    stsd_entry.extend(be16(16)); // sample_size
    stsd_entry.extend(be16(0)); // compression_id
    stsd_entry.extend(be16(0)); // packet_size
    stsd_entry.extend(be32(44100 << 16)); // sample_rate 16.16
    assert_eq!(stsd_entry.len(), 28);
    let stsd = {
        let mut p = be32(0); // version+flags
        p.extend(be32(1)); // entry_count
        p.extend(mp4_box(b"mp4a", &stsd_entry));
        mp4_box(b"stsd", &p)
    };
    let mut stbl_p = stsd;
    stbl_p.extend(mp4_box(b"stts", &[be32(0), be32(0)].concat()));
    stbl_p.extend(mp4_box(b"stsc", &[be32(0), be32(0)].concat()));
    stbl_p.extend(mp4_box(b"stsz", &[be32(0), be32(0), be32(0)].concat()));
    stbl_p.extend(mp4_box(b"stco", &[be32(0), be32(0)].concat()));
    let stbl = mp4_box(b"stbl", &stbl_p);

    // ---- minf ----
    let smhd = mp4_box(b"smhd", &[be32(0), be16(0), be16(0)].concat());
    let dref = mp4_box(
        b"dref",
        &[be32(0), be32(1), mp4_box(b"url ", &be32(1))].concat(),
    );
    let mut minf_p = smhd;
    minf_p.extend(mp4_box(b"dinf", &dref));
    minf_p.extend(stbl);
    let minf = mp4_box(b"minf", &minf_p);

    // ---- mdia ----
    let mdhd = mp4_box(
        b"mdhd",
        &[
            be32(0),
            be32(0),
            be32(0),
            be32(44100),
            be32(1024),
            be16(0x55C4), // 'und'
            be16(0),
        ]
        .concat(),
    );
    let hdlr = mp4_box(
        b"hdlr",
        &[
            be32(0),
            be32(0),
            b"soun".to_vec(),
            std::iter::repeat_n(0u8, 12).collect::<Vec<u8>>(),
            vec![0u8],
        ]
        .concat(),
    );
    let mut mdia_p = mdhd;
    mdia_p.extend(hdlr);
    mdia_p.extend(minf);
    let mdia = mp4_box(b"mdia", &mdia_p);

    // ---- trak ----
    let tkhd = mp4_box(
        b"tkhd",
        &[
            be32(7), // version+flags
            be32(0),
            be32(0),
            be32(1), // track_ID
            be32(0), // reserved
            be32(1000),
            std::iter::repeat_n(0u8, 8).collect::<Vec<u8>>(),
            be16(0),      // layer
            be16(0),      // alternate_group
            be16(0x0100), // volume
            be16(0),
            unity_matrix(),
            be32(0), // width
            be32(0), // height
        ]
        .concat(),
    );
    let trak = mp4_box(b"trak", &[tkhd, mdia].concat());

    let moov = mp4_box(b"moov", &[mvhd, trak].concat());
    let mdat = mp4_box(b"mdat", &std::iter::repeat_n(0u8, 64).collect::<Vec<u8>>());
    [ftyp, moov, mdat].concat()
}

#[test]
fn regression_mp4_path_writes_ilst() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("x.m4a");
    std::fs::write(&p, minimal_m4a()).unwrap();

    let (written, embedded) =
        write_tags(&p, Format::M4a, &meta(), REAL_PNG).expect("M4A 写入应成功");
    assert!(written >= 3, "M4A 至少应写 3 个文本字段，实际 {written}");
    if embedded {
        assert!(total_pictures(&p) > 0, "M4A 报 embedded=true 却无图片落盘");
    }
    assert_eq!(
        primary_string(&p, ItemKey::TrackTitle).as_deref(),
        Some("验证标题")
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 7. 容器/格式错配：不得 panic（硬约束 1）
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn container_mismatch_never_panics() {
    let dir = tempfile::tempdir().unwrap();
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("a.mp3", mpeg_frames(4)),
        ("b.flac", minimal_flac(&[])),
        ("c.m4a", minimal_m4a()),
    ];
    for (name, bytes) in &cases {
        let p = dir.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        for fmt in [Format::Mp3, Format::Flac, Format::M4a] {
            let r = std::panic::catch_unwind(|| write_tags(&p, fmt, &meta(), REAL_PNG));
            assert!(r.is_ok(), "{name} + {fmt:?} 触发了 panic（硬约束 1 违反）");
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 8. T3 模板回退同样受段长上界约束
// ─────────────────────────────────────────────────────────────────────────────

/// 模板所有段渲染为空 + 超长源文件名 → 回退结果必须被截断到 100 字符。
///
/// 触发方式刻意与实现者（`{format}` + format=None）不同：这里用
/// `{album}` + album=Some("")。`value_for` 对 `{album}` 是
/// `meta.and_then(|m| m.album.clone())`，拿到 `Some("")` 直接返回空串 ⇒
/// 段渲染为空 ⇒ 走回退分支。
///
/// ⚠ 不能用 `{track}`：`track_value` 在 track=None 时返回 "00"（非空），
/// 段不会被视为空，压根不会进回退分支 —— 用它会写出一条**假通过**的用例。
#[test]
fn t3_fallback_stem_is_truncated() {
    let mut m = meta();
    m.album = Some(String::new()); // 空专辑名 ⇒ {album} 渲染为空 ⇒ 全部段为空
    let long_stem = "曲".repeat(300);
    let out = musicforge_core::template::render_filename("{album}", Some(&m), &long_stem);
    // 2026-09-05 契约升级：段长上界从「100 字符」变为 min(100 字符, 200 字节)。
    // 根因：Linux/macOS 文件名组件上限 255 字节，"曲"×100 = 300 字节在 ext4 上
    // 直接 ENAMETOOLONG（CI ubuntu 实测暴露）。字符/字节双上限缺一不可。
    assert!(
        out.len() <= 200 && out.chars().count() <= 100,
        "回退分支必须受字符+字节双上限约束，实际 {} chars / {} bytes",
        out.chars().count(),
        out.len()
    );
}

/// 边界：源文件名 99/100/101/260 字符（在截断阈值两侧各测一次）。
/// 用 `{format}` + format=None 触发回退（`value_for` 走 `unwrap_or_default()` ⇒ 空串）。
#[test]
fn t3_fallback_boundary() {
    let mut m = meta();
    m.format = None;
    for n in [99usize, 100, 101, 260] {
        let stem = "A".repeat(n);
        let out = musicforge_core::template::render_filename("{format}", Some(&m), &stem);
        assert_eq!(
            out.chars().count(),
            n.min(100),
            "源文件名 {n} 字符时回退结果应为 {} 字符，实际 {}",
            n.min(100),
            out.chars().count()
        );
    }
}

/// 回退分支同样要清洗非法字符（不得因加了截断而丢掉 sanitize）。
#[test]
fn t3_fallback_stem_still_sanitized() {
    let mut m = meta();
    m.format = None;
    let stem = format!("{}<>:|?*", "w".repeat(200));
    let out = musicforge_core::template::render_filename("{format}", Some(&m), &stem);
    assert!(out.chars().count() <= 100);
    for c in ['<', '>', ':', '|', '?', '*'] {
        assert!(!out.contains(c), "回退值仍含非法字符 {c}");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 9. 附件：为 Python 校验脚本提供 lofty 真值
// ─────────────────────────────────────────────────────────────────────────────
//
// `scratch/check_id3v1_affected.py` 按容器格式手写解析，用它决定 5701 个文件是否重转。
// 判定对错的**唯一权威**是 lofty（Rust 侧真实解析器）。这里把两个被怀疑存在
// 残留假阳性的边界样本交给 lofty，打印它的真值，供与脚本结论对拍。

fn v24_frame(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(id);
    f.extend_from_slice(&syncsafe(body.len() as u32));
    f.extend_from_slice(&[0x00, 0x00]);
    f.extend_from_slice(body);
    f
}

/// 组装 ID3v2.4 标签（可带扩展头）
fn id3v24(flags: u8, ext: &[u8], frames: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = ext.iter().chain(frames.iter().flatten()).copied().collect();
    let mut out = Vec::new();
    out.extend_from_slice(b"ID3");
    out.extend_from_slice(&[0x04, 0x00]);
    out.push(flags);
    out.extend_from_slice(&syncsafe(body.len() as u32));
    out.extend_from_slice(&body);
    out
}

fn v24_apic(png: &[u8]) -> Vec<u8> {
    let mut b = vec![0x00u8];
    b.extend_from_slice(b"image/png\0");
    b.push(0x03);
    b.push(0x00);
    b.extend_from_slice(png);
    v24_frame(b"APIC", &b)
}

fn flac_block(t: u8, last: bool, body: &[u8]) -> Vec<u8> {
    let mut v = vec![if last { 0x80 | t } else { t }];
    v.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    v.extend_from_slice(body);
    v
}

fn flac_streaminfo() -> Vec<u8> {
    let mut si = Vec::new();
    si.extend_from_slice(&4096u16.to_be_bytes());
    si.extend_from_slice(&4096u16.to_be_bytes());
    si.extend_from_slice(&[0u8; 3]);
    si.extend_from_slice(&[0u8; 3]);
    let packed: u64 = (44100u64 << 44) | (1u64 << 41) | (15u64 << 36);
    si.extend_from_slice(&packed.to_be_bytes());
    si.extend_from_slice(&[0u8; 16]);
    si
}

fn vorbis_comment(entries: &[&str]) -> Vec<u8> {
    let mut v = Vec::new();
    let vendor = b"musicforge";
    v.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    v.extend_from_slice(vendor);
    v.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for e in entries {
        v.extend_from_slice(&(e.len() as u32).to_le_bytes());
        v.extend_from_slice(e.as_bytes());
    }
    v
}

/// 打印 lofty 真值（供与 Python 校验脚本对拍；本身不做断言）。
#[test]
fn yan_lofty_ground_truth_for_scanner_edge_cases() {
    let png: Vec<u8> = vec![
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3, 4, 5, 6, 7, 8,
    ];
    let dir = tempfile::tempdir().unwrap();

    let report = |name: &str, p: &Path| match lofty::read_from_path(p) {
        Ok(t) => {
            let primary_type = t.primary_tag_type();
            let pics: usize = t.tags().iter().map(|x| x.pictures().len()).sum();
            let has_primary = t.tag(primary_type).is_some();
            println!(
                "GROUND_TRUTH {name} | file_type={:?} | primary_tag_type={:?} | \
                 has_primary={} | total_pictures={}",
                t.file_type(),
                primary_type,
                has_primary,
                pics
            );
        }
        Err(e) => println!("GROUND_TRUTH {name} | READ_ERR: {e}"),
    };

    // --- N1：ID3v2.4 **带扩展头** + APIC。脚本在 v2.4 扩展头上少跳 4 字节 ---
    // v2.4 规范：扩展头 size 字段**不含**自身 4 字节。
    // ext = 1 字节「flag 字节数」+ 1 字节 flags ⇒ size=2
    let ext = [syncsafe(2).to_vec(), vec![0x01u8], vec![0x00u8]].concat();
    let mut n1 = id3v24(
        0x40,
        &ext,
        &[
            v24_frame(b"TIT2", &[0x03, b'T', b'i', b't']),
            v24_apic(&png),
        ],
    );
    n1.extend(mpeg_frames(4));
    let p1 = dir.path().join("N1_mp3_v24_extheader_apic.mp3");
    std::fs::write(&p1, &n1).unwrap();
    report("N1_mp3_v24_extheader_apic", &p1);

    // --- N2：FLAC 文件头**前置**一个 ID3v2（部分打标工具会这么写）---
    let mut n2 = id3v24(
        0x00,
        &[],
        &[
            v24_frame(b"TIT2", &[0x03, b'T', b'i', b't']),
            v24_apic(&png),
        ],
    );
    n2.extend_from_slice(b"fLaC");
    n2.extend(flac_block(0, false, &flac_streaminfo()));
    n2.extend(flac_block(4, true, &vorbis_comment(&["TITLE=Leading"])));
    let p2 = dir.path().join("N2_flac_with_leading_id3v2.flac");
    std::fs::write(&p2, &n2).unwrap();
    report("N2_flac_with_leading_id3v2", &p2);

    // --- 对照：无前置 ID3v2 的普通 FLAC（应有 VorbisComments 主标签）---
    let mut n3 = b"fLaC".to_vec();
    n3.extend(flac_block(0, false, &flac_streaminfo()));
    n3.extend(flac_block(4, true, &vorbis_comment(&["TITLE=Plain"])));
    let p3 = dir.path().join("N3_flac_plain.flac");
    std::fs::write(&p3, &n3).unwrap();
    report("N3_flac_plain", &p3);

    // --- N4：musicforge 自己产出的 MP3（lofty 写的标签）是哪一版、带不带扩展头？---
    // 这决定 N1 场景在真实产物里会不会出现。
    let p4 = dir.path().join("N4_musicforge_written.mp3");
    std::fs::write(&p4, mpeg_frames(4)).unwrap();
    write_tags(&p4, Format::Mp3, &meta(), &png).unwrap();
    let raw = std::fs::read(&p4).unwrap();
    println!(
        "GROUND_TRUTH N4_musicforge_written | ID3 头前 10 字节 = {:02x?} (byte3=版本, byte5=flags)",
        &raw[..raw.len().min(10)]
    );
    report("N4_musicforge_written", &p4);
}
