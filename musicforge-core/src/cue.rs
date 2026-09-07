//! CUE 解析与整轨切分（P5.2，D21）。
//!
//! 设计要点：
//!
//! - **编码检测**：BOM（UTF-8/UTF-16LE/BE）→ UTF-8 严格校验 → `chardetng`
//!   检测（GBK/BIG5/Windows-125x）——宽容方言，未识别指令保留不报错（R21）；
//! - **支持的指令**：REM DATE/GENRE、TITLE、PERFORMER、FILE（单文件镜像，多
//!   FILE 显式拒绝）、TRACK、INDEX（MM:SS:FF，75fps）；INDEX 00 视为前间隙，
//!   切分边界 = 各轨 INDEX 01（首轨从 0 起）；
//! - **切分**：整轨解码 → 按帧号换算采样边界切片 → 逐轨编码（同格式，无损）
//!   → lofty 写轨标签（TITLE/ARTIST/ALBUM/TRACK/DATE）+ 整轨内嵌封面写每分轨；
//! - **校验**：单轨时长 vs INDEX 差 <1s（P5a 验收）；校验在写盘**前**完成，
//!   失败轨不落盘、计数上报；
//! - **永不修改源文件**。

use std::path::{Path, PathBuf};

use crate::error::NcmError;
use crate::ffmpeg::Ffmpeg;
use crate::lossless::{decode_to_pcm, encode_pcm, probe_lossless_file, LosslessFormat, Pcm};

// ---------------------------------------------------------------- 编码检测 --

/// 字节 → UTF-8 字符串：BOM 优先，UTF-8 严格校验，失败交 chardetng（GBK/BIG5/125x）。
pub fn decode_bytes_to_utf8(bytes: &[u8]) -> String {
    // BOM 检测
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let (decoded, _, _) = encoding_rs::UTF_16LE.decode(bytes);
        return decoded.into_owned();
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let (decoded, _, _) = encoding_rs::UTF_16BE.decode(bytes);
        return decoded.into_owned();
    }
    // UTF-8 严格校验（无 BOM 的 UTF-8 是现代常态）
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    // chardetng 检测（GBK/BIG5/Windows-125x…）；0.1.17 的 guess 签名为
    // (tld, allow_utf8) -> &'static Encoding，置信度经 guess_assess 获取
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(bytes, true);
    let (encoding, confident) = detector.guess_assess(None, true);
    let (decoded, _, had_errors) = encoding.decode(bytes);
    if !confident || had_errors {
        // 检测置信度不足或仍有替换符：回退 GBK（中文 CUE 的历史主流编码）
        let (fallback, _, _) = encoding_rs::GBK.decode(bytes);
        return fallback.into_owned();
    }
    decoded.into_owned()
}

// ---------------------------------------------------------------- 解析 --

/// 单轨信息。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CueTrack {
    pub number: u32,
    pub title: Option<String>,
    pub performer: Option<String>,
    /// INDEX 01 位置（75fps 帧数）
    pub index01_frames: Option<u64>,
}

/// 解析后的 CUE 表。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CueSheet {
    pub title: Option<String>,
    pub performer: Option<String>,
    pub rem_date: Option<String>,
    pub rem_genre: Option<String>,
    /// FILE 指向的音频文件（相对 CUE 所在目录）
    pub file: Option<String>,
    pub tracks: Vec<CueTrack>,
}

impl CueSheet {
    /// 切分边界（采样数）：第 i 轨起点 = tracks[i].index01 换算采样；首轨 = 0。
    pub fn sample_boundaries(&self, sample_rate: u32) -> Vec<u64> {
        let mut bounds = Vec::with_capacity(self.tracks.len() + 1);
        for (i, t) in self.tracks.iter().enumerate() {
            if i == 0 {
                bounds.push(0);
            } else {
                match t.index01_frames {
                    Some(frames) => bounds.push(frames_to_samples(frames, sample_rate)),
                    None => bounds.push(0),
                }
            }
        }
        bounds.push(u64::MAX); // 末轨到文件尾
        bounds
    }
}

/// CUE 帧号（75fps）→ 采样序号（按目标采样率换算，四舍五入）。
pub fn frames_to_samples(frames: u64, sample_rate: u32) -> u64 {
    (frames * sample_rate as u64 + 37) / 75
}

/// 解析 CUE 文本（宽容方言：未知指令/REM 键忽略不报错）。
pub fn parse_cue_text(text: &str) -> Result<CueSheet, NcmError> {
    let mut sheet = CueSheet::default();
    let mut current: Option<CueTrack> = None;

    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (cmd, rest) = match line.split_once(' ') {
            Some((c, r)) => (c, r.trim()),
            None => (line, ""),
        };
        match cmd.to_ascii_uppercase().as_str() {
            "REM" => {
                if let Some((key, val)) = rest.split_once(' ') {
                    let val = val.trim_matches('"');
                    match key.to_ascii_uppercase().as_str() {
                        "DATE" if sheet.rem_date.is_none() => {
                            sheet.rem_date = Some(val.to_string());
                        }
                        "GENRE" if sheet.rem_genre.is_none() => {
                            sheet.rem_genre = Some(val.to_string());
                        }
                        _ => {} // 未知 REM 键宽容忽略（R21）
                    }
                }
            }
            "TITLE" => {
                let val = unquote(rest);
                match current.as_mut() {
                    Some(t) => t.title = Some(val),
                    None => sheet.title = Some(val),
                }
            }
            "PERFORMER" => {
                let val = unquote(rest);
                match current.as_mut() {
                    Some(t) => t.performer = Some(val),
                    None => sheet.performer = Some(val),
                }
            }
            "FILE" => {
                if sheet.file.is_some() {
                    return Err(NcmError::Lossless(format!(
                        "CUE 第 {} 行出现第二个 FILE：多文件镜像切分本版不支持（仅单文件镜像）",
                        lineno + 1
                    )));
                }
                // 文件名 = 第一个引号段（其后是 WAVE/MPEG 格式标记，忽略）；
                // 无引号的方言取首个空白前 token
                let name = extract_quoted(rest)
                    .or_else(|| rest.split_whitespace().next().map(|s| s.to_string()))
                    .ok_or_else(|| {
                        NcmError::Lossless(format!("CUE 第 {} 行 FILE 缺少文件名", lineno + 1))
                    })?;
                sheet.file = Some(name);
            }
            "TRACK" => {
                if let Some(t) = current.take() {
                    sheet.tracks.push(t);
                }
                let number = rest
                    .split_whitespace()
                    .next()
                    .and_then(|n| n.parse::<u32>().ok())
                    .ok_or_else(|| {
                        NcmError::Lossless(format!("CUE 第 {} 行 TRACK 编号无法解析", lineno + 1))
                    })?;
                current = Some(CueTrack {
                    number,
                    ..Default::default()
                });
            }
            "INDEX" => {
                let Some(t) = current.as_mut() else {
                    return Err(NcmError::Lossless(format!(
                        "CUE 第 {} 行 INDEX 出现在 TRACK 之前",
                        lineno + 1
                    )));
                };
                let mut parts = rest.split_whitespace();
                let idx_no = parts.next().unwrap_or("").to_string();
                let mmssff = parts.next().unwrap_or("");
                if idx_no == "01" {
                    t.index01_frames = Some(parse_mmssff(mmssff).ok_or_else(|| {
                        NcmError::Lossless(format!(
                            "CUE 第 {} 行 INDEX 01 时间格式无法解析: {mmssff}",
                            lineno + 1
                        ))
                    })?);
                }
                // INDEX 00（前间隙）与 02+ 忽略——切分边界统一取 INDEX 01
            }
            "PREGAP" | "POSTGAP" | "FLAGS" | "CATALOG" | "CDTEXTFILE" | "ISRC" => {
                // 合法但本版不消费的指令：宽容忽略（R21）
            }
            _ => {
                // 未知指令：保留不报错（宽容方言，R21）
            }
        }
    }
    if let Some(t) = current.take() {
        sheet.tracks.push(t);
    }

    if sheet.tracks.is_empty() {
        return Err(NcmError::Lossless("CUE 未包含任何 TRACK".to_string()));
    }
    if sheet.file.is_none() {
        return Err(NcmError::Lossless(
            "CUE 缺少 FILE 指令（未指定音频镜像）".to_string(),
        ));
    }
    Ok(sheet)
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    t.trim_matches('"').to_string()
}

/// 取第一个引号段内容（FILE 行：`"文件名" WAVE` → `文件名`）。
fn extract_quoted(s: &str) -> Option<String> {
    let start = s.find('"')?;
    let rest = &s[start + 1..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// `MM:SS:FF`（75fps）→ 帧数。
fn parse_mmssff(s: &str) -> Option<u64> {
    let mut it = s.split(':');
    let mm: u64 = it.next()?.parse().ok()?;
    let ss: u64 = it.next()?.parse().ok()?;
    let ff: u64 = it.next()?.parse().ok()?;
    Some(mm * 60 * 75 + ss * 75 + ff)
}

/// 读 CUE 文件（自动编码检测）并解析。
pub fn parse_cue_file(path: &Path) -> Result<CueSheet, NcmError> {
    let bytes = std::fs::read(path)?;
    let text = decode_bytes_to_utf8(&bytes);
    parse_cue_text(&text)
}

// ---------------------------------------------------------------- 切分结果 --

/// 单轨切分结果。
#[derive(Debug, Clone)]
pub struct SplitTrack {
    pub index: usize,
    pub dst: PathBuf,
    pub title: Option<String>,
    pub sample_count: usize,
    pub duration_secs: f64,
}

/// 切分报告。
#[derive(Debug, Default)]
pub struct SplitReport {
    pub tracks: Vec<SplitTrack>,
    /// 时长校验失败而未写盘的轨号（1-based）
    pub failed: Vec<(u32, String)>,
    pub sheet: CueSheet,
    pub source: PathBuf,
}

/// CUE 整轨切分：解析 → 整轨解码 → 按采样边界切片 → 逐轨编码（同格式无损）
/// → lofty 写轨标签与封面。**校验在写盘前完成**（时长 vs INDEX <1s），失败轨
/// 不落盘、计数上报。源文件与 CUE 永不修改。
pub fn split_cue(
    cue_path: &Path,
    out_dir: &Path,
    naming: impl Fn(usize, &CueTrack) -> String,
) -> Result<SplitReport, NcmError> {
    split_cue_ex(cue_path, out_dir, None, None, naming)
}

/// 扩展版：`target` 指定输出格式（None=源格式；APE/WV/TAK 源默认 FLAC）；
/// `ff` 为 ffmpeg sidecar——APE/WavPack/TAK 源经它解码为临时 WAV 再切片（D21）。
pub fn split_cue_ex(
    cue_path: &Path,
    out_dir: &Path,
    target: Option<LosslessFormat>,
    ff: Option<&Ffmpeg>,
    naming: impl Fn(usize, &CueTrack) -> String,
) -> Result<SplitReport, NcmError> {
    let sheet = parse_cue_file(cue_path)?;
    let audio_rel = sheet
        .file
        .as_deref()
        .ok_or_else(|| NcmError::Lossless("CUE 缺少 FILE 指令".to_string()))?;
    let cue_dir = cue_path.parent().unwrap_or(Path::new("."));
    let source = cue_dir.join(audio_rel);
    if !source.exists() {
        return Err(NcmError::Lossless(format!(
            "CUE 指向的音频文件不存在: {}",
            source.display()
        )));
    }

    // 源解析：WAV/FLAC 走纯 Rust 解码；APE/WV/TAK 走 ffmpeg sidecar 解码为临时
    // WAV（24-bit PCM 容器承载 16/24 位源，值空间无损；临时文件用后即删）
    const SIDECAR_EXTS: &[&str] = &["ape", "wv", "tak"];
    let src_format = probe_lossless_file(&source);
    let (whole, out_format) = match src_format {
        Some(f) => (decode_to_pcm(&source)?, target.unwrap_or(f)),
        None => {
            let ext = source
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
                .unwrap_or_default();
            if !SIDECAR_EXTS.contains(&ext.as_str()) {
                return Err(NcmError::Lossless(format!(
                    "{}: 不是支持的切分格式（WAV/FLAC/APE/WV/TAK）",
                    source.display()
                )));
            }
            let ff = ff.ok_or_else(|| {
                NcmError::Lossless(
                    "APE/WavPack/TAK 整轨切分需要 ffmpeg sidecar（未提供）".to_string(),
                )
            })?;
            let temp = out_dir.join(".mf-split-tmp.wav");
            ff.export_custom(&source, &temp, &["-c:a", "pcm_s24le"])?;
            let pcm = decode_to_pcm(&temp);
            let _ = std::fs::remove_file(&temp);
            (pcm?, target.unwrap_or(LosslessFormat::Flac))
        }
    };

    // 整轨内嵌封面提取（写每个分轨；无封面则跳过）——ffmpeg 解码源无内嵌封面
    // 可提，同目录 cover.jpg 约定归 v0.7.0 资产策略域
    let cover_bytes = if src_format.is_some() {
        extract_cover(&source)?
    } else {
        None
    };
    let ch = whole.spec.channels.max(1) as usize;

    std::fs::create_dir_all(out_dir)?;
    let mut report = SplitReport {
        sheet: sheet.clone(),
        source: source.clone(),
        ..Default::default()
    };

    // 逐轨切片：INDEX 帧位置（75fps）→ 采样帧位置 → ×channels = 交错样本位置。
    // （真机教训：混用「帧」与「交错样本」单位会把轨长折半——立体声下）
    let frame_bounds = sheet.sample_boundaries(whole.spec.sample_rate);
    let inter = |frame: u64| (frame as usize) * ch;

    for (i, track) in sheet.tracks.iter().enumerate() {
        let start = inter(frame_bounds[i]);
        let end = if i + 1 < sheet.tracks.len() {
            inter(frame_bounds[i + 1]).min(whole.samples.len())
        } else {
            whole.samples.len()
        };
        if end <= start {
            report
                .failed
                .push((track.number, "采样区间为空（INDEX 边界异常）".to_string()));
            continue;
        }
        let samples = whole.samples[start..end].to_vec();
        let track_pcm = Pcm {
            spec: whole.spec,
            samples,
        };
        let track_secs = track_pcm.sample_count() as f64 / whole.spec.sample_rate as f64;
        // INDEX 差校验：<1s
        if i + 1 < sheet.tracks.len() {
            if let Some(next_frames) = sheet.tracks[i + 1].index01_frames {
                let next_inter = inter(frames_to_samples(next_frames, whole.spec.sample_rate));
                if next_inter > whole.samples.len() {
                    report.failed.push((
                        track.number,
                        "下一轨 INDEX 超出整轨长度（CUE 与音频不匹配）".to_string(),
                    ));
                    continue;
                }
                let idx_secs =
                    (next_inter - start) as f64 / whole.spec.sample_rate as f64 / ch as f64;
                if (track_secs - idx_secs).abs() >= 1.0 {
                    report.failed.push((
                        track.number,
                        format!(
                            "单轨时长 {track_secs:.2}s 与 INDEX 差 {idx_secs:.2}s 偏差 ≥1s（CUE 与音频可能不匹配）"
                        ),
                    ));
                    continue;
                }
            }
        }

        // 写盘 + 标签 + 封面
        let name = naming(i + 1, track);
        let safe = crate::template::sanitize(&name);
        let dst = out_dir.join(format!("{safe}.{}", out_format.extension()));
        let _bytes = encode_pcm(&dst, out_format, &track_pcm)?;
        write_track_tags(
            &dst,
            out_format,
            track.title.as_deref(),
            track.performer.as_deref().or(sheet.performer.as_deref()),
            sheet.title.as_deref(),
            track.number,
            sheet.rem_date.as_deref(),
            cover_bytes.as_deref(),
        )?;

        report.tracks.push(SplitTrack {
            index: i + 1,
            dst,
            title: track.title.clone(),
            sample_count: track_pcm.sample_count(),
            duration_secs: track_secs,
        });
    }
    Ok(report)
}

/// 从整轨提取内嵌封面（首个图片；无封面 → None）。
fn extract_cover(source: &Path) -> Result<Option<Vec<u8>>, NcmError> {
    use lofty::prelude::*;
    let tagged = lofty::read_from_path(source).map_err(|e| NcmError::Lossless(e.to_string()))?;
    Ok(tagged
        .primary_tag()
        .or_else(|| tagged.first_tag())
        .and_then(|t| t.pictures().first())
        .map(|p| p.data().to_vec()))
}

/// 给分轨写标签（TITLE/ARTIST/ALBUM/TRACKNUMBER/DATE + 封面）。
/// FillMissingOnly 语义不适用（新产物，直接写全量）。
#[allow(clippy::too_many_arguments)]
fn write_track_tags(
    path: &Path,
    format: LosslessFormat,
    title: Option<&str>,
    artist: Option<&str>,
    album: Option<&str>,
    track_no: u32,
    date: Option<&str>,
    cover: Option<&[u8]>,
) -> Result<(), NcmError> {
    use lofty::config::WriteOptions;
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::prelude::*;
    use lofty::tag::{ItemKey, Tag, TagType};

    let mut tagged = lofty::read_from_path(path).map_err(|e| NcmError::Lossless(e.to_string()))?;
    // 目标标签类型按容器选择：WAV → Id3v2（RiffInfo 不支持图片块），FLAC → VorbisComments
    let ttype = match format {
        LosslessFormat::Wav => TagType::Id3v2,
        _ => tagged.primary_tag_type(),
    };
    if tagged.tag(ttype).is_none() {
        tagged.insert_tag(Tag::new(ttype));
    }
    let Some(tag) = tagged.tag_mut(ttype) else {
        return Err(NcmError::Lossless(format!(
            "{}: 无法创建标签容器",
            path.display()
        )));
    };
    if let Some(t) = title {
        tag.insert_text(ItemKey::TrackTitle, t.to_string());
    }
    if let Some(a) = artist {
        tag.insert_text(ItemKey::TrackArtist, a.to_string());
    }
    if let Some(al) = album {
        tag.insert_text(ItemKey::AlbumTitle, al.to_string());
    }
    if let Some(d) = date {
        tag.insert_text(ItemKey::Year, d.to_string());
    }
    tag.insert_text(ItemKey::TrackNumber, track_no.to_string());
    if let Some(data) = cover {
        let mime = if data.starts_with(&[0x89, b'P', b'N', b'G']) {
            MimeType::Png
        } else {
            MimeType::Jpeg
        };
        let pic = Picture::unchecked(data.to_vec())
            .mime_type(mime)
            .pic_type(PictureType::CoverFront)
            .build();
        tag.push_picture(pic);
    }
    tagged
        .save_to_path(path, WriteOptions::default())
        .map_err(|e| NcmError::Lossless(e.to_string()))?;
    Ok(())
}
