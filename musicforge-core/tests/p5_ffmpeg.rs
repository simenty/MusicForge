//! P5b.1：FFmpeg sidecar——五级探测、有损导出预设、升级拦截、回读校验。
//!
//! 运行环境无 ffmpeg 时相关用例**自跳过**（CI 三平台镜像均预装 ffmpeg，
//! 本地无 ffmpeg 时也不阻塞其余测试）。

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use musicforge_core::error::NcmError;
use musicforge_core::ffmpeg::{classify_source, Ffmpeg, SourceClass};
use musicforge_core::lossless::PcmSpec;
use musicforge_core::lossless::{transcode, LosslessFormat};
use musicforge_core::synth::{sine, write_pcm};

fn uniq_root(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("mf-ff-{tag}-{n}-{seq}-{}", std::process::id()))
}

fn ff() -> Option<Ffmpeg> {
    Ffmpeg::find(None).ok()
}

fn require_ff() -> Option<Ffmpeg> {
    match ff() {
        Some(f) => Some(f),
        None => {
            eprintln!(
                "SKIP：本机未找到 ffmpeg（MF-FFMPEG-MISSING 属预期行为，跳过 ffmpeg 依赖用例）"
            );
            None
        }
    }
}

#[test]
fn missing_explicit_path_is_explicit_error() {
    let bogus = uniq_root("no-such-dir").join("ffmpeg.exe");
    let err = Ffmpeg::find(Some(&bogus)).unwrap_err();
    assert!(
        matches!(err, NcmError::FfmpegMissing { .. }),
        "显式无效路径必须报 FfmpegMissing: {err}"
    );
    assert_eq!(err.mf_code(), "MF-FFMPEG-MISSING");
}

#[test]
fn source_classification_lossless_vs_lossy() {
    let root = uniq_root("cls");
    std::fs::create_dir_all(&root).unwrap();
    let wav = root.join("s.wav");
    write_pcm(&wav, LosslessFormat::Wav, &sine(spec(), 0.2, 440.0, 0.5)).unwrap();
    assert_eq!(classify_source(&wav), Some(SourceClass::Lossless));
    // mp3 扩展名（即使内容非音频，分类按扩展名认领——交给 ffmpeg 去报错）
    let mp3 = root.join("s.mp3");
    std::fs::write(&mp3, b"ID3fake-for-classification").unwrap();
    assert_eq!(classify_source(&mp3), Some(SourceClass::Lossy));
    // 未知扩展不认领
    assert_eq!(classify_source(&root.join("x.txt")), None);
    std::fs::remove_dir_all(&root).ok();
}

fn spec() -> PcmSpec {
    PcmSpec {
        channels: 2,
        sample_rate: 44100,
        bits_per_sample: 16,
    }
}

#[test]
fn upgrade_intercept_blocks_lossy_to_lossless() {
    let root = uniq_root("upgrade");
    std::fs::create_dir_all(&root).unwrap();
    let src = root.join("fake.mp3");
    std::fs::write(&src, b"ID3fake").unwrap();
    let dst = root.join("out.flac");

    let err = transcode(&src, &dst, LosslessFormat::Flac).unwrap_err();
    assert_eq!(err.mf_code(), "MF-LOSSY-TO-LOSSLESS", "{err}");
    assert!(!dst.exists());
    std::fs::remove_dir_all(&root).ok();
}

// ---- 以下用例需要真实 ffmpeg ----

#[test]
fn wav_to_mp3_bitrate_and_magic_and_duration() {
    let Some(ff) = require_ff() else { return };
    let root = uniq_root("mp3");
    std::fs::create_dir_all(&root).unwrap();
    let wav = root.join("s.wav");
    let mp3 = root.join("s.mp3");
    // 5s 立体声 → MP3 320
    write_pcm(&wav, LosslessFormat::Wav, &sine(spec(), 5.0, 440.0, 0.8)).unwrap();

    let bytes = ff
        .export_lossy(&wav, &mp3, musicforge_core::ffmpeg::LossyPreset::Mp3)
        .unwrap();
    assert!(mp3.exists());

    // 码率验收：总码率 ∈ [310, 330] kbps（P5b 验收断言）
    let dur = ff.probe_duration(&wav).unwrap();
    let kbps = bytes * 8 / dur as u64 / 1000;
    assert!(
        (310..=330).contains(&kbps),
        "MP3 320 预设总码率应 ∈[310,330]kbps，实测 {kbps}"
    );

    // 回读校验已内置（时长差 <1s + 魔数）；此处复核魔数
    let data = std::fs::read(&mp3).unwrap();
    assert!(data.starts_with(b"ID3") || data[0] == 0xFF, "MP3 容器魔数");

    // 升级拦截：mp3 → flac 被拦（真有损源）
    let flac = root.join("s.flac");
    let err = transcode(&mp3, &flac, LosslessFormat::Flac).unwrap_err();
    assert_eq!(err.mf_code(), "MF-LOSSY-TO-LOSSLESS");
    assert!(!flac.exists());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn aac_and_opus_container_magic() {
    let Some(ff) = require_ff() else { return };
    let root = uniq_root("ao");
    std::fs::create_dir_all(&root).unwrap();
    let wav = root.join("s.wav");
    write_pcm(&wav, LosslessFormat::Wav, &sine(spec(), 2.0, 440.0, 0.8)).unwrap();

    let m4a = root.join("s.m4a");
    ff.export_lossy(&wav, &m4a, musicforge_core::ffmpeg::LossyPreset::Aac)
        .unwrap();
    let d = std::fs::read(&m4a).unwrap();
    assert!(&d[4..8] == b"ftyp", "M4A 容器魔数（ftyp @4）");

    let opus = root.join("s.opus");
    ff.export_lossy(&wav, &opus, musicforge_core::ffmpeg::LossyPreset::Opus)
        .unwrap();
    let d = std::fs::read(&opus).unwrap();
    assert!(d.starts_with(b"OggS"), "Opus 容器魔数（OggS）");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn lossy_to_lossless_with_explicit_flag_succeeds() {
    let Some(ff) = require_ff() else { return };
    let root = uniq_root("flag");
    std::fs::create_dir_all(&root).unwrap();
    let wav = root.join("s.wav");
    write_pcm(&wav, LosslessFormat::Wav, &sine(spec(), 2.0, 440.0, 0.8)).unwrap();
    let mp3 = root.join("s.mp3");
    ff.export_lossy(&wav, &mp3, musicforge_core::ffmpeg::LossyPreset::Mp3)
        .unwrap();

    // 放行后的有损→无损（用户显式承担伪升级）
    let flac = root.join("s.flac");
    ff.export_custom(&mp3, &flac, &["-codec:a", "flac"])
        .unwrap();
    assert!(flac.exists(), "放行后应产出 FLAC");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn export_lossy_refuses_to_overwrite_existing_dst() {
    // 稳定审计 B11 回归：dst 已存在 → MF-OUTPUT-EXISTS（ffmpeg -y 的覆盖语义被守卫拦截）
    let Some(ff) = require_ff() else { return };
    let root = uniq_root("ovw");
    std::fs::create_dir_all(&root).unwrap();
    let wav = root.join("s.wav");
    write_pcm(&wav, LosslessFormat::Wav, &sine(spec(), 1.0, 440.0, 0.5)).unwrap();
    let dst = root.join("out.mp3");
    std::fs::write(&dst, b"PRECIOUS").unwrap();

    let err = ff
        .export_lossy(&wav, &dst, musicforge_core::ffmpeg::LossyPreset::Mp3)
        .unwrap_err();
    assert_eq!(err.mf_code(), "MF-OUTPUT-EXISTS", "{err}");
    assert_eq!(
        std::fs::read(&dst).unwrap(),
        b"PRECIOUS",
        "既有文件原样保留"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn upgrade_bypass_wav_output_keeps_24bit_depth() {
    // 稳定审计 B3 回归：升级放行路径 WAV 输出位深 = 24（pcm_s24le），
    // 不得静默降为 16 位
    let Some(ff) = require_ff() else { return };
    let root = uniq_root("b3");
    std::fs::create_dir_all(&root).unwrap();
    let wav = root.join("s.wav");
    write_pcm(&wav, LosslessFormat::Wav, &sine(spec(), 2.0, 440.0, 0.8)).unwrap();
    let mp3 = root.join("s.mp3");
    ff.export_lossy(&wav, &mp3, musicforge_core::ffmpeg::LossyPreset::Mp3)
        .unwrap();

    let back = root.join("s24.wav");
    ff.export_custom(&mp3, &back, &["-codec:a", "pcm_s24le"])
        .unwrap();
    let pcm = musicforge_core::lossless::decode_to_pcm(&back).unwrap();
    assert_eq!(pcm.spec.bits_per_sample, 24, "放行路径 WAV 输出必须 24 位");
    std::fs::remove_dir_all(&root).ok();
}
