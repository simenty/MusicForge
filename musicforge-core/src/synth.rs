//! 合成波形生成器（D19）：程序生成 PCM → 编码 WAV/FLAC，测试样本版权零风险。

use std::path::Path;

use crate::error::NcmError;
use crate::lossless::{encode_pcm, LosslessFormat, Pcm, PcmSpec};

/// 生成正弦波 PCM（交错多声道）。
///
/// `amplitude` 为峰值（相对满幅 `2^(bits-1)-1` 的比例，0.0–1.0）。
pub fn sine(spec: PcmSpec, seconds: f64, freq_hz: f64, amplitude: f64) -> Pcm {
    let total = (spec.sample_rate as f64 * seconds).round() as usize;
    let full = (1i64 << (spec.bits_per_sample - 1)) - 1;
    let peak = (full as f64 * amplitude.clamp(0.0, 1.0)) as i64;
    let ch = spec.channels.max(1) as usize;
    let mut samples = Vec::with_capacity(total * ch);
    for t in 0..total {
        let v = (peak as f64
            * (2.0 * std::f64::consts::PI * freq_hz * t as f64 / spec.sample_rate as f64).sin())
        .round() as i32;
        for _ in 0..ch {
            samples.push(v);
        }
    }
    Pcm { spec, samples }
}

/// 生成静音 PCM。
pub fn silence(spec: PcmSpec, seconds: f64) -> Pcm {
    let total = (spec.sample_rate as f64 * seconds).round() as usize;
    let ch = spec.channels.max(1) as usize;
    Pcm {
        spec,
        samples: vec![0i32; total * ch],
    }
}

/// 把 PCM 写为指定格式的文件（幂等封装 encode_pcm）。
pub fn write_pcm(path: &Path, format: LosslessFormat, pcm: &Pcm) -> Result<u64, NcmError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    encode_pcm(path, format, pcm)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(bits: u16) -> PcmSpec {
        PcmSpec {
            channels: 1,
            sample_rate: 44100,
            bits_per_sample: bits,
        }
    }

    #[test]
    fn sine_sample_formula_holds() {
        let pcm = sine(spec(16), 0.5, 440.0, 0.5);
        assert_eq!(pcm.sample_count(), 22050);
        // t=0 → sin(0)=0 → 0
        assert_eq!(pcm.samples[0], 0);
        // 峰值不超过振幅
        let full = (1i64 << 15) - 1;
        assert!(pcm
            .samples
            .iter()
            .all(|&s| (s as f64).abs() <= full as f64 * 0.5 + 1.0));
    }

    #[test]
    fn synth_wav_roundtrip_is_sample_exact() {
        let dir = std::env::temp_dir().join(format!("mf-synth-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("t.wav");
        let pcm = sine(spec(16), 0.25, 440.0, 0.5);
        write_pcm(&wav, LosslessFormat::Wav, &pcm).unwrap();
        let back = crate::lossless::decode_to_pcm(&wav).unwrap();
        assert_eq!(back, pcm, "WAV 写读必须逐样本一致");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn synth_flac_roundtrip_is_sample_exact() {
        let dir = std::env::temp_dir().join(format!("mf-synth-fl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let flac = dir.join("t.flac");
        let pcm = sine(spec(24), 0.25, 440.0, 0.5);
        write_pcm(&flac, LosslessFormat::Flac, &pcm).unwrap();
        let back = crate::lossless::decode_to_pcm(&flac).unwrap();
        assert_eq!(back, pcm, "FLAC 写读必须逐样本一致");
        std::fs::remove_dir_all(&dir).ok();
    }
}
