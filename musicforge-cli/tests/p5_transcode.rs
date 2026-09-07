//! P5 CLI 集成：transcode 子命令（跑真二进制）。
//!
//! 覆盖：目录递归收集 + 格式互转 + 回读校验承诺 + 同格式跳过 + JSON 输出。

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use musicforge_core::lossless::{decode_to_pcm, LosslessFormat, PcmSpec};
use musicforge_core::synth::{silence, sine, write_pcm};

fn exe() -> &'static str {
    env!("CARGO_BIN_EXE_musicforge")
}

fn uniq_root(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("mf-p5tr-{tag}-{n}-{seq}-{}", std::process::id()))
}

fn run_cli(args: &[&str]) -> (i32, String) {
    let out = Command::new(exe()).args(args).output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

fn spec(bits: u16, channels: u16) -> PcmSpec {
    PcmSpec {
        channels,
        sample_rate: 44100,
        bits_per_sample: bits,
    }
}

#[test]
fn transcode_dir_recurses_and_roundtrips() {
    let root = uniq_root("dir");
    let lib = root.join("lib");
    std::fs::create_dir_all(lib.join("sub")).unwrap();

    let pcm1 = sine(spec(16, 2), 0.5, 440.0, 0.8);
    let pcm2 = silence(spec(24, 1), 0.5);
    write_pcm(&lib.join("a.wav"), LosslessFormat::Wav, &pcm1).unwrap();
    write_pcm(&lib.join("sub").join("b.wav"), LosslessFormat::Wav, &pcm2).unwrap();
    // 非音频文件不收集
    std::fs::write(lib.join("notes.txt"), b"text").unwrap();

    let out = root.join("out");
    let (code, text) = run_cli(&[
        "transcode",
        lib.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--format",
        "flac",
        "--json",
    ]);
    assert_eq!(code, 0, "{text}");
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        v["ok"].as_u64(),
        Some(2),
        "两个 WAV 应转出两个 FLAC: {text}"
    );
    assert_eq!(v["skipped_same_format"].as_u64(), Some(0));

    // 回读：产物解码必须与源逐样本一致（核心验收）
    let items = v["items"].as_array().unwrap();
    let p1 = items[0]["target"].as_str().unwrap();
    let back1 = decode_to_pcm(std::path::Path::new(p1)).unwrap();
    assert_eq!(back1, pcm1, "回环逐样本一致");
    let p2 = items[1]["target"].as_str().unwrap();
    let back2 = decode_to_pcm(std::path::Path::new(p2)).unwrap();
    assert_eq!(back2, pcm2, "回环逐样本一致");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn transcode_same_format_is_skipped() {
    let root = uniq_root("same");
    let lib = root.join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    write_pcm(
        &lib.join("x.flac"),
        LosslessFormat::Flac,
        &sine(spec(16, 1), 0.2, 440.0, 0.5),
    )
    .unwrap();

    let out = root.join("out");
    let (code, text) = run_cli(&[
        "transcode",
        lib.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--format",
        "flac",
        "--json",
    ]);
    assert_eq!(code, 0, "{text}");
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["skipped_same_format"].as_u64(), Some(1));
    assert_eq!(v["ok"].as_u64(), Some(0));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn transcode_rejects_unknown_format() {
    let root = uniq_root("badfmt");
    let (code, out) = run_cli(&[
        "transcode",
        root.to_str().unwrap(),
        "-o",
        root.join("out").to_str().unwrap(),
        "--format",
        "mp3",
    ]);
    assert_eq!(code, 2, "未知目标格式必须退出码 2: {out}");
    // 两条拒绝路径皆可：clap value_parser 解析层拒绝，或 handler 显式报错
    assert!(
        out.contains("invalid value") || out.contains("未知目标格式"),
        "{out}"
    );
    std::fs::remove_dir_all(&root).ok();
}
