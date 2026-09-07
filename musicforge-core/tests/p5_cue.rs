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
