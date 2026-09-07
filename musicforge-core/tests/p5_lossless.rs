//! P5.1：无损转码基座（WAV↔FLAC 采样级精确互转）。
//!
//! 验收口径（对齐 ROADMAP P5a）：
//! - WAV→FLAC→WAV 回环**逐样本一致**（16/24/32 位整数 PCM）；
//! - 探测认魔数不认扩展名；非无损格式不认领；
//! - 浮点 WAV 显式拒绝（本版范围）；
//! - 内置回读校验：产物解码必须与源一致。

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use musicforge_core::lossless::PcmSpec;
use musicforge_core::lossless::{decode_to_pcm, probe_lossless_file, transcode, LosslessFormat};
use musicforge_core::synth::{silence, sine, write_pcm};

fn uniq_root(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("mf-p5-{tag}-{n}-{seq}-{}", std::process::id()))
}

fn spec(bits: u16, channels: u16) -> PcmSpec {
    PcmSpec {
        channels,
        sample_rate: 44100,
        bits_per_sample: bits,
    }
}

fn raw_wav_bytes(bits: u16, data_len: usize) -> Vec<u8> {
    let byte_rate = 44100u32 * channels_of(bits) as u32 * bits as u32 / 8;
    let block_align = channels_of(bits) * bits / 8;
    let data = vec![0u8; data_len];
    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    v.extend_from_slice(b"WAVE");
    v.extend_from_slice(b"fmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&channels_of(bits).to_le_bytes());
    v.extend_from_slice(&44100u32.to_le_bytes());
    v.extend_from_slice(&byte_rate.to_le_bytes());
    v.extend_from_slice(&block_align.to_le_bytes());
    v.extend_from_slice(&bits.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&(data.len() as u32).to_le_bytes());
    v.extend_from_slice(&data);
    v
}

fn channels_of(_bits: u16) -> u16 {
    1
}

#[test]
fn probe_recognizes_magic_not_extension() {
    // WAV 魔数（改扩展名为 .flac 仍应识别为 WAV）
    let dir = uniq_root("probe");
    std::fs::create_dir_all(&dir).unwrap();
    let wav = dir.join("mystery.flac");
    std::fs::write(&wav, raw_wav_bytes(16, 1024)).unwrap();
    assert_eq!(probe_lossless_file(&wav), Some(LosslessFormat::Wav));
    // FLAC 魔数（改扩展名为 .wav 仍应识别为 FLAC）
    let flac = dir.join("mystery2.wav");
    std::fs::write(&flac, b"fLaC\x00\x00\x00\x22").unwrap();
    assert_eq!(probe_lossless_file(&flac), Some(LosslessFormat::Flac));
    // 非无损格式不认领
    let txt = dir.join("n.txt");
    std::fs::write(&txt, b"hello world padding").unwrap();
    assert_eq!(probe_lossless_file(&txt), None);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn wav_to_flac_to_wav_is_sample_exact_16bit() {
    let root = uniq_root("r16");
    let a = root.join("a.wav");
    let b = root.join("mid.flac");
    let c = root.join("back.wav");

    let pcm = sine(spec(16, 2), 0.5, 440.0, 0.8);
    write_pcm(&a, LosslessFormat::Wav, &pcm).unwrap();

    transcode(&a, &b, LosslessFormat::Flac).unwrap();
    transcode(&b, &c, LosslessFormat::Wav).unwrap();

    let back = decode_to_pcm(&c).unwrap();
    assert_eq!(back, pcm, "WAV→FLAC→WAV 必须逐样本一致（16bit 立体声）");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn roundtrip_24bit_sample_exact() {
    let root = uniq_root("deep24");
    let a = root.join("t24.wav");
    let b = root.join("t24.flac");
    let c = root.join("t24-back.wav");
    let pcm = sine(spec(24, 1), 0.3, 523.25, 0.9);
    write_pcm(&a, LosslessFormat::Wav, &pcm).unwrap();
    transcode(&a, &b, LosslessFormat::Flac).unwrap();
    transcode(&b, &c, LosslessFormat::Wav).unwrap();
    assert_eq!(decode_to_pcm(&c).unwrap(), pcm, "24 位回环逐样本一致");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn flac_encode_32bit_limitation_is_explicit() {
    // flacenc 0.5.1 编码器位深上限 ≤25（FLAC 解码支持 32，编码不支持）：
    // 32 位源转 FLAC → 显式 MF-LOSSLESS-FAILED，绝不产出未验证产物
    let root = uniq_root("deep32");
    let a = root.join("t32.wav");
    let b = root.join("t32.flac");
    let pcm = sine(spec(32, 1), 0.3, 523.25, 0.9);
    write_pcm(&a, LosslessFormat::Wav, &pcm).unwrap();
    let err = transcode(&a, &b, LosslessFormat::Flac).unwrap_err();
    assert!(
        err.to_string().contains("bits_per_sample"),
        "32 位 FLAC 编码限制必须显式报错: {err}"
    );
    assert!(!b.exists(), "失败后不得留下未验证产物");
    assert!(a.exists(), "源文件永不修改");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn flac_to_wav_to_flac_is_sample_exact() {
    let root = uniq_root("rev");
    let a = root.join("a.flac");
    let b = root.join("mid.wav");
    let c = root.join("back.flac");
    let pcm = sine(spec(16, 2), 0.4, 261.63, 0.7);
    write_pcm(&a, LosslessFormat::Flac, &pcm).unwrap();
    transcode(&a, &b, LosslessFormat::Wav).unwrap();
    transcode(&b, &c, LosslessFormat::Flac).unwrap();
    assert_eq!(decode_to_pcm(&c).unwrap(), pcm);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn float_wav_is_rejected_explicitly() {
    let root = uniq_root("float");
    std::fs::create_dir_all(&root).unwrap();
    let wav = root.join("f.wav");
    // 手工构造 IEEE float WAV（format=3）
    let data = vec![0u8; 256];
    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    v.extend_from_slice(b"WAVE");
    v.extend_from_slice(b"fmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&44100u32.to_le_bytes());
    v.extend_from_slice(&176400u32.to_le_bytes());
    v.extend_from_slice(&4u16.to_le_bytes());
    v.extend_from_slice(&32u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&(data.len() as u32).to_le_bytes());
    v.extend_from_slice(&data);
    std::fs::write(&wav, &v).unwrap();

    let err = decode_to_pcm(&wav).unwrap_err();
    assert!(
        err.to_string().contains("浮点"),
        "浮点 WAV 必须显式拒绝: {err}"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn transcode_verifies_and_reports_sample_count() {
    let root = uniq_root("verify");
    let a = root.join("s.wav");
    let b = root.join("s.flac");
    let pcm = silence(spec(16, 1), 1.0);
    write_pcm(&a, LosslessFormat::Wav, &pcm).unwrap();
    let out = transcode(&a, &b, LosslessFormat::Flac).unwrap();
    assert!(out.verified);
    assert_eq!(out.sample_count, 44100);
    assert!(b.exists());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn silence_sine_sizes_are_consistent() {
    let pcm = silence(spec(16, 2), 1.0);
    assert_eq!(pcm.samples.len(), 44100 * 2);
    let pcm2 = sine(spec(16, 2), 1.0, 440.0, 0.5);
    assert_eq!(pcm2.samples.len(), 44100 * 2);
}

#[test]
fn transcode_refuses_to_overwrite_existing_dst() {
    // 稳定审计 B1 回归：dst 已存在 → MF-OUTPUT-EXISTS，且**原文件字节不变**
    let root = uniq_root("exists");
    let a = root.join("a.wav");
    let dst = root.join("out.wav");
    write_pcm(&a, LosslessFormat::Wav, &sine(spec(16, 1), 0.3, 440.0, 0.5)).unwrap();
    // 预置一个「既有产物」（内容为非音频哨兵字节）
    std::fs::write(&dst, b"PRECIOUS-EXISTING-CONTENT").unwrap();

    let err = transcode(&a, &dst, LosslessFormat::Flac).unwrap_err();
    assert_eq!(err.mf_code(), "MF-OUTPUT-EXISTS", "{err}");
    assert_eq!(
        std::fs::read(&dst).unwrap(),
        b"PRECIOUS-EXISTING-CONTENT",
        "既有文件必须原样保留"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn transcode_to_fresh_dst_still_works_after_guard() {
    // 守卫不得误伤正常路径：fresh dst 照常转码
    let root = uniq_root("fresh");
    let a = root.join("a.wav");
    let dst = root.join("out.flac");
    let pcm = sine(spec(16, 1), 0.3, 440.0, 0.5);
    write_pcm(&a, LosslessFormat::Wav, &pcm).unwrap();
    let o = transcode(&a, &dst, LosslessFormat::Flac).unwrap();
    assert!(o.verified && dst.exists());
    std::fs::remove_dir_all(&root).ok();
}
