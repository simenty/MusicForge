//! 曲库索引层（P1 曲库体验）：把磁盘上的音频文件索引进 db v2 维度表。
//!
//! 分工与边界：
//! - [`crate::scan`]：只读扫描 + 分类（含 D17 哈希缓存）——不解析音频元数据；
//! - 本模块：在扫描结果之上用 lofty 读取容器/标签元数据，产出
//!   [`crate::db::TrackInput`] 批量入库（artists / albums / tracks）。
//!
//! 铁律沿用：
//! - **db 是可再生的**——单个文件标签读取失败只降级（基础字段照常入库、
//!   title 退化为文件名），绝不中止整次索引；
//! - 音乐文件全程只读；
//! - 写库集中在主线程（SQLite 单写者），读标签并行（CPU 密集）。

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use lofty::prelude::*;
use lofty::tag::{ItemKey, Tag};

use crate::db::{Db, TrackInput};
use crate::error::NcmError;
use crate::scan::{scan_library, Category, ScanItem, ScanOptions};

/// 格式级无损判定（容器本身无损；「真无损」由 `lossless` 检测另行判定）。
///
/// 不复用 [`crate::lossless::LosslessFormat::parse`]：那是**转码支持集合**
/// （仅 wav/flac），本判定是**识别集合**（要覆盖曲库里可能存在的全部无损容器）。
fn is_lossless_ext(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "flac" | "wav" | "wave" | "aif" | "aiff" | "ape" | "wv" | "tta" | "dff" | "dsf" | "alac"
    )
}

/// 索引结果统计。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct IndexOutcome {
    /// 扫描到的文件总数（含非音频）
    pub scanned_files: usize,
    /// 其中音频文件数（参与索引）
    pub audio: usize,
    /// 写入 db 的行数
    pub indexed: usize,
    /// 读到了标题或艺术家标签
    pub tagged: usize,
    /// 成功读取但无标签（title 退化为文件名）
    pub untagged: usize,
    /// 标签读取失败（仅基础字段入库）
    pub failed: usize,
    /// 清理的陈旧行（文件已从磁盘移除）
    pub removed: usize,
}

/// 单文件解析结果：`(入库行, 是否有标签, 是否读取失败)`。
type Parsed = (TrackInput, bool, bool);

/// 索引一个媒体源目录：扫描 → 并行读标签 → 批量入库 → 清理陈旧行。
///
/// `source_id` 必须已由 [`Db::upsert_source`] 登记。
pub fn index_library(
    db: &Db,
    source_id: i64,
    root: &Path,
    options: &ScanOptions,
) -> Result<IndexOutcome, NcmError> {
    let report = scan_library(root, options)?;
    let audio: Vec<&ScanItem> = report
        .items
        .iter()
        .filter(|i| i.category == Category::Audio)
        .collect();

    let parsed = parallel_parse(&audio, source_id, jobs_of(options));

    let mut tagged = 0usize;
    let mut failed = 0usize;
    for (_, t, f) in &parsed {
        if *t {
            tagged += 1;
        }
        if *f {
            failed += 1;
        }
    }
    let untagged = parsed.len().saturating_sub(tagged + failed);

    let run_id = now_secs();
    // 先入库（move 掉已统计的解析结果），再按 run 标记清陈旧行
    let inputs: Vec<TrackInput> = parsed.into_iter().map(|(t, _, _)| t).collect();
    let indexed = db.upsert_tracks_batch(&inputs, run_id)?;
    let removed = db.remove_stale_tracks(source_id, run_id)?;

    Ok(IndexOutcome {
        scanned_files: report.scanned_files,
        audio: audio.len(),
        indexed,
        tagged,
        untagged,
        failed,
        removed,
    })
}

/// 并行读标签（分块 + 每块一次锁），返回与输入一一对应的解析结果。
fn parallel_parse(items: &[&ScanItem], source_id: i64, jobs: usize) -> Vec<Parsed> {
    if items.is_empty() {
        return Vec::new();
    }
    let chunk = items.len().div_ceil(jobs.max(1)).max(1);
    let collected: std::sync::Mutex<Vec<Parsed>> =
        std::sync::Mutex::new(Vec::with_capacity(items.len()));
    std::thread::scope(|scope| {
        for part in items.chunks(chunk) {
            scope.spawn(|| {
                let local: Vec<Parsed> = part.iter().map(|it| parse_one(it, source_id)).collect();
                // 锁中毒恢复（本层零 panic，中毒意味着上游 bug——仍不放弃已解析数据）
                let mut guard = match collected.lock() {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };
                guard.extend(local);
            });
        }
    });
    collected.into_inner().unwrap_or_default()
}

/// 单文件：基础字段（扫描已有）+ 标签字段（lofty 读取，失败降级）。
fn parse_one(item: &ScanItem, source_id: i64) -> Parsed {
    let ext = item
        .path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    // 文件名（无扩展名）作为无标签时的标题回退——含 .cue 分轨名等场景的朴素解
    let stem = item
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    let mut t = TrackInput {
        source_id,
        path: item.path.to_string_lossy().into_owned(),
        size: item.size as i64,
        mtime: item.mtime,
        format: (!ext.is_empty()).then(|| ext.clone()),
        is_lossless: is_lossless_ext(&ext),
        ..Default::default()
    };

    match read_audio_meta(&item.path) {
        Some(m) => {
            let tagged = m.title.is_some() || m.artist.is_some();
            t.title = m.title.or(Some(stem));
            t.artist = m.artist;
            t.album = m.album;
            t.album_artist = m.album_artist;
            t.year = m.year;
            t.track_no = m.track_no;
            t.disc_no = m.disc_no;
            t.duration_ms = m.duration_ms;
            t.sample_rate = m.sample_rate;
            t.bit_depth = m.bit_depth;
            t.channels = m.channels;
            (t, tagged, false)
        }
        None => {
            t.title = Some(stem);
            (t, false, true)
        }
    }
}

/// 从音频文件读出的一批元数据（读不到的字段为 `None`）。
#[derive(Debug, Default)]
struct AudioMeta {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    year: Option<i64>,
    track_no: Option<i64>,
    disc_no: Option<i64>,
    duration_ms: Option<i64>,
    sample_rate: Option<i64>,
    bit_depth: Option<i64>,
    channels: Option<i64>,
}

/// lofty 读取：容器属性（时长/采样率/位深/声道）+ 主标签文本字段。
///
/// 读取失败（损坏/虚假扩展名）→ `None`，调用方降级处理。
fn read_audio_meta(path: &Path) -> Option<AudioMeta> {
    let tagged = lofty::read_from_path(path).ok()?;
    let props = tagged.properties();
    let mut m = AudioMeta {
        duration_ms: Some(props.duration().as_millis() as i64),
        sample_rate: props.sample_rate().map(i64::from),
        bit_depth: props.bit_depth().map(i64::from),
        channels: props.channels().map(i64::from),
        ..Default::default()
    };
    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
        m.title = tag_string(tag, &ItemKey::TrackTitle);
        m.artist = tag_string(tag, &ItemKey::TrackArtist);
        m.album = tag_string(tag, &ItemKey::AlbumTitle);
        m.album_artist = tag_string(tag, &ItemKey::AlbumArtist);
        m.track_no = tag_string(tag, &ItemKey::TrackNumber).and_then(|s| first_number(&s));
        m.disc_no = tag_string(tag, &ItemKey::DiscNumber).and_then(|s| first_number(&s));
        m.year = tag_string(tag, &ItemKey::Year).and_then(|s| first_year(&s));
    }
    Some(m)
}

/// 标签文本字段（trim 后空串按缺失处理——对齐 tagger 的 `has_value` 语义）。
fn tag_string(tag: &Tag, key: &ItemKey) -> Option<String> {
    tag.get_string(*key)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 取字符串开头的连续数字（`"3/12"` → 3、`"07"` → 7）。
fn first_number(s: &str) -> Option<i64> {
    let digits: String = s
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// 从日期类字符串取年份（`"2003"` / `"2003-05-01"` → 2003）。
fn first_year(s: &str) -> Option<i64> {
    let y = first_number(s)?;
    (1000..=2999).contains(&y).then_some(y)
}

/// 并行度（与 `scan` 同一策略：0 = 自动，上限 8）。
fn jobs_of(options: &ScanOptions) -> usize {
    if options.parallel_jobs == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .min(8)
    } else {
        options.parallel_jobs
    }
}

/// UNIX 秒（索引 run 标记；获取失败按 0——只会让本次清理更保守）。
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
