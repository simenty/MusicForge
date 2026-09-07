//! 无损转码（P5a）：WAV↔FLAC 采样级精确互转。
//!
//! 设计要点（对齐 ROADMAP P5a / D1 / D7）：
//!
//! - **纯 Rust**：WAV 读写走 [`hound`]，FLAC 解码走 [`claxon`]，FLAC 编码走
//!   [`flacenc`]——零 C 依赖、零网络（CI 硬闸覆盖）；
//! - **采样级无损**：FLAC 按定义无损；转码后内置**回读校验**（目标文件解码
//!   出的样本必须与源逐样本一致），任何不一致 = `MF-LOSSLESS-FAILED` 显式失败，
//!   绝不产出未验证的产物（G5 教训：兜底伪装成功是最大敌）；
//! - **探测认魔数不认扩展名**（G5 同款纪律）：`RIFF…WAVE` 与 `fLaC`；
//! - PCM 统一为交错 `i32`（hound/claxon 原生形态），位深 16/24/32 整数；
//!   浮点 WAV（format 3）显式拒绝（本版范围，不做浮点归一化的有损争论）；
//! - **永不修改源文件**：只写 `dst`。

use std::path::Path;

use crate::error::NcmError;

/// 无损格式（本版支持集合）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LosslessFormat {
    Wav,
    Flac,
}

impl LosslessFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Flac => "flac",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "wav" | "wave" => Some(Self::Wav),
            "flac" => Some(Self::Flac),
            _ => None,
        }
    }
}

/// PCM 规格。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmSpec {
    pub channels: u16,
    pub sample_rate: u32,
    /// 整数位深（16/24/32）
    pub bits_per_sample: u16,
}

/// 交错 PCM 样本（`samples[c0,t0], samples[c1,t0], samples[c0,t1], …`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcm {
    pub spec: PcmSpec,
    pub samples: Vec<i32>,
}

impl Pcm {
    pub fn sample_count(&self) -> usize {
        self.samples.len() / self.spec.channels.max(1) as usize
    }
}

/// 按魔数探测无损格式（`RIFF…WAVE` / `fLaC`）。
pub fn probe_lossless(header: &[u8]) -> Option<LosslessFormat> {
    if header.len() >= 12 && &header[0..4] == b"RIFF" && &header[8..12] == b"WAVE" {
        return Some(LosslessFormat::Wav);
    }
    if header.len() >= 4 && &header[0..4] == b"fLaC" {
        return Some(LosslessFormat::Flac);
    }
    None
}

/// 便捷入口：读文件头后探测。
pub fn probe_lossless_file(path: &Path) -> Option<LosslessFormat> {
    let mut buf = [0u8; 12];
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut read = 0usize;
    while read < buf.len() {
        match f.read(&mut buf[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(_) => return None,
        }
    }
    probe_lossless(&buf[..read])
}

/// 解码任意支持的无损格式到 PCM。
pub fn decode_to_pcm(path: &Path) -> Result<Pcm, NcmError> {
    let header = read_header_min(path)?;
    match probe_lossless(&header) {
        Some(LosslessFormat::Wav) => decode_wav(path),
        Some(LosslessFormat::Flac) => decode_flac(path),
        None => Err(NcmError::Lossless(format!(
            "{}: 不是支持的无损格式（仅 WAV/FLAC）",
            path.display()
        ))),
    }
}

/// 解码 WAV（整数 PCM 16/24/32 位；浮点 WAV 显式拒绝）。
fn decode_wav(path: &Path) -> Result<Pcm, NcmError> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|e| NcmError::Lossless(format!("WAV 打开失败: {e}")))?;
    let spec = reader.spec();
    if spec.sample_format == hound::SampleFormat::Float {
        return Err(NcmError::Lossless(format!(
            "{}: 浮点 WAV 本版不支持（仅整数 PCM 16/24/32 位）",
            path.display()
        )));
    }
    let spec = PcmSpec {
        channels: spec.channels,
        sample_rate: spec.sample_rate,
        bits_per_sample: spec.bits_per_sample,
    };
    let mut samples = Vec::new();
    for s in reader.samples::<i32>() {
        let s = s.map_err(|e| NcmError::Lossless(format!("WAV 样本读取失败: {e}")))?;
        samples.push(s);
    }
    if samples.is_empty() {
        return Err(NcmError::Lossless(format!(
            "{}: WAV 无样本数据",
            path.display()
        )));
    }
    Ok(Pcm { spec, samples })
}

/// 解码 FLAC（claxon）。
fn decode_flac(path: &Path) -> Result<Pcm, NcmError> {
    let mut reader = claxon::FlacReader::open(path)
        .map_err(|e| NcmError::Lossless(format!("FLAC 打开失败: {e}")))?;
    let si = reader.streaminfo();
    let spec = PcmSpec {
        channels: si.channels as u16,
        sample_rate: si.sample_rate,
        bits_per_sample: si.bits_per_sample as u16,
    };
    let mut samples = Vec::new();
    for s in reader.samples() {
        let s = s.map_err(|e| NcmError::Lossless(format!("FLAC 样本读取失败: {e}")))?;
        samples.push(s);
    }
    if samples.is_empty() {
        return Err(NcmError::Lossless(format!(
            "{}: FLAC 无样本数据",
            path.display()
        )));
    }
    Ok(Pcm { spec, samples })
}

/// 编码 PCM 到目标格式文件，返回写入字节数。
pub fn encode_pcm(path: &Path, format: LosslessFormat, pcm: &Pcm) -> Result<u64, NcmError> {
    match format {
        LosslessFormat::Wav => encode_wav(path, pcm),
        LosslessFormat::Flac => encode_flac(path, pcm),
    }
}

fn encode_wav(path: &Path, pcm: &Pcm) -> Result<u64, NcmError> {
    let spec = hound::WavSpec {
        channels: pcm.spec.channels,
        sample_rate: pcm.spec.sample_rate,
        bits_per_sample: pcm.spec.bits_per_sample,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|e| NcmError::Lossless(format!("WAV 创建失败: {e}")))?;
    for s in &pcm.samples {
        writer
            .write_sample(*s)
            .map_err(|e| NcmError::Lossless(format!("WAV 写入失败: {e}")))?;
    }
    writer
        .finalize()
        .map_err(|e| NcmError::Lossless(format!("WAV 收尾失败: {e}")))?;
    file_len(path)
}

fn encode_flac(path: &Path, pcm: &Pcm) -> Result<u64, NcmError> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, e)| NcmError::Lossless(format!("编码器配置校验失败: {e}")))?;
    let source = flacenc::source::MemSource::from_samples(
        &pcm.samples,
        pcm.spec.channels as usize,
        pcm.spec.bits_per_sample as usize,
        pcm.spec.sample_rate as usize,
    );
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|e| NcmError::Lossless(format!("FLAC 编码失败: {e}")))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| NcmError::Lossless(format!("FLAC 比特流写出失败: {e}")))?;
    std::fs::write(path, sink.as_slice())?;
    file_len(path)
}

fn file_len(path: &Path) -> Result<u64, NcmError> {
    Ok(std::fs::metadata(path)?.len())
}

fn read_header_min(path: &Path) -> Result<Vec<u8>, NcmError> {
    use std::io::Read;
    let mut buf = vec![0u8; 12];
    let mut f = std::fs::File::open(path)?;
    let mut read = 0usize;
    while read < buf.len() {
        match f.read(&mut buf[read..])? {
            0 => break,
            n => read += n,
        }
    }
    buf.truncate(read);
    Ok(buf)
}

/// 转码结果（含内置回读校验的统计）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscodeOutcome {
    pub dst: std::path::PathBuf,
    pub bytes_written: u64,
    pub sample_count: usize,
    /// 回读校验通过（逐样本一致）——恒为 true 时才返回 Ok
    pub verified: bool,
}

/// 无损转码：解码源 → 编码目标 → **回读校验（逐样本一致）**。
///
/// 校验失败 = 目标文件删除 + `MF-LOSSLESS-FAILED`（绝不留下未验证产物）。
/// 源文件**永不修改**；dst 已存在时覆盖前先删除（调用方负责冲突策略）。
pub fn transcode(
    src: &Path,
    dst: &Path,
    target: LosslessFormat,
) -> Result<TranscodeOutcome, NcmError> {
    let pcm = decode_to_pcm(src)?;
    let bytes_written = encode_pcm(dst, target, &pcm)?;
    // 内置回读校验：目标解码后必须与源逐样本一致
    let out_pcm = decode_to_pcm(dst)?;
    if out_pcm.spec != pcm.spec || out_pcm.samples != pcm.samples {
        let _ = std::fs::remove_file(dst);
        return Err(NcmError::Lossless(format!(
            "回读校验失败：{} 的解码结果与源不一致（已删除未验证产物）",
            dst.display()
        )));
    }
    Ok(TranscodeOutcome {
        dst: dst.to_path_buf(),
        bytes_written,
        sample_count: pcm.sample_count(),
        verified: true,
    })
}
