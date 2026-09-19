//! DSD（DSF / DFF）解析与 **PCM 转换**（P6.3）。
//!
//! 定位：symphonia 不支持 DSD；本项目**不做** DoP/独占原生输出（路线图项），
//! 而是把 DSD 位流多级抽取为 88.2kHz f32 PCM，接入现有播放引擎（**非原生路径**）。
//! 音质为"常规 PCM 转换"水平（8 位求和 + box 抽取，非发烧级 FIR）——如实标注。
//!
//! 支持：DSF（块交错）与 DFF（字节交错）；DSD64/128/256 按位率自动适配抽取因子。
//! DSF 的 ID3 标签由 lofty 另行读取（本模块只关心音频结构）。

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use crate::error::NcmError;

/// PCM 输出采样率（44.1k 家族，普遍被音频设备支持；DSD64 = 32:1 抽取）。
pub const OUT_RATE: u32 = 88_200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DsdKind {
    /// DSF：块交错（每块内每声道连续 `block_size` 字节）
    Dsf,
    /// DFF：字节级交错（每字节按声道轮转）
    Dff,
}

/// 头信息（probe 结果；不读全量数据）。
#[derive(Debug, Clone, PartialEq)]
pub struct DsdInfo {
    pub kind: DsdKind,
    pub channels: u32,
    /// DSD 位率（bits/s；DSD64 = 2_822_400）
    pub dsd_rate: u32,
    /// 时长（每声道位数 / 位率）
    pub duration_secs: f64,
}

/// 内部布局（probe 与 Reader 共用）。
struct Layout {
    kind: DsdKind,
    channels: u32,
    dsd_rate: u32,
    data_start: u64,
    /// 每声道的数据字节数
    data_bytes_per_ch: u64,
    /// DSF 块大小（字节/声道）；DFF 为 0
    block_size: u64,
}

fn bad(msg: String) -> NcmError {
    NcmError::Lossless(msg)
}

fn u32_le(b: &[u8]) -> u32 {
    u32::from_le_bytes(b.try_into().unwrap_or([0; 4]))
}
fn u64_le(b: &[u8]) -> u64 {
    u64::from_le_bytes(b.try_into().unwrap_or([0; 8]))
}
fn u16_be(b: &[u8]) -> u16 {
    u16::from_be_bytes(b.try_into().unwrap_or([0; 2]))
}
fn u32_be(b: &[u8]) -> u32 {
    u32::from_be_bytes(b.try_into().unwrap_or([0; 4]))
}
fn u64_be(b: &[u8]) -> u64 {
    u64::from_be_bytes(b.try_into().unwrap_or([0; 8]))
}

/// 解析头信息（不读全量数据）。损坏 / 非 DSD → `Err`。
pub fn probe_dsd(path: &Path) -> Result<DsdInfo, NcmError> {
    let l = open_layout(path)?;
    Ok(DsdInfo {
        kind: l.kind,
        channels: l.channels,
        dsd_rate: l.dsd_rate,
        duration_secs: (l.data_bytes_per_ch as f64 * 8.0) / l.dsd_rate as f64,
    })
}

fn open_layout(path: &Path) -> Result<Layout, NcmError> {
    let mut r = BufReader::new(File::open(path)?);
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic)?;
    match &magic {
        b"DSD " => parse_dsf(&mut r),
        b"FRM8" => parse_dff(&mut r),
        _ => Err(bad(format!("不是 DSF/DFF 文件（魔数 {magic:?}）"))),
    }
}

/// DSF：12 字节 chunk 头（"id " + 8B LE 长度，长度含头 12 字节）。
fn parse_dsf<R: Read + Seek>(r: &mut R) -> Result<Layout, NcmError> {
    // 已读 magic（4/28 头的一部分）：跳到 28 开始遍历 chunk
    r.seek(SeekFrom::Start(28))?;
    let mut channels = 0u32;
    let mut dsd_rate = 0u32;
    let mut block_size = 0u32;
    let mut bits_per_ch = 0u64;
    let mut data_start = 0u64;
    let mut data_bytes = 0u64;
    let mut header = [0u8; 12];
    loop {
        match r.read_exact(&mut header) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }
        let size = u64_le(&header[4..12]);
        if size < 12 {
            return Err(bad("DSF chunk 长度非法".to_string()));
        }
        let body = r.stream_position()?;
        match &header[0..4] {
            b"fmt " => {
                let mut b = [0u8; 40];
                r.read_exact(&mut b)?;
                channels = u32_le(&b[12..16]);
                dsd_rate = u32_le(&b[16..20]);
                bits_per_ch = u64_le(&b[24..32]);
                block_size = u32_le(&b[32..36]);
            }
            b"data" => {
                data_start = body;
                data_bytes = size - 12;
            }
            _ => {}
        }
        if &header[0..4] == b"data" {
            break; // data 之后不再需要（尾部 ID3 由标签层处理）
        }
        r.seek(SeekFrom::Start(body + size - 12))?;
    }
    if channels == 0 || dsd_rate == 0 || block_size == 0 || data_bytes == 0 {
        return Err(bad("DSF 头不完整（fmt/data 缺失）".to_string()));
    }
    let per_ch = (data_bytes / channels as u64).min(bits_per_ch.div_ceil(8));
    Ok(Layout {
        kind: DsdKind::Dsf,
        channels,
        dsd_rate,
        data_start,
        data_bytes_per_ch: per_ch,
        block_size: block_size as u64,
    })
}

/// DFF：12 字节 chunk 头（"id " + 8B BE 长度，**不含头**；chunk 按偶字节对齐 pad）。
fn parse_dff<R: Read + Seek>(r: &mut R) -> Result<Layout, NcmError> {
    // 已读 "FRM8"：+8B BE 表单长度 + "DSD " 表单类型
    let mut fh = [0u8; 12];
    r.read_exact(&mut fh)?;
    if &fh[8..12] != b"DSD " {
        return Err(bad("FRM8 不是 DSD 表单".to_string()));
    }
    let mut channels = 0u32;
    let mut dsd_rate = 0u32;
    let mut data_start = 0u64;
    let mut data_bytes = 0u64;
    let mut header = [0u8; 12];
    loop {
        match r.read_exact(&mut header) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }
        let size = u64_be(&header[4..12]);
        let body = r.stream_position()?;
        match &header[0..4] {
            b"PROP" => {
                let mut snd = [0u8; 4];
                r.read_exact(&mut snd)?;
                let prop_end = body + size;
                let mut h2 = [0u8; 12];
                while r.stream_position()? + 12 <= prop_end {
                    r.read_exact(&mut h2)?;
                    let sz2 = u64_be(&h2[4..12]);
                    let b2 = r.stream_position()?;
                    match &h2[0..4] {
                        b"FS  " => {
                            let mut v = [0u8; 4];
                            r.read_exact(&mut v)?;
                            dsd_rate = u32_be(&v);
                        }
                        b"CHNL" => {
                            let mut n = [0u8; 2];
                            r.read_exact(&mut n)?;
                            channels = u16_be(&n) as u32;
                        }
                        b"CMPR" => {
                            let mut c = [0u8; 4];
                            r.read_exact(&mut c)?;
                            if &c != b"DSD " {
                                return Err(bad("DFF 使用了不支持的压缩方式".to_string()));
                            }
                        }
                        _ => {}
                    }
                    r.seek(SeekFrom::Start(b2 + sz2 + (sz2 & 1)))?;
                }
                r.seek(SeekFrom::Start(prop_end + (size & 1)))?;
            }
            b"DSD " => {
                data_start = body;
                data_bytes = size;
                break;
            }
            _ => {
                r.seek(SeekFrom::Start(body + size + (size & 1)))?;
            }
        }
    }
    if channels == 0 || dsd_rate == 0 || data_bytes == 0 {
        return Err(bad("DFF 头不完整（FS/CHNL/DSD 缺失）".to_string()));
    }
    Ok(Layout {
        kind: DsdKind::Dff,
        channels,
        dsd_rate,
        data_start,
        data_bytes_per_ch: data_bytes / channels as u64,
        block_size: 0,
    })
}

/// 低通 FIR 设计（Blackman 窗 sinc，DC 增益归一化为 1）。
///
/// - `n` 抽头数（必须是 2×factor 的整数倍——`48 × factor` 起步）；
/// - `fc` 通带截止（Hz，本项目固定 40kHz：覆盖音频带并给 88.2kHz 输出留过渡）；
/// - `fs` 输入率（= OUT_RATE × factor）。
///
/// 相比 box（sinc 第一旁瓣仅 −13dB），Blackman 窗阻带约 −74dB——
/// DSD 的**带外噪声不会被折叠回音频带**（这是音质升级的核心）。
fn design_lowpass(n: usize, fc: f64, fs: f64) -> Vec<f32> {
    let m = (n - 1) as f64;
    let w0 = 2.0 * std::f64::consts::PI * fc / fs;
    let mut c = vec![0f64; n];
    let mut sum = 0.0f64;
    for (i, v) in c.iter_mut().enumerate() {
        let x = i as f64 - m / 2.0;
        let sinc = if x.abs() < 1e-12 {
            1.0
        } else {
            (w0 * x).sin() / (w0 * x)
        };
        let t = 2.0 * std::f64::consts::PI * i as f64 / m;
        let win = 0.42 - 0.5 * t.cos() + 0.08 * (2.0 * t).cos();
        *v = sinc * win;
        sum += *v;
    }
    let g = 1.0 / sum;
    c.iter().map(|v| (v * g) as f32).collect()
}

/// 每声道抽取器（P6.7）：MSB-first 逐位 → 8:1 求和（±1 归一）→
/// **多相 FIR 低通抽取**到 [`OUT_RATE`]（每 `factor` 个中间样本出一个输出帧）。
///
/// 抽头数 `48 × factor`（factor=4→192 / 8→384 / 16→768）：过渡带宽度按输入率
/// 归一化后保持不变（≈8kHz @88.2k 输出域），阻带 ~74dB。
struct DsdDecimator {
    bit_sum: i32,
    bit_cnt: u32,
    /// 每输出帧的中间样本数
    factor: u32,
    /// 距上次输出的中间样本数
    mid_cnt: u32,
    /// 环形延时线（输入域样本）
    hist: Vec<f32>,
    /// 最老样本位置（推进后即指向它）
    hist_pos: usize,
    /// FIR 系数（与 hist 时间序对齐）
    coeffs: Vec<f32>,
}

impl DsdDecimator {
    fn new(factor: u32) -> Self {
        let n = (48 * factor) as usize;
        let coeffs = design_lowpass(n, 40_000.0, OUT_RATE as f64 * factor as f64);
        Self {
            bit_sum: 0,
            bit_cnt: 0,
            factor,
            mid_cnt: 0,
            hist: vec![0.0; n],
            hist_pos: 0,
            coeffs,
        }
    }

    fn push_byte(&mut self, byte: u8, out: &mut Vec<f32>) {
        let mut b = byte;
        for _ in 0..8 {
            self.bit_sum += if b & 0x80 != 0 { 1 } else { -1 };
            b <<= 1;
            self.bit_cnt += 1;
            if self.bit_cnt == 8 {
                // 8:1：一个中间样本（值域 ±1）
                let mid = self.bit_sum as f32 / 8.0;
                self.bit_sum = 0;
                self.bit_cnt = 0;
                self.push_mid(mid, out);
            }
        }
    }

    fn push_mid(&mut self, mid: f32, out: &mut Vec<f32>) {
        let n = self.hist.len();
        self.hist[self.hist_pos] = mid;
        self.hist_pos = (self.hist_pos + 1) % n;
        self.mid_cnt += 1;
        if self.mid_cnt < self.factor {
            return;
        }
        self.mid_cnt = 0;
        // 卷积：hist 按时间序读取（hist_pos 指向最老），与 coeffs 对齐
        let mut acc = 0.0f32;
        for (k, c) in self.coeffs.iter().enumerate() {
            acc += self.hist[(self.hist_pos + k) % n] * c;
        }
        out.push(acc);
    }
}

/// DSD 流式读取 + PCM 转换（interleaved f32 @ [`OUT_RATE`]）。
pub struct DsdReader {
    kind: DsdKind,
    pub channels: u32,
    pub dsd_rate: u32,
    pub out_rate: u32,
    data_bytes_per_ch: u64,
    block_size: u64,
    /// 已消费（每声道）字节；DSF 保持块对齐
    consumed: u64,
    factor: u32,
    file: BufReader<File>,
    data_start: u64,
    file_pos: u64,
    decim: Vec<DsdDecimator>,
    /// 超量帧缓冲（组粒度大于请求量时的余量；保证 `read_pcm_f32` 至多返回 frames）
    pending: std::collections::VecDeque<f32>,
}

impl DsdReader {
    /// 打开并定位到数据区。位率须为 44.1k 家族（抽取因子整除；非标准值向下取整）。
    pub fn open(path: &Path) -> Result<Self, NcmError> {
        let l = open_layout(path)?;
        let factor = (l.dsd_rate / 8 / OUT_RATE).max(1);
        let mut file = BufReader::new(File::open(path)?);
        file.seek(SeekFrom::Start(l.data_start))?;
        let ch = l.channels as usize;
        Ok(Self {
            kind: l.kind,
            channels: l.channels,
            dsd_rate: l.dsd_rate,
            out_rate: OUT_RATE,
            data_bytes_per_ch: l.data_bytes_per_ch,
            block_size: l.block_size,
            consumed: 0,
            factor,
            file,
            data_start: l.data_start,
            file_pos: l.data_start,
            decim: (0..ch).map(|_| DsdDecimator::new(factor)).collect(),
            pending: std::collections::VecDeque::new(),
        })
    }

    /// 总时长（秒）。
    pub fn duration_secs(&self) -> f64 {
        (self.data_bytes_per_ch as f64 * 8.0) / self.dsd_rate as f64
    }

    /// 读 PCM（**至多** `frames` 帧/声道）；EOF → 空 vec。
    ///
    /// 内部按"每声道一组字节"推进：DSF 必须整块（`block_size`）、块内按帧取字节；
    /// DFF 任意长度、字节交错。两种布局的文件偏移都满足 `data_start + 每声道字节数 × 声道数`。
    /// 组粒度可能产出超过请求量的帧——余量入 `pending`，下次优先返回（保持至多 frames 契约）。
    pub fn read_pcm_f32(&mut self, frames: usize) -> Result<Vec<f32>, NcmError> {
        let ch = self.channels as usize;
        let want = frames * ch;
        let mut out: Vec<f32> = Vec::with_capacity(want);

        // ① 先消费上次的余量
        while out.len() < want {
            match self.pending.pop_front() {
                Some(v) => out.push(v),
                None => break,
            }
        }

        // ② 按组推进
        while out.len() < want {
            let remain = self.data_bytes_per_ch - self.consumed;
            if remain == 0 {
                break;
            }
            let per_ch = if self.kind == DsdKind::Dsf {
                self.block_size.min(remain)
            } else {
                4096u64.min(remain)
            } as usize;
            let off = self.data_start + self.consumed * ch as u64;
            if self.file_pos != off {
                self.file.seek(SeekFrom::Start(off))?;
                self.file_pos = off;
            }
            let mut raw = vec![0u8; per_ch * ch];
            self.file.read_exact(&mut raw)?;
            self.file_pos += raw.len() as u64;

            let mut chunk: Vec<f32> = Vec::with_capacity(per_ch * ch / self.factor as usize + ch);
            for i in 0..per_ch {
                for (c, d) in self.decim.iter_mut().enumerate() {
                    let byte = match self.kind {
                        // DFF：字节交错 → 第 i 帧、声道 c 的字节
                        DsdKind::Dff => raw[i * ch + c],
                        // DSF：块内每声道连续 → 声道 c 段的第 i 字节
                        DsdKind::Dsf => raw[c * per_ch + i],
                    };
                    d.push_byte(byte, &mut chunk);
                }
            }

            let need = want - out.len();
            if chunk.len() > need {
                out.extend_from_slice(&chunk[..need]);
                self.pending.extend(chunk[need..].iter().copied());
            } else {
                out.extend_from_slice(&chunk);
            }
            self.consumed += per_ch as u64;
        }
        Ok(out)
    }

    /// 跳到 `ms`（DSF 块对齐——最多损失一个块 ≈ 11.6ms 音频；抽取器状态重置）。
    pub fn seek_ms(&mut self, ms: i64) -> Result<(), NcmError> {
        let ms = ms.max(0) as u64;
        let target_frame = ms.saturating_mul(self.out_rate as u64) / 1000;
        let mut byte_pos = target_frame.saturating_mul(self.factor as u64);
        if self.kind == DsdKind::Dsf && self.block_size > 0 {
            byte_pos -= byte_pos % self.block_size;
        }
        self.consumed = byte_pos.min(self.data_bytes_per_ch);
        for d in &mut self.decim {
            *d = DsdDecimator::new(self.factor);
        }
        self.pending.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// 构造最小 DSF 文件：mono、给定数据字节（每声道连续单块）。
    fn write_min_dsf(path: &Path, dsd_rate: u32, block_size: u32, data: &[u8]) {
        let mut f = File::create(path).unwrap();
        // DSD chunk（28 字节）
        f.write_all(b"DSD ").unwrap();
        f.write_all(&28u64.to_le_bytes()).unwrap();
        f.write_all(&0u64.to_le_bytes()).unwrap(); // total file size（探测不校验）
        f.write_all(&0u64.to_le_bytes()).unwrap(); // metadata ptr
        // fmt chunk（52 字节）
        f.write_all(b"fmt ").unwrap();
        f.write_all(&52u64.to_le_bytes()).unwrap();
        f.write_all(&1u32.to_le_bytes()).unwrap(); // version
        f.write_all(&0u32.to_le_bytes()).unwrap(); // format id = DSD raw
        f.write_all(&1u32.to_le_bytes()).unwrap(); // channel type
        f.write_all(&1u32.to_le_bytes()).unwrap(); // channel num = 1
        f.write_all(&dsd_rate.to_le_bytes()).unwrap();
        f.write_all(&1u32.to_le_bytes()).unwrap(); // bits per sample
        f.write_all(&((data.len() as u64) * 8).to_le_bytes()).unwrap(); // sample count（位）
        f.write_all(&block_size.to_le_bytes()).unwrap();
        f.write_all(&0u32.to_le_bytes()).unwrap(); // reserved
        // data chunk
        f.write_all(b"data").unwrap();
        f.write_all(&(12 + data.len() as u64).to_le_bytes()).unwrap();
        f.write_all(data).unwrap();
    }

    /// FIR 群延迟（192 taps → 96 中间样本 ≈ 24 输出帧）内的输出是暂态——
    /// 断言只看稳态段（后段）。
    const STEADY: usize = 48;

    #[test]
    fn dsf_probe_and_decode_all_ones_is_dc_one() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.dsf");
        // 512 字节全 0xFF → 每位 +1 → 中间样本全 +1 → PCM 稳态全 +1.0
        write_min_dsf(&p, 2_822_400, 512, &[0xFFu8; 512]);

        let info = probe_dsd(&p).unwrap();
        assert_eq!(info.kind, DsdKind::Dsf);
        assert_eq!(info.channels, 1);
        assert_eq!(info.dsd_rate, 2_822_400);
        // 512 字节 = 4096 位 / 2.8224M = 1.4512ms
        assert!((info.duration_secs - 4096.0 / 2_822_400.0).abs() < 1e-9);

        let mut r = DsdReader::open(&p).unwrap();
        let pcm = r.read_pcm_f32(64).unwrap();
        assert_eq!(pcm.len(), 64);
        for v in &pcm[STEADY..] {
            assert!((v - 1.0).abs() < 1e-3, "全 1 位流稳态应为 DC +1，实际 {v}");
        }
        // 组粒度超量帧在内部缓冲：第二次仍能取满，第三次才 EOF
        assert_eq!(r.read_pcm_f32(64).unwrap().len(), 64);
        assert!(r.read_pcm_f32(64).unwrap().is_empty());
    }

    #[test]
    fn dsf_all_zero_is_dc_minus_one_and_seek() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("z.dsf");
        write_min_dsf(&p, 2_822_400, 512, &[0x00u8; 512]);
        let mut r = DsdReader::open(&p).unwrap();
        let pcm = r.read_pcm_f32(64).unwrap();
        assert_eq!(pcm.len(), 64); // 512 字节 → 128 输出帧（取 64）
        for v in &pcm[STEADY..] {
            assert!((v + 1.0).abs() < 1e-3, "全 0 位流稳态应为 DC −1，实际 {v}");
        }
        r.seek_ms(0).unwrap();
        let again = r.read_pcm_f32(4).unwrap();
        assert_eq!(again.len(), 4, "seek 后长度守恒（前段为 FIR 暂态，不做值断言）");
        // seek 到尾部 → 空
        r.seek_ms(100_000).unwrap();
        assert!(r.read_pcm_f32(4).unwrap().is_empty());
    }

    /// FIR 阻带验证：176.4kHz 方波（0xFF/0x00 交替 → 中间样本 +1/−1 交替）
    /// 超出 88.2k 输出奈奎斯特 → 稳态幅度应被压到接近零。
    /// （这是音质升级的核心属性：DSD 带外噪声不会被折叠回音频带。）
    #[test]
    fn fir_suppresses_out_of_band() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("hf.dsf");
        let mut data = vec![0u8; 1024];
        for (i, b) in data.iter_mut().enumerate() {
            *b = if i % 2 == 0 { 0xFF } else { 0x00 };
        }
        write_min_dsf(&p, 2_822_400, 1024, &data);
        let mut r = DsdReader::open(&p).unwrap();
        let pcm = r.read_pcm_f32(256).unwrap();
        assert_eq!(pcm.len(), 256);
        let steady = &pcm[STEADY..];
        let peak = steady.iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(peak < 0.05, "带外 176.4kHz 应被低通抑制，实测稳态峰值 {peak}");
    }

    /// 构造最小 DFF：mono、byte 交错（mono 即普通字节流）。
    fn write_min_dff(path: &Path, dsd_rate: u32, data: &[u8]) {
        let mut f = File::create(path).unwrap();
        let prop_len: u64 = 4 /*SND */ + (12 + 4) /*FS*/ + (12 + 2) /*CHNL*/ + (12 + 4 + 2) /*CMPR+pad?*/;
        // 简化：FS/CHNL/CMPR 各自 chunk 按其 size 写，末尾不 pad（size 偶）
        let mut prop = Vec::new();
        prop.extend_from_slice(b"SND ");
        prop.extend_from_slice(b"FS  ");
        prop.extend_from_slice(&4u64.to_be_bytes());
        prop.extend_from_slice(&dsd_rate.to_be_bytes());
        prop.extend_from_slice(b"CHNL");
        prop.extend_from_slice(&2u64.to_be_bytes());
        prop.extend_from_slice(&1u16.to_be_bytes()); // 1 声道
        let _ = prop_len;
        // FRM8 表单：size 不校验（探测不读）
        f.write_all(b"FRM8").unwrap();
        f.write_all(&0u64.to_be_bytes()).unwrap();
        f.write_all(b"DSD ").unwrap();
        // PROP
        f.write_all(b"PROP").unwrap();
        f.write_all(&(prop.len() as u64).to_be_bytes()).unwrap();
        f.write_all(&prop).unwrap();
        // DSD data
        f.write_all(b"DSD ").unwrap();
        f.write_all(&(data.len() as u64).to_be_bytes()).unwrap();
        f.write_all(data).unwrap();
    }

    #[test]
    fn dff_probe_and_decodes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.dff");
        // 0xAA = 1010_1010 → 每 8 位求和 = 0 → PCM 全 0
        write_min_dff(&p, 2_822_400, &[0xAAu8; 16]);
        let info = probe_dsd(&p).unwrap();
        assert_eq!(info.kind, DsdKind::Dff);
        assert_eq!(info.channels, 1);
        assert_eq!(info.dsd_rate, 2_822_400);

        let mut r = DsdReader::open(&p).unwrap();
        let pcm = r.read_pcm_f32(8).unwrap();
        assert_eq!(pcm.len(), 4); // 16 字节 → 4 输出帧
        for v in &pcm {
            assert!(v.abs() < 1e-6, "交替位应为 DC 0，实际 {v}");
        }
    }

    #[test]
    fn rejects_non_dsd() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.bin");
        std::fs::write(&p, b"RIFFxxxx").unwrap();
        assert!(probe_dsd(&p).is_err());
    }

    /// 0.1 秒 mono DSF 全长读取：帧数 = 0.1 × 88200 = 8820（EOF 精确 + 块粒度无损）。
    #[test]
    fn full_length_read_matches_duration() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("long.dsf");
        // 0.1s × 2822400 bit/s / 8 = 35280 字节
        let data = vec![0xAAu8; 35_280];
        write_min_dsf(&p, 2_822_400, 4096, &data);

        let info = probe_dsd(&p).unwrap();
        assert!((info.duration_secs - 0.1).abs() < 1e-9);

        let mut r = DsdReader::open(&p).unwrap();
        let mut total = 0usize;
        loop {
            let chunk = r.read_pcm_f32(4096).unwrap();
            if chunk.is_empty() {
                break;
            }
            total += chunk.len();
            assert!(chunk.len() <= 4096, "单次返回不得超过请求帧数");
        }
        assert_eq!(total, 8820, "0.1s @88.2k 应为 8820 帧");
    }
}
