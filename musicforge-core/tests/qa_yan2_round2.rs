//! QA（严过关）第二轮**独立验证**：B7 tagger 谎报 / T3 模板回退上限 / 第一轮修复回归。
//!
//! 与 `qa_adversarial.rs` 的区别：本文件所有容器载荷**逐字节自建**
//!（MPEG 帧、ID3v1、FLAC STREAMINFO、MP4 box 树），不复用既有 fixture 与辅助函数，
//! 以便对实现者的自查结论做第三方交叉验证。
//!
//! 核心不变量：`write_tags` 报告的 `(written, embedded)` 必须与**磁盘回读真值**一致。

use lofty::config::WriteOptions;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::*;
use lofty::tag::{ItemKey, Tag, TagType};
use musicforge_core::{tagger, Format, Metadata};
use std::path::{Path, PathBuf};

// ==================== 自建容器载荷（不复用任何既有辅助函数） ====================

/// 一帧 MPEG-1 Layer III / 128kbps / 44100Hz / stereo 帧。
/// 帧长 = floor(144 * 128000 / 44100) = 417 字节。
/// 字节 4 用 0x00（通道模式 stereo），与既有测试的 0x64 区分，避免同源。
fn mp3_frame() -> Vec<u8> {
    let mut f = vec![0xFFu8, 0xFB, 0x90, 0x00];
    f.extend(std::iter::repeat_n(0u8, 417 - 4));
    assert_eq!(f.len(), 417);
    f
}

fn bare_mp3(frames: usize) -> Vec<u8> {
    let mut v = Vec::new();
    for _ in 0..frames {
        v.extend(mp3_frame());
    }
    v
}

/// 128 字节 ID3v1 尾标签。字段以空格填充（真实世界定长标签的普遍形态）。
fn id3v1(title: &str, artist: &str, album: &str) -> Vec<u8> {
    let mut out = b"TAG".to_vec();
    for field in [title, artist, album] {
        let mut buf = vec![b' '; 30];
        let n = field.len().min(30);
        buf[..n].copy_from_slice(&field.as_bytes()[..n]);
        out.extend_from_slice(&buf);
    }
    out.extend_from_slice(b"2026"); // year
    out.extend(std::iter::repeat_n(b' ', 30)); // comment
    out.push(12); // genre
    assert_eq!(out.len(), 128);
    out
}

/// 最小合法 FLAC：`fLaC` + STREAMINFO（last-block）+ 帧区填充。
fn minimal_flac() -> Vec<u8> {
    let mut info = Vec::new();
    info.extend_from_slice(&4096u16.to_be_bytes()); // min blocksize
    info.extend_from_slice(&4096u16.to_be_bytes()); // max blocksize
    info.extend_from_slice(&[0u8; 3]); // min framesize
    info.extend_from_slice(&[0u8; 3]); // max framesize
                                       // 20bit sample_rate | 3bit(channels-1) | 5bit(bps-1) | 36bit total_samples
    let packed: u64 = (44100u64 << 44) | (1u64 << 41) | (15u64 << 36);
    info.extend_from_slice(&packed.to_be_bytes());
    info.extend_from_slice(&[0u8; 16]); // md5 of unencoded audio
    assert_eq!(info.len(), 34);

    let mut out = b"fLaC".to_vec();
    out.push(0x80); // last-metadata-block=1, type=0 (STREAMINFO)
    out.extend_from_slice(&[0u8, 0u8, 34u8]); // 24-bit 长度
    out.extend_from_slice(&info);
    out.extend_from_slice(&[0u8; 512]); // 帧区填充
    out
}

// ---------- MP4/M4A box 树（自建最小可解析音频容器） ----------

fn box32(tag: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
    v.extend_from_slice(tag);
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

fn ftyp() -> Vec<u8> {
    let mut p = b"M4A ".to_vec();
    p.extend(be32(0)); // minor version
    p.extend_from_slice(b"M4A ");
    p.extend_from_slice(b"mp42");
    p.extend_from_slice(b"isom");
    box32(b"ftyp", &p)
}

fn mvhd() -> Vec<u8> {
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
    assert_eq!(p.len(), 100);
    box32(b"mvhd", &p)
}

fn stsd() -> Vec<u8> {
    let mut entry = std::iter::repeat_n(0u8, 6).collect::<Vec<u8>>(); // reserved
    entry.extend(be16(1)); // data_reference_index
    entry.extend(be16(0)); // version
    entry.extend(be16(0)); // revision
    entry.extend(be32(0)); // vendor
    entry.extend(be16(2)); // channel_count
    entry.extend(be16(16)); // sample_size
    entry.extend(be16(0)); // compression_id
    entry.extend(be16(0)); // packet_size
    entry.extend(be32(44100 << 16)); // sample_rate 16.16
    assert_eq!(entry.len(), 28);
    let mp4a = box32(b"mp4a", &entry);

    let mut p = be32(0); // version+flags
    p.extend(be32(1)); // entry_count
    p.extend(mp4a);
    box32(b"stsd", &p)
}

fn minimal_mp4() -> Vec<u8> {
    // ---- stbl ----
    let mut stbl_p = stsd();
    stbl_p.extend(box32(b"stts", &[be32(0), be32(0)].concat()));
    stbl_p.extend(box32(b"stsc", &[be32(0), be32(0)].concat()));
    stbl_p.extend(box32(b"stsz", &[be32(0), be32(0), be32(0)].concat()));
    stbl_p.extend(box32(b"stco", &[be32(0), be32(0)].concat()));
    let stbl = box32(b"stbl", &stbl_p);

    // ---- minf ----
    let smhd = box32(b"smhd", &[be32(0), be16(0), be16(0)].concat());
    let dref = box32(
        b"dref",
        &[
            be32(0),
            be32(1),
            box32(b"url ", &be32(1)).to_vec().as_slice().to_vec(),
        ]
        .concat(),
    );
    let mut minf_p = smhd;
    minf_p.extend(box32(b"dinf", &dref));
    minf_p.extend(stbl);
    let minf = box32(b"minf", &minf_p);

    // ---- mdia ----
    let mdhd = box32(
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
    let hdlr = box32(
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
    let mdia = box32(b"mdia", &mdia_p);

    // ---- trak ----
    let tkhd = box32(
        b"tkhd",
        &[
            be32(7),
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
    let trak = box32(b"trak", &[tkhd, mdia].concat());

    // ---- moov / mdat ----
    let moov = box32(b"moov", &[mvhd(), trak].concat());
    let mdat = box32(b"mdat", &std::iter::repeat_n(0u8, 64).collect::<Vec<u8>>());

    [ftyp(), moov, mdat].concat()
}

// ==================== 被测样本与回读断言 ====================

fn meta(title: &str, artist: &str, album: &str) -> Metadata {
    Metadata {
        name: Some(title.to_string()),
        artist: Some(artist.to_string()),
        album: Some(album.to_string()),
        format: Some("mp3".to_string()),
        bitrate: None,
        duration: None,
        track: None,
        album_pic_url: None,
    }
}

/// 一个最小 8 字节 PNG 签名 + 载荷（够 lofty 判定 MimeType::Png）
const PNG_COVER: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0xAA, 0xBB, 0xCC,
];

fn tmp_file(name: &str, blob: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(name);
    std::fs::write(&p, blob).unwrap();
    (dir, p)
}

/// 回读并统计「真实落盘」的字段数：三个文本字段 + 封面（各计 1）。
fn disk_truth(
    p: &Path,
) -> (
    TagType,
    usize,
    usize,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let tagged = lofty::read_from_path(p).expect("产物必须可被 lofty 回读");
    let primary = tagged.primary_tag().expect("必须存在主标签");
    let mut fields = 0usize;
    for key in [
        ItemKey::TrackTitle,
        ItemKey::TrackArtist,
        ItemKey::AlbumTitle,
    ] {
        if primary
            .get_string(key)
            .is_some_and(|s| !s.trim().is_empty())
        {
            fields += 1;
        }
    }
    let pics: usize = tagged.tags().iter().map(|t| t.pictures().len()).sum();
    if pics > 0 {
        fields += 1;
    }
    (
        primary.tag_type(),
        fields,
        pics,
        primary
            .get_string(ItemKey::TrackTitle)
            .map(|s| s.to_string()),
        primary
            .get_string(ItemKey::TrackArtist)
            .map(|s| s.to_string()),
        primary
            .get_string(ItemKey::AlbumTitle)
            .map(|s| s.to_string()),
    )
}

// ==================== B7：只带 ID3v1 的 MP3（谎报场景核心） ====================

/// 子情形 ①：ID3v1 三个文本字段非空 + PNG 封面字节。
///
/// 修复前 `first_tag_mut()` 会选中 ID3v1：它不支持图片 → lofty 静默丢弃封面，
/// 函数仍返回 `embedded = true` → 谎报「已嵌入封面」。
#[test]
fn yan_b7_id3v1_three_fields_nonempty() {
    let mut payload = bare_mp3(4);
    payload.extend(id3v1("Old Title", "Old Artist", "Old Album"));
    let (_d, p) = tmp_file("id3v1_nonempty.mp3", &payload);

    let (written, embedded) = tagger::write_tags(
        &p,
        Format::Mp3,
        &meta("新标题", "新歌手", "新专辑"),
        PNG_COVER,
    )
    .expect("写入应成功");

    let (ptype, disk_fields, pics, title, artist, album) = disk_truth(&p);

    // 断言 1：embedded == true ⇒ 磁盘上真的有图片
    assert!(embedded, "无已有封面且传入封面 → 应报告 embedded=true");
    assert!(
        pics > 0,
        "报 embedded=true，磁盘回读 pictures 却为 0（谎报）"
    );
    assert_eq!(pics, 1, "封面数应为 1，实际 {pics}");

    // 断言 2：主标签类型必须是 ID3v2（容器主标签），不是能力受限的 ID3v1
    assert_eq!(
        ptype,
        TagType::Id3v2,
        "写入目标必须是 ID3v2，实际 {ptype:?}"
    );

    // 断言 3：三字段回读值与写入值**精确相等**
    assert_eq!(title.as_deref(), Some("新标题"), "title 回读不符");
    assert_eq!(artist.as_deref(), Some("新歌手"), "artist 回读不符");
    assert_eq!(album.as_deref(), Some("新专辑"), "album 回读不符");

    // 断言 4：written 与实际落盘字段数一致（3 文本 + 1 封面 = 4）
    assert_eq!(written, 4, "written 应为 4，实际 {written}");
    assert_eq!(
        written, disk_fields,
        "written={written} 与磁盘真值 {disk_fields} 不一致（谎报）"
    );

    // 断言 5：封面字节内容必须与传入的完全一致（不是被重新编码/丢弃）
    let tagged = lofty::read_from_path(&p).unwrap();
    let primary = tagged.primary_tag().unwrap();
    assert_eq!(
        primary.pictures()[0].data(),
        PNG_COVER,
        "封面字节必须与传入值逐字节一致"
    );
    assert_eq!(
        primary.pictures()[0].mime_type(),
        Some(&MimeType::Png),
        "PNG 魔数应识别为 Png"
    );
}

/// 子情形 ②：ID3v1 三个字段全空（定长标签极常见：字段存在但内容全空格）。
///
/// 修复前 `get_string()` 返回 `Some("")` → 被判定为「已有值」→ 跳过写入 →
/// 元数据 100% 丢失却报成功。
#[test]
fn yan_b7_id3v1_all_fields_empty() {
    let mut payload = bare_mp3(4);
    payload.extend(id3v1("", "", ""));
    let (_d, p) = tmp_file("id3v1_blank.mp3", &payload);

    let (written, embedded) = tagger::write_tags(
        &p,
        Format::Mp3,
        &meta("新标题", "新歌手", "新专辑"),
        PNG_COVER,
    )
    .expect("写入应成功");

    let (ptype, disk_fields, pics, title, artist, album) = disk_truth(&p);

    assert!(
        written >= 3,
        "三字段全空 → 都应写入，实际 written={written}"
    );
    assert_eq!(ptype, TagType::Id3v2);
    assert_eq!(title.as_deref(), Some("新标题"));
    assert_eq!(artist.as_deref(), Some("新歌手"));
    assert_eq!(album.as_deref(), Some("新专辑"));
    if embedded {
        assert!(pics > 0, "报 embedded=true 却无图片落盘");
    }
    assert_eq!(
        written, disk_fields,
        "written={written} vs 磁盘真值 {disk_fields}"
    );
}

/// 子情形 ②b：ID3v1 字段为**纯空白**（"   "，非全空）——同样必须按缺失处理。
#[test]
fn yan_b7_id3v1_whitespace_only_fields() {
    let mut payload = bare_mp3(4);
    payload.extend(id3v1("   ", "\t", "  "));
    let (_d, p) = tmp_file("id3v1_ws.mp3", &payload);

    let (written, _) =
        tagger::write_tags(&p, Format::Mp3, &meta("标题2", "歌手2", "专辑2"), PNG_COVER)
            .expect("写入应成功");

    let (_, disk_fields, _, title, _, _) = disk_truth(&p);
    assert_eq!(title.as_deref(), Some("标题2"), "纯空白字段必须按缺失处理");
    assert_eq!(written, disk_fields);
    assert!(written >= 3, "纯空白三字段都应写入，实际 {written}");
}

/// 子情形 ③：ID3v1 + 有效封面字节（JPEG 分支，验证 mime 判定与字节保真）。
#[test]
fn yan_b7_id3v1_with_valid_cover_bytes() {
    let mut payload = bare_mp3(4);
    payload.extend(id3v1("T", "A", "AL"));
    let (_d, p) = tmp_file("id3v1_cover.mp3", &payload);

    let jpeg: Vec<u8> = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F'].to_vec();
    let (written, embedded) =
        tagger::write_tags(&p, Format::Mp3, &meta("T2", "A2", "AL2"), &jpeg).expect("写入应成功");

    let tagged = lofty::read_from_path(&p).unwrap();
    let primary = tagged.primary_tag().unwrap();
    assert!(
        embedded && !primary.pictures().is_empty(),
        "embedded=true 但主标签无图片"
    );
    assert_eq!(
        primary.pictures()[0].data(),
        jpeg.as_slice(),
        "JPEG 封面字节必须保真"
    );
    assert_eq!(
        primary.pictures()[0].mime_type(),
        Some(&MimeType::Jpeg),
        "JPEG 魔数应识别为 Jpeg"
    );
    assert!(written >= 1);
}

/// 加固：ID3v1 字段上限 30 字符。超长标题写入 ID3v1-only MP3 时
/// **不得被静默截断**（修复前落到 ID3v1 → lofty 静默截断到 30 字符）。
#[test]
fn yan_b7_long_title_not_silently_truncated() {
    let mut payload = bare_mp3(4);
    payload.extend(id3v1("Old", "Old", "Old"));
    let (_d, p) = tmp_file("id3v1_long.mp3", &payload);

    let long_title = "这是一个远超三十字符上限的超长标题内容用于验证静默截断是否已消除".to_string();
    assert!(long_title.chars().count() > 30);

    tagger::write_tags(
        &p,
        Format::Mp3,
        &meta(&long_title, "歌手", "专辑"),
        PNG_COVER,
    )
    .expect("写入应成功");

    let (ptype, _, _, title, _, _) = disk_truth(&p);
    assert_eq!(ptype, TagType::Id3v2);
    assert_eq!(
        title.as_deref(),
        Some(long_title.as_str()),
        "超长标题被截断（ID3v1 的 30 字符上限仍在生效）"
    );
}

/// 反向：不覆盖已有封面。目标已有图片时 `embedded` 必须为 false 且字节不变。
#[test]
fn yan_b7_does_not_overwrite_existing_cover() {
    let mut payload = bare_mp3(4);
    payload.extend(id3v1("T", "A", "AL"));
    let (_d, p) = tmp_file("has_cover.mp3", &payload);

    // 先用 lofty 直接写入一张「既有封面」
    let existing: Vec<u8> = [0x01, 0x02, 0x03, 0x04].to_vec();
    {
        let mut tf = lofty::read_from_path(&p).unwrap();
        let mut t = Tag::new(TagType::Id3v2);
        t.push_picture(
            Picture::unchecked(existing.clone())
                .mime_type(MimeType::Jpeg)
                .pic_type(PictureType::CoverFront)
                .build(),
        );
        tf.insert_tag(t);
        tf.save_to_path(&p, WriteOptions::default()).unwrap();
    }

    let (_, embedded) = tagger::write_tags(&p, Format::Mp3, &meta("T2", "A2", "AL2"), PNG_COVER)
        .expect("写入应成功");

    assert!(!embedded, "已有封面时不得报告 embedded=true");
    let tagged = lofty::read_from_path(&p).unwrap();
    let primary = tagged.primary_tag().unwrap();
    assert_eq!(
        primary.pictures()[0].data(),
        existing.as_slice(),
        "既有封面不得被覆盖"
    );
    assert_eq!(primary.pictures().len(), 1, "不得追加第二张封面");
}

// ==================== 反向回归：裸 MP3 / FLAC / MP4 三条路径 ====================
// 修复只改变「存在次要标签时的标签选择」。这三类载荷不存在次要标签，
// 因此行为必须与修复前一致：正常写入、不谎报。

#[test]
fn yan_b7_reverse_bare_mp3() {
    let (_d, p) = tmp_file("bare.mp3", &bare_mp3(4));
    let (written, embedded) =
        tagger::write_tags(&p, Format::Mp3, &meta("BT", "BA", "BAL"), PNG_COVER).expect("成功");

    let (ptype, disk_fields, pics, title, artist, album) = disk_truth(&p);
    assert_eq!(ptype, TagType::Id3v2);
    assert_eq!(title.as_deref(), Some("BT"));
    assert_eq!(artist.as_deref(), Some("BA"));
    assert_eq!(album.as_deref(), Some("BAL"));
    assert!(embedded && pics == 1);
    assert_eq!(written, 4);
    assert_eq!(written, disk_fields);
}

#[test]
fn yan_b7_reverse_flac() {
    let (_d, p) = tmp_file("s.flac", &minimal_flac());
    let (written, embedded) =
        tagger::write_tags(&p, Format::Flac, &meta("FT", "FA", "FAL"), PNG_COVER).expect("成功");

    let (ptype, disk_fields, pics, title, artist, album) = disk_truth(&p);
    assert_eq!(
        ptype,
        TagType::VorbisComments,
        "FLAC 主标签应为 Vorbis Comments"
    );
    assert_eq!(title.as_deref(), Some("FT"));
    assert_eq!(artist.as_deref(), Some("FA"));
    assert_eq!(album.as_deref(), Some("FAL"));
    if embedded {
        assert!(pics > 0, "报 embedded=true 但 FLAC 回读无图片");
    }
    assert_eq!(
        written, disk_fields,
        "written={written} vs 磁盘 {disk_fields}"
    );
}

#[test]
fn yan_b7_reverse_mp4() {
    let blob = minimal_mp4();
    let (_d, p) = tmp_file("s.m4a", &blob);
    // 前置校验：自建 MP4 必须能被 lofty 解析（否则本用例无意义）
    let probe = lofty::read_from_path(&p);
    assert!(probe.is_ok(), "自建最小 MP4 无法被 lofty 解析");

    let (written, embedded) =
        tagger::write_tags(&p, Format::M4a, &meta("MT", "MA", "MAL"), PNG_COVER).expect("成功");

    let (ptype, disk_fields, pics, title, artist, album) = disk_truth(&p);
    assert_eq!(ptype, TagType::Mp4Ilst, "MP4 主标签应为 ilst");
    assert_eq!(title.as_deref(), Some("MT"));
    assert_eq!(artist.as_deref(), Some("MA"));
    assert_eq!(album.as_deref(), Some("MAL"));
    if embedded {
        assert!(pics > 0, "报 embedded=true 但 MP4 回读无图片");
    }
    assert_eq!(
        written, disk_fields,
        "written={written} vs 磁盘 {disk_fields}"
    );
}

// ==================== T3：模板回退绕过段长上界 ====================

/// 模板所有段渲染为空 + 超长源文件名 → 回退结果必须仍受 100 字符上界约束。
#[test]
fn yan_t3_fallback_stem_is_clamped() {
    use musicforge_core::template::render_filename;

    let mut m = meta("t", "a", "al");
    m.format = None; // 让 {format} 渲染为空 → 所有段为空 → 走回退分支

    for (label, stem) in [
        ("300 个中文字符", "曲".repeat(300)),
        ("500 个 ASCII", "x".repeat(500)),
        ("恰好 101 字符", "y".repeat(101)),
        ("恰好 100 字符", "z".repeat(100)),
        ("混合中日英 250 字", "あ漢A".repeat(50)),
    ] {
        let out = render_filename("{format}", Some(&m), &stem);
        assert!(
            out.chars().count() <= 100,
            "[{label}] 回退值未受段长上界约束: {} 字符",
            out.chars().count()
        );
    }

    // 边界：恰好 100 字符时不得被破坏
    let exact = render_filename("{format}", Some(&m), &"z".repeat(100));
    assert_eq!(exact.chars().count(), 100, "恰好 100 字符应原样保留");
    assert_eq!(exact, "z".repeat(100));
}

/// 回退分支同样要清洗非法字符（不得因加了截断而丢掉 sanitize）。
#[test]
fn yan_t3_fallback_stem_still_sanitized() {
    use musicforge_core::template::render_filename;

    let mut m = meta("t", "a", "al");
    m.format = None;
    let stem = format!("{}<>:|?*", "w".repeat(200));
    let out = render_filename("{format}", Some(&m), &stem);
    assert!(out.chars().count() <= 100);
    for c in ['<', '>', ':', '|', '?', '*'] {
        assert!(!out.contains(c), "回退值仍含非法字符 {c}");
    }
}

/// 把 lofty 实际写出的 ID3v2 头部形态**钉死**，因为它决定了
/// `scratch/check_id3v1_affected.py`（一次性受影响文件扫描脚本）的解析前提：
/// 该脚本对 ID3v2.4 **扩展头**的处理存在 4 字节偏差（v2.4 的 ext size 不计自身
/// 4 字节），一旦 lofty 开始写扩展头，脚本就会把「有封面」误报成「无封面」。
/// 本用例在 lofty 行为变化时会立刻红掉，提醒同步修脚本。
#[test]
fn yan_lofty_writes_id3v2_4_without_extended_header() {
    let (_d, p) = tmp_file("probe.mp3", &bare_mp3(4));
    tagger::write_tags(&p, Format::Mp3, &meta("探针", "QA", "专辑"), PNG_COVER).expect("成功");
    let bytes = std::fs::read(&p).unwrap();
    assert_eq!(&bytes[0..3], b"ID3", "产物必须以 ID3v2 开头");
    assert_eq!(bytes[3], 4, "lofty 应写 ID3v2.4");
    assert_eq!(
        bytes[5] & 0x40,
        0,
        "lofty 写出扩展头会让扫描脚本误报，需同步修脚本"
    );
    assert_eq!(
        bytes[5] & 0x80,
        0,
        "lofty 开启 unsynchronisation 需同步复核扫描脚本"
    );
}
