//! P5.2：CUE 解析（编码检测/宽容方言）与整轨切分。

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use musicforge_core::cue::{decode_bytes_to_utf8, parse_cue_text, split_cue};
use musicforge_core::lossless::{decode_to_pcm, LosslessFormat, PcmSpec};
use musicforge_core::synth::{sine, write_pcm};

fn uniq_root(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("mf-cue-{tag}-{n}-{seq}-{}", std::process::id()))
}

const CUE_UTF8: &str = "\
REM DATE 2023
REM GENRE Pop
PERFORMER \"周杰伦\"
TITLE \"叶惠美\"
FILE \"album.wav\" WAVE
  TRACK 01 AUDIO
    TITLE \"晴天\"
    PERFORMER \"周杰伦\"
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    TITLE \"七里香\"
    PERFORMER \"周杰伦\"
    INDEX 01 02:30:00
  TRACK 03 AUDIO
    TITLE \"搁浅\"
    PERFORMER \"周杰伦\"
    INDEX 01 05:00:00
";

#[test]
fn parse_utf8_cue_full_structure() {
    let sheet = parse_cue_text(CUE_UTF8).unwrap();
    assert_eq!(sheet.performer.as_deref(), Some("周杰伦"));
    assert_eq!(sheet.title.as_deref(), Some("叶惠美"));
    assert_eq!(sheet.rem_date.as_deref(), Some("2023"));
    assert_eq!(sheet.rem_genre.as_deref(), Some("Pop"));
    assert_eq!(sheet.file.as_deref(), Some("album.wav"));
    assert_eq!(sheet.tracks.len(), 3);
    assert_eq!(sheet.tracks[0].title.as_deref(), Some("晴天"));
    assert_eq!(sheet.tracks[1].index01_frames, Some((2 * 60 + 30) * 75));
    assert_eq!(sheet.tracks[2].number, 3);
}

#[test]
fn gbk_cue_decodes_to_correct_chinese() {
    // "晴天" GBK = C7E7 CCEC；"周杰伦" GBK = D6DC BDDC C2D7
    let mut bytes: Vec<u8> = Vec::new();
    bytes.extend_from_slice(b"PERFORMER \"");
    bytes.extend_from_slice(&[0xD6, 0xDC, 0xBD, 0xDC, 0xC2, 0xD7]);
    bytes.extend_from_slice(b"\"\r\nTITLE \"");
    bytes.extend_from_slice(&[0xC7, 0xE7, 0xCC, 0xEC]);
    bytes.extend_from_slice(b"\"\r\nFILE \"album.wav\" WAVE\r\n");
    bytes.extend_from_slice(b"  TRACK 01 AUDIO\r\n    INDEX 01 00:00:00\r\n");

    let text = decode_bytes_to_utf8(&bytes);
    assert!(text.contains("周杰伦"), "GBK 解码: {text}");
    assert!(text.contains("晴天"), "GBK 解码: {text}");
    let sheet = parse_cue_text(&text).unwrap();
    assert_eq!(sheet.tracks.len(), 1);
    // TRACK 之前的 TITLE 是专辑级（sheet.title）；轨级无 TITLE → None
    assert_eq!(sheet.title.as_deref(), Some("晴天"));
    assert_eq!(sheet.tracks[0].title, None);
}

#[test]
fn tolerant_dialect_ignores_unknown_commands() {
    let cue = "\
REM REPLAYGAIN_ALBUM_GAIN -6.20 dB
FOOBAR 42 \"unknown thing\"
CATALOG 1234567890123
FLAGS DCP
FILE \"x.wav\" WAVE
TRACK 01 AUDIO
  ISRC ABC123
  INDEX 00 00:00:00
  INDEX 01 00:01:00
";
    let sheet = parse_cue_text(cue).unwrap();
    assert_eq!(sheet.tracks.len(), 1);
    assert_eq!(sheet.tracks[0].index01_frames, Some(75));
}

#[test]
fn multi_file_cue_is_rejected_explicitly() {
    let cue = "\
FILE \"a.wav\" WAVE
TRACK 01 AUDIO
  INDEX 01 00:00:00
FILE \"b.wav\" WAVE
TRACK 02 AUDIO
  INDEX 01 01:00:00
";
    let err = parse_cue_text(cue).unwrap_err();
    assert!(err.to_string().contains("多文件镜像"), "{err}");
}

#[test]
fn cue_without_tracks_or_file_is_rejected() {
    assert!(parse_cue_text("TITLE \"empty\"").is_err());
    assert!(parse_cue_text("TRACK 01 AUDIO\nINDEX 01 00:00:00").is_err());
}

/// 构造整轨 WAV（5s 立体声 16bit）+ 3 轨 CUE，执行切分并全量断言。
#[test]
fn split_wav_three_tracks_with_tags() {
    let root = uniq_root("split");
    let lib = root.join("lib");
    std::fs::create_dir_all(&lib).unwrap();

    let spec = PcmSpec {
        channels: 2,
        sample_rate: 44100,
        bits_per_sample: 16,
    };
    let whole = sine(spec, 6.0, 440.0, 0.8);
    write_pcm(&lib.join("album.wav"), LosslessFormat::Wav, &whole).unwrap();

    // CUE 必须与音频同目录（FILE 路径相对 CUE 所在目录解析）
    let cue = lib.join("album.cue");
    std::fs::write(
        &cue,
        CUE_UTF8
            .replace("INDEX 01 02:30:00", "INDEX 01 00:02:00")
            .replace("INDEX 01 05:00:00", "INDEX 01 00:04:00"),
    )
    .unwrap();

    let out = root.join("tracks");
    let report = split_cue(&cue, &out, |n, t| {
        format!(
            "{:02} {}.flac_placeholder",
            n,
            t.title.as_deref().unwrap_or("unknown")
        )
        .trim_end_matches(".flac_placeholder")
        .to_string()
    })
    .unwrap();

    assert_eq!(report.tracks.len(), 3, "失败轨: {:?}", report.failed);
    assert!(report.failed.is_empty());

    // 时长验收：INDEX 差 <1s（2s / 2s / 2s）
    for t in &report.tracks {
        assert!(
            (t.duration_secs - 2.0).abs() < 1.0,
            "{}: {}",
            t.index,
            t.duration_secs
        );
    }
    // 首轨起点 = 0 → 轨 1 从整轨开头切
    let t1 = decode_to_pcm(&report.tracks[0].dst).unwrap();
    assert_eq!(t1.samples[0], whole.samples[0], "轨 1 必须从采样 0 开始");

    // 标签回读（不信任写入方自证）
    use lofty::prelude::*;
    let tagged = lofty::read_from_path(&report.tracks[0].dst).unwrap();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag()).unwrap();
    assert_eq!(
        tag.get_string(lofty::tag::ItemKey::TrackTitle),
        Some("晴天")
    );
    assert_eq!(
        tag.get_string(lofty::tag::ItemKey::TrackArtist),
        Some("周杰伦")
    );
    assert_eq!(tag.get_string(lofty::tag::ItemKey::TrackNumber), Some("1"));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn cue_pointing_to_missing_audio_is_rejected() {
    let root = uniq_root("missing");
    std::fs::create_dir_all(&root).unwrap();
    let cue = root.join("x.cue");
    std::fs::write(
        &cue,
        "FILE \"ghost.wav\" WAVE\nTRACK 01 AUDIO\nINDEX 01 00:00:00\n",
    )
    .unwrap();
    let err = split_cue(&cue, &root.join("out"), |_, _| "t".to_string()).unwrap_err();
    assert!(err.to_string().contains("不存在"), "{err}");
}

#[test]
fn track_without_title_must_not_inherit_album_title_as_tracktitle() {
    // 稳定审计 B2 回归：轨无 TITLE → TrackTitle 标签**缺席**（专辑名 ≠ 轨名）；
    // 文件名由命名闭包给 "NN"（split_cue 的 sanitize 不受影响）
    let root = uniq_root("notitle");
    let lib = root.join("lib");
    std::fs::create_dir_all(&lib).unwrap();

    let spec = PcmSpec {
        channels: 1,
        sample_rate: 44100,
        bits_per_sample: 16,
    };
    write_pcm(
        &lib.join("a.wav"),
        LosslessFormat::Wav,
        &sine(spec, 3.0, 440.0, 0.5),
    )
    .unwrap();
    let cue = lib.join("a.cue");
    std::fs::write(
        &cue,
        "TITLE \"专辑名\"\nPERFORMER \"艺人\"\nFILE \"a.wav\" WAVE\nTRACK 01 AUDIO\nINDEX 01 00:00:00\n",
    )
    .unwrap();

    let out = root.join("out");
    let report = split_cue(&cue, &out, |n, _| format!("{n:02}")).unwrap();
    assert_eq!(report.tracks.len(), 1);

    use lofty::prelude::*;
    let tagged = lofty::read_from_path(&report.tracks[0].dst).unwrap();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag()).unwrap();
    assert_eq!(
        tag.get_string(lofty::tag::ItemKey::TrackTitle),
        None,
        "轨无 TITLE 时 TrackTitle 必须缺席（专辑名不得冒充轨名）"
    );
    assert_eq!(
        tag.get_string(lofty::tag::ItemKey::TrackArtist),
        Some("艺人")
    );
    std::fs::remove_dir_all(&root).ok();
}

/// D21「失败进 quarantine」：单轨写入失败时，输出目录里已存在的同名产物/占位
/// 物必须移入 `.mf-quarantine/`，且**不得中断**后续轨切分。修复前
/// `encode_pcm` / `write_track_tags` 的错误用 `?` 直接中断整个整轨切分，
/// 已写出的半成品还留在输出目录冒充有效分轨。
#[test]
fn failed_track_is_quarantined_and_split_continues() {
    let root = uniq_root("quarantine");
    let lib = root.join("lib");
    std::fs::create_dir_all(&lib).unwrap();

    let spec = PcmSpec {
        channels: 2,
        sample_rate: 44100,
        bits_per_sample: 16,
    };
    let whole = sine(spec, 6.0, 440.0, 0.8);
    write_pcm(&lib.join("album.wav"), LosslessFormat::Wav, &whole).unwrap();

    let cue = lib.join("album.cue");
    std::fs::write(
        &cue,
        CUE_UTF8
            .replace("INDEX 01 02:30:00", "INDEX 01 00:02:00")
            .replace("INDEX 01 05:00:00", "INDEX 01 00:04:00"),
    )
    .unwrap();

    let out = root.join("tracks");
    std::fs::create_dir_all(&out).unwrap();
    // 预置：轨 2 的目标路径被目录占用 → 写盘必然失败（跨平台确定）
    std::fs::create_dir_all(out.join("t02.wav")).unwrap();

    let report = split_cue(&cue, &out, |n, _| format!("t{n:02}")).unwrap();

    // 轨 1 / 轨 3 正常产出，轨 2 失败 —— 单轨失败不中断整轨切分
    assert!(
        report.tracks.iter().all(|t| t.index != 2),
        "轨 2 不应进入成功列表: {:?}",
        report.tracks.iter().map(|t| t.index).collect::<Vec<_>>()
    );
    assert_eq!(report.tracks.len(), 2, "成功轨数应为 2（轨 1 与轨 3）");
    assert!(
        report.failed.iter().any(|(n, _)| *n == 2),
        "轨 2 应记入 failed: {:?}",
        report.failed
    );

    // 轨 2 的占位物已隔离，且不在输出目录原位残留
    let (_, q) = report
        .quarantined
        .iter()
        .find(|(n, _)| *n == 2)
        .expect("轨 2 应被隔离进 .mf-quarantine");
    assert_eq!(
        q.parent().unwrap().file_name().unwrap().to_string_lossy(),
        ".mf-quarantine"
    );
    assert!(q.exists(), "隔离物应存在: {}", q.display());
    assert!(!out.join("t02.wav").exists(), "占位物不得留在输出目录原位");

    // 成功轨产物完好
    assert!(out.join("t01.wav").exists(), "轨 1 应正常产出");
    assert!(out.join("t03.wav").exists(), "轨 3 应正常产出（未被中断）");

    std::fs::remove_dir_all(&root).ok();
}

/// P5a 缺口补齐：FLAC 整轨 → FLAC 分轨 e2e（此前仅 WAV 源有 e2e 覆盖）。
/// 验收：源为 FLAC 且未指定 target 时，分轨**保持 FLAC**（claxon 解码 + flacenc
/// 无损重编码），且分轨样本与整轨对应区间**逐样本一致**。
#[test]
fn split_flac_source_produces_flac_tracks_sample_exact() {
    let root = uniq_root("flacsrc");
    let lib = root.join("lib");
    std::fs::create_dir_all(&lib).unwrap();

    let spec = PcmSpec {
        channels: 2,
        sample_rate: 44100,
        bits_per_sample: 16,
    };
    let whole = sine(spec, 4.0, 440.0, 0.7);
    write_pcm(&lib.join("album.flac"), LosslessFormat::Flac, &whole).unwrap();

    let cue = lib.join("album.cue");
    std::fs::write(
        &cue,
        "PERFORMER \"P\"\nTITLE \"A\"\nFILE \"album.flac\" WAVE\n  TRACK 01 AUDIO\n    TITLE \"T1\"\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    TITLE \"T2\"\n    INDEX 01 00:02:00\n",
    )
    .unwrap();

    let out = root.join("tracks");
    let report = split_cue(&cue, &out, |n, t| {
        format!("{:02} {}", n, t.title.as_deref().unwrap_or("unknown"))
    })
    .unwrap();

    assert!(report.failed.is_empty(), "失败轨: {:?}", report.failed);
    assert_eq!(report.tracks.len(), 2);

    // 源为 FLAC 且未指定 target → 分轨保持 FLAC（不降级为 WAV）
    for t in &report.tracks {
        assert_eq!(
            t.dst.extension().and_then(|e| e.to_str()),
            Some("flac"),
            "分轨应保持 FLAC 无损格式: {}",
            t.dst.display()
        );
    }

    // 逐样本一致：分轨拼接 == 整轨
    let ch = spec.channels as usize;
    let boundary = 2 * spec.sample_rate as usize * ch; // 第 2 轨起点 = 2s
    let t1 = decode_to_pcm(&report.tracks[0].dst).unwrap();
    let t2 = decode_to_pcm(&report.tracks[1].dst).unwrap();

    assert_eq!(t1.spec, spec, "分轨 spec 应与整轨一致");
    assert_eq!(t1.samples.len(), boundary, "轨 1 长度应为 2s");
    assert_eq!(
        t2.samples.len(),
        whole.samples.len() - boundary,
        "轨 2 长度应为剩余 2s"
    );
    assert_eq!(t1.samples[..], whole.samples[..boundary], "轨 1 逐样本一致");
    assert_eq!(t2.samples[..], whole.samples[boundary..], "轨 2 逐样本一致");

    std::fs::remove_dir_all(&root).ok();
}

/// P5a 缺口补齐：整轨内嵌封面必须写入**每个分轨**（D21「CUE 元数据流 + 整轨
/// 封面写分轨」）。该路径此前 **0 断言**——`synth` 只能生成纯 PCM 样本，造不出
/// 带封面整轨。本测试在整轨写出后用 lofty 直接嵌入封面，再验证分轨确实继承。
#[test]
fn album_embedded_cover_is_inherited_by_split_tracks() {
    use lofty::config::WriteOptions;
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::prelude::*;
    use lofty::tag::Tag;

    let root = uniq_root("cover");
    let lib = root.join("lib");
    std::fs::create_dir_all(&lib).unwrap();

    let spec = PcmSpec {
        channels: 2,
        sample_rate: 44100,
        bits_per_sample: 16,
    };
    let whole = sine(spec, 3.0, 440.0, 0.6);
    let album = lib.join("album.flac");
    write_pcm(&album, LosslessFormat::Flac, &whole).unwrap();

    // 封面字节：带 PNG 魔数（cue.rs 据此判 MimeType::Png）+ payload。
    // 非真图片，仅用于验证「字节原样从整轨传递到分轨」这一链路。
    let mut png: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend_from_slice(b"fake-png-payload-for-cover-inheritance");

    // 整轨嵌入封面
    {
        let mut tagged = lofty::read_from_path(&album).unwrap();
        let ttype = tagged.primary_tag_type();
        if tagged.tag(ttype).is_none() {
            tagged.insert_tag(Tag::new(ttype));
        }
        let tag = tagged.tag_mut(ttype).unwrap();
        let pic = Picture::unchecked(png.clone())
            .mime_type(MimeType::Png)
            .pic_type(PictureType::CoverFront)
            .build();
        tag.push_picture(pic);
        tagged
            .save_to_path(&album, WriteOptions::default())
            .unwrap();
    }
    let cue = lib.join("album.cue");
    std::fs::write(
        &cue,
        "FILE \"album.flac\" WAVE\n  TRACK 01 AUDIO\n    TITLE \"T1\"\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    TITLE \"T2\"\n    INDEX 01 00:01:00\n",
    )
    .unwrap();

    let out = root.join("tracks");
    let report = split_cue(&cue, &out, |n, t| {
        format!("{:02} {}", n, t.title.as_deref().unwrap_or("unknown"))
    })
    .unwrap();
    assert!(report.failed.is_empty(), "失败轨: {:?}", report.failed);
    assert_eq!(report.tracks.len(), 2);

    // 每个分轨都必须继承整轨封面（此前无测试覆盖）
    for t in &report.tracks {
        let tagged = lofty::read_from_path(&t.dst).unwrap();
        let tag = tagged.primary_tag().or_else(|| tagged.first_tag()).unwrap();
        let pics = tag.pictures();
        assert!(!pics.is_empty(), "分轨应继承整轨封面: {}", t.dst.display());
        assert_eq!(
            pics[0].data(),
            png.as_slice(),
            "分轨封面字节应与整轨一致: {}",
            t.dst.display()
        );
    }

    std::fs::remove_dir_all(&root).ok();
}

/// P5a 验收**负向**：CUE 与音频不匹配时，被拒轨必须**不落盘**并计入 failed——
/// 校验在写盘前完成，绝不产出错误时长的分轨（此前只有正向用例）。
///
/// 场景：2s 整轨 + 轨 2 INDEX 指向 10s（远超音频长度）→
/// 轨 1 命中「下一轨 INDEX 超出整轨长度」、轨 2 命中「采样区间为空」。
#[test]
fn mismatched_cue_rejects_tracks_without_writing() {
    let root = uniq_root("mismatch");
    let lib = root.join("lib");
    std::fs::create_dir_all(&lib).unwrap();

    let spec = PcmSpec {
        channels: 2,
        sample_rate: 44100,
        bits_per_sample: 16,
    };
    let whole = sine(spec, 2.0, 440.0, 0.6);
    write_pcm(&lib.join("a.wav"), LosslessFormat::Wav, &whole).unwrap();

    let cue = lib.join("a.cue");
    std::fs::write(
        &cue,
        "FILE \"a.wav\" WAVE\n  TRACK 01 AUDIO\n    TITLE \"T1\"\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    TITLE \"T2\"\n    INDEX 01 00:10:00\n",
    )
    .unwrap();

    let out = root.join("out");
    let report = split_cue(&cue, &out, |n, _| format!("t{n:02}")).unwrap();

    // 两轨均被拒：无成功分轨
    assert!(report.tracks.is_empty(), "被拒轨不应进入成功列表");
    assert!(
        report
            .failed
            .iter()
            .any(|(n, r)| *n == 1 && r.contains("exceeds whole-track length")),
        "轨 1 应因 INDEX 越界被拒: {:?}",
        report.failed
    );
    assert!(
        report
            .failed
            .iter()
            .any(|(n, r)| *n == 2 && r.contains("sample range is empty")),
        "轨 2 应因空区间被拒: {:?}",
        report.failed
    );

    // 关键：被拒轨绝不落盘（校验在写盘前完成）
    assert!(!out.join("t01.wav").exists(), "被拒轨 1 不得落盘");
    assert!(!out.join("t02.wav").exists(), "被拒轨 2 不得落盘");

    std::fs::remove_dir_all(&root).ok();
}

/// P5a 缺口补齐：BIG5 / Windows-1252 专项解码用例。此前仅 **GBK** 有专项用例，
/// BIG5 与 Windows-125x 只靠 chardetng 通用路径，无回归保护。
/// 字节由 encoding_rs 现算（避免硬编码码表出错）。
#[test]
fn big5_and_windows1252_cue_decode_correctly() {
    use encoding_rs::{BIG5, WINDOWS_1252};

    // BIG5：完整 CUE 文本（多行上下文，贴近真实文件）。
    // 注意：极短中文片段（如仅「晴天」2 字）在字节层面与 GBK 高度重叠，
    // chardetng 无足够统计特征可区分，会误判为 GBK —— 这是编码检测的固有
    // 局限（非实现缺陷），故本用例用真实长度的多行 CUE 验证 BIG5 支持。
    // 必须用**繁体**字：BIG5 字符集不含简体「伦 / 叶」，encoding_rs 会把未映射
    // 字符编码成 `&#NNNN;` 实体（此前误用简体导致断言失败，属用例错误非解码缺陷）
    let cue_big5 = "PERFORMER \"周杰倫\"\r\nTITLE \"葉惠美\"\r\nFILE \"album.wav\" WAVE\r\n  TRACK 01 AUDIO\r\n    TITLE \"晴天\"\r\n    INDEX 01 00:00:00\r\n";
    let (big5_bytes, _, _) = BIG5.encode(cue_big5);
    let text = decode_bytes_to_utf8(&big5_bytes);
    assert!(text.contains("晴天"), "BIG5 解码失败: {text:?}");
    assert!(text.contains("周杰倫"), "BIG5 解码失败: {text:?}");
    assert!(text.contains("葉惠美"), "BIG5 解码失败: {text:?}");

    // Windows-1252：西欧重音字符
    let (cp_bytes, _, _) = WINDOWS_1252.encode("Café");
    let text2 = decode_bytes_to_utf8(&cp_bytes);
    assert!(text2.contains("Café"), "Windows-1252 解码失败: {text2:?}");
}
