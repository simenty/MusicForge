//! 曲库去重（P4.1）：exact 内容重复 + 同名候选 + 可解释保留评分。
//!
//! 设计要点（对齐 ROADMAP P4 与 v2.4 方案 §P4）：
//!
//! - **exact**：大小预筛 → 流式哈希分桶；内容完全相同的组保留 1 份，
//!   其余为牺牲项——apply 时**全部进回收站**（复用 [`crate::scan`] 的
//!   `CleanPlan`/`apply_clean_plan`/rollback 机制），绝不直接删除；
//! - **同名候选**：同 stem 不同内容的组（同一首歌的不同编码是典型形态）。
//!   **默认仅报告不执行**——同名≠同歌（现场版/重制版风险），牺牲决策留给
//!   用户或 v0.7.0 的 AI 复核；CLI `--include-same-name` 才纳入 apply；
//! - **保留评分解释器**：无损+40 / 采样率+8 / 位深+8 / 标签+10 / 封面+5 /
//!   校验+20（完整性侧车 `<file>.musicforge.json` 双一致）→ 总分；
//!   牺牲项 reason = 明细展开，**两次运行分数与保留项完全一致**（确定性）；
//!   平分取路径字典序最小为保留（同样确定，reason 中明示）；
//! - **哈希来源**：复用 D17 sha256 哈希缓存（P3.3）——对 v2.4 方案「BLAKE3
//!   扫描哈希」的有据偏差：缓存已就绪，零新依赖、零二次全量哈希，正确性等价；
//! - **`--suggest`**（AI 保留建议）在离线版显式报 `MF-PLUGIN-NOT-FOUND`，
//!   绝不静默装作给过建议。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::NcmError;
use crate::scan::{scan_library, Category, ScanItem, ScanOptions};

// ---------------------------------------------------------------- 评分 --

/// 评分权重（v2.4 方案 §P4 固定值；改动=评分语义变更，必须在文档记录）。
pub const W_LOSSLESS: i32 = 40;
pub const W_SAMPLE_RATE: i32 = 8;
pub const W_BIT_DEPTH: i32 = 8;
pub const W_TAGS: i32 = 10;
pub const W_COVER: i32 = 5;
pub const W_VERIFIED: i32 = 20;

/// 单文件评分明细（可解释、可复算：确定性函数 of (文件可观测属性, 组内最大值)）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScoreBreakdown {
    /// 无损容器（lofty 可解析且为 FLAC/WAV/APE/WavPack）→ +40
    pub lossless: bool,
    /// 采样率（组内最高者 +8；不可得=0 永不得分）
    pub sample_rate: u32,
    /// 位深（组内最高者 +8；不可得=0 永不得分）
    pub bit_depth: u32,
    /// 任一标签含标题或歌手 → +10
    pub has_tags: bool,
    /// 内嵌封面 → +5
    pub has_cover: bool,
    /// 完整性侧车（`<file>.musicforge.json`）存在且 size+sha256 双一致 → +20
    pub verified: bool,
    /// 降级说明（解析失败等；计入 reason，绝不静默）
    pub notes: Vec<String>,
}

impl ScoreBreakdown {
    /// 总分。`max_sample_rate`/`max_bit_depth` 为**组内**最大值（0 表示全组未知）。
    pub fn total(&self, max_sample_rate: u32, max_bit_depth: u32) -> i32 {
        let mut s = 0i32;
        if self.lossless {
            s += W_LOSSLESS;
        }
        if max_sample_rate > 0 && self.sample_rate == max_sample_rate {
            s += W_SAMPLE_RATE;
        }
        if max_bit_depth > 0 && self.bit_depth == max_bit_depth {
            s += W_BIT_DEPTH;
        }
        if self.has_tags {
            s += W_TAGS;
        }
        if self.has_cover {
            s += W_COVER;
        }
        if self.verified {
            s += W_VERIFIED;
        }
        s
    }

    /// 明细展开（牺牲项 reason 的组成部分；与 [`Self::total`] 同源，可复算）。
    pub fn detail(&self, max_sample_rate: u32, max_bit_depth: u32) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.push(if self.lossless {
            format!("无损+{W_LOSSLESS}")
        } else {
            "无损+0".to_string()
        });
        parts.push(format!(
            "采样率+{}",
            if max_sample_rate > 0 && self.sample_rate == max_sample_rate {
                W_SAMPLE_RATE
            } else {
                0
            }
        ));
        parts.push(format!(
            "位深+{}",
            if max_bit_depth > 0 && self.bit_depth == max_bit_depth {
                W_BIT_DEPTH
            } else {
                0
            }
        ));
        parts.push(format!("标签+{}", if self.has_tags { W_TAGS } else { 0 }));
        parts.push(format!("封面+{}", if self.has_cover { W_COVER } else { 0 }));
        parts.push(format!(
            "校验+{}",
            if self.verified { W_VERIFIED } else { 0 }
        ));
        let mut s = parts.join(" ");
        for n in &self.notes {
            s.push_str(&format!("；{n}"));
        }
        s
    }
}

/// 完整性侧车校验（与 CLI 转换写入的 `<file>.musicforge.json` 同语义：
/// size + sha256 双一致才算 verified；缺失/损坏 = false，不报错）。
fn sidecar_verified(path: &Path, size: u64, sha256: &str) -> bool {
    let sidecar = PathBuf::from(format!("{}.musicforge.json", path.display()));
    let Ok(text) = std::fs::read_to_string(&sidecar) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    v["size"].as_u64() == Some(size) && v["sha256"].as_str() == Some(sha256)
}

/// 读取单个音频文件的可观测属性用于评分。
///
/// 解析失败**不报错**：按全未知计 0 分并写入 notes（评分是偏好启发，
/// 不应让整个去重因单个坏文件失败；坏文件本身也不会被认领为牺牲依据）。
fn score_file(path: &Path, size: u64, sha256: &str) -> ScoreBreakdown {
    use lofty::prelude::*;
    let verified = sidecar_verified(path, size, sha256);
    let mut sc = ScoreBreakdown {
        verified,
        ..Default::default()
    };
    match lofty::read_from_path(path) {
        Ok(tagged) => {
            let props = tagged.properties();
            sc.sample_rate = props.sample_rate().unwrap_or(0);
            sc.bit_depth = props.bit_depth().map(|d| d as u32).unwrap_or(0);
            sc.lossless = matches!(
                tagged.file_type(),
                lofty::file::FileType::Flac
                    | lofty::file::FileType::Wav
                    | lofty::file::FileType::Ape
                    | lofty::file::FileType::WavPack
            );
            sc.has_tags = tagged.tags().iter().any(|t| {
                non_empty(t.get_string(lofty::tag::ItemKey::TrackTitle))
                    || non_empty(t.get_string(lofty::tag::ItemKey::TrackArtist))
            });
            sc.has_cover = tagged.tags().iter().any(|t| !t.pictures().is_empty());
        }
        Err(e) => sc.notes.push(format!("属性解析失败（按未知计 0 分）: {e}")),
    }
    sc
}

fn non_empty(s: Option<&str>) -> bool {
    s.map(|v| !v.trim().is_empty()).unwrap_or(false)
}

// ---------------------------------------------------------------- 扫描 --

/// 去重组内成员。
#[derive(Debug, Clone)]
pub struct DedupeFile {
    pub path: PathBuf,
    pub size: u64,
    pub sha256: String,
    pub score: ScoreBreakdown,
}

/// exact 内容重复组（全部成员 sha256 相同）。
#[derive(Debug, Clone)]
pub struct DupGroup {
    pub sha256: String,
    pub size: u64,
    /// 全部成员（含保留项；≥2）
    pub files: Vec<DedupeFile>,
    /// 建议保留成员的下标（分数最高；平分取路径字典序最小）
    pub keep_index: usize,
}

impl DupGroup {
    pub fn keep(&self) -> &DedupeFile {
        &self.files[self.keep_index]
    }

    /// 牺牲项（apply 时移入回收站的成员）。
    pub fn sacrifices(&self) -> Vec<&DedupeFile> {
        self.files
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != self.keep_index)
            .map(|(_, f)| f)
            .collect()
    }

    /// 组内最大采样率/位深（评分上下文）。
    fn maxima(&self) -> (u32, u32) {
        (
            self.files
                .iter()
                .map(|f| f.score.sample_rate)
                .max()
                .unwrap_or(0),
            self.files
                .iter()
                .map(|f| f.score.bit_depth)
                .max()
                .unwrap_or(0),
        )
    }

    /// 成员得分（含组上下文）。
    pub fn score_of(&self, f: &DedupeFile) -> i32 {
        let (mr, md) = self.maxima();
        f.score.total(mr, md)
    }

    /// 牺牲项 reason（明细展开；两次运行逐字节一致）。
    pub fn sacrifice_reason(&self, f: &DedupeFile) -> String {
        let keep = self.keep();
        let (mr, md) = self.maxima();
        let tie = self
            .files
            .iter()
            .all(|x| x.score.total(mr, md) == keep.score.total(mr, md));
        let mut r = format!(
            "完全重复（sha256 {}…）：得分 {} [{}]；保留 {}（得分 {}）",
            &self.sha256[..8.min(self.sha256.len())],
            f.score.total(mr, md),
            f.score.detail(mr, md),
            keep.path.display(),
            keep.score.total(mr, md),
        );
        if tie {
            r.push_str("；全体平分，优先保留无 (N) 重复标记的文件名，仍平分取路径字典序最小");
        }
        r
    }
}

/// 同名候选组（同 stem、内容不全相同；**默认仅报告**）。
#[derive(Debug, Clone)]
pub struct SameNameGroup {
    pub stem: String,
    pub files: Vec<DedupeFile>,
    /// 建议保留下标（评分最高；平分取路径字典序最小）
    pub keep_index: usize,
}

impl SameNameGroup {
    pub fn keep(&self) -> &DedupeFile {
        &self.files[self.keep_index]
    }

    fn maxima(&self) -> (u32, u32) {
        (
            self.files
                .iter()
                .map(|f| f.score.sample_rate)
                .max()
                .unwrap_or(0),
            self.files
                .iter()
                .map(|f| f.score.bit_depth)
                .max()
                .unwrap_or(0),
        )
    }

    pub fn score_of(&self, f: &DedupeFile) -> i32 {
        let (mr, md) = self.maxima();
        f.score.total(mr, md)
    }

    /// 候选牺牲项 reason（仅报告；`--include-same-name` 时随 apply 进回收站）。
    pub fn candidate_reason(&self, f: &DedupeFile) -> String {
        let keep = self.keep();
        let (mr, md) = self.maxima();
        format!(
            "同名不同内容（{}.*）：得分 {} [{}]；建议保留 {}（得分 {}）",
            self.stem,
            f.score.total(mr, md),
            f.score.detail(mr, md),
            keep.path.display(),
            keep.score.total(mr, md),
        )
    }
}

/// 去重报告。
#[derive(Debug, Default)]
pub struct DedupeReport {
    /// exact 内容重复组（≥2 成员）
    pub groups: Vec<DupGroup>,
    /// 同名候选组（默认仅报告）
    pub same_name: Vec<SameNameGroup>,
    pub files_seen: usize,
    pub cache_hits: usize,
    pub hashed_now: usize,
    /// 无法哈希（读取失败）而跳过的文件数
    pub skipped: usize,
}

/// 去重选项。
#[derive(Debug, Clone)]
pub struct DedupeOptions {
    /// 是否检测同名候选组（默认开；关闭可省去对 size 唯一文件的补算）
    pub same_name: bool,
}

impl Default for DedupeOptions {
    fn default() -> Self {
        Self { same_name: true }
    }
}

/// 扫描 `root` 生成去重报告（只读；哈希结果经 `db` 缓存复用/回写）。
///
/// `db = None` 时直接流式计算、不落缓存（小库/一次性场景）。
pub fn dedupe_scan(
    root: &Path,
    options: &DedupeOptions,
    db: Option<&crate::db::Db>,
) -> Result<DedupeReport, NcmError> {
    if !root.is_dir() {
        return Err(NcmError::Db(format!(
            "dedupe: 目录不存在或不可读: {}",
            root.display()
        )));
    }
    let scan = scan_library(root, &ScanOptions::default())?;
    let audio: Vec<&ScanItem> = scan
        .items
        .iter()
        .filter(|i| i.category == Category::Audio)
        .collect();

    let mut rep = DedupeReport {
        files_seen: audio.len(),
        ..Default::default()
    };

    // 1) 大小预筛：size 唯一且 stem 唯一的文件不可能进任何组，跳过哈希
    let mut size_counts: BTreeMap<u64, usize> = BTreeMap::new();
    let mut stem_counts: BTreeMap<String, usize> = BTreeMap::new();
    for it in &audio {
        *size_counts.entry(it.size).or_default() += 1;
        if options.same_name {
            *stem_counts.entry(same_name_key(&it.path)).or_default() += 1;
        }
    }
    let needs_hash = |it: &ScanItem| -> bool {
        size_counts.get(&it.size).copied().unwrap_or(0) > 1
            || (options.same_name
                && stem_counts
                    .get(&same_name_key(&it.path))
                    .copied()
                    .unwrap_or(0)
                    > 1)
    };

    // 2) 哈希（缓存优先）并携带评分
    let mut by_hash: BTreeMap<String, Vec<DedupeFile>> = BTreeMap::new();
    let mut by_stem: BTreeMap<String, Vec<DedupeFile>> = BTreeMap::new();
    for it in &audio {
        if !needs_hash(it) {
            continue;
        }
        let Some(sha) = hash_of(it, db, &mut rep) else {
            rep.skipped += 1;
            continue;
        };
        let score = score_file(&it.path, it.size, &sha);
        let f = DedupeFile {
            path: it.path.clone(),
            size: it.size,
            sha256: sha.clone(),
            score,
        };
        by_hash.entry(sha.clone()).or_default().push(f.clone());
        if options.same_name {
            by_stem.entry(same_name_key(&it.path)).or_default().push(f);
        }
    }

    // 3) exact 组（≥2 成员）
    for (sha, mut files) in by_hash {
        if files.len() < 2 {
            continue;
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let (mr, md) = (
            files.iter().map(|f| f.score.sample_rate).max().unwrap_or(0),
            files.iter().map(|f| f.score.bit_depth).max().unwrap_or(0),
        );
        let keep_index = pick_keep(&files, mr, md);
        rep.groups.push(DupGroup {
            sha256: sha,
            size: files[0].size,
            files,
            keep_index,
        });
    }

    // 4) 同名候选组（组内哈希种类 ≥2 才算——全同的已在 exact 组）
    if options.same_name {
        for (same_name_key, mut files) in by_stem {
            // 分组键（dir::stem）仅用于聚合；展示 stem 取成员真实文件名
            let _ = same_name_key;
            if files.len() < 2 {
                continue;
            }
            files.sort_by(|a, b| a.path.cmp(&b.path));
            let distinct = files
                .iter()
                .map(|f| f.sha256.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            if distinct.len() < 2 {
                continue;
            }
            let (mr, md) = (
                files.iter().map(|f| f.score.sample_rate).max().unwrap_or(0),
                files.iter().map(|f| f.score.bit_depth).max().unwrap_or(0),
            );
            let keep_index = pick_keep(&files, mr, md);
            // 展示用 stem 取成员真实文件名（分组键是 dir::stem，不直接展示）
            let stem = files[0]
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            rep.same_name.push(SameNameGroup {
                stem,
                files,
                keep_index,
            });
        }
    }

    Ok(rep)
}

/// 确定性保留选择：得分最高；平分时优先保留**无 ` (N)` 重复标记**的文件名
/// （真库实测：重复下载残留形如 `song.flac` + `song (2).flac`，用户期望保留
/// 干净命名；` (N)` 垨记视为复制产物）；仍平分取路径字典序最小（files 已按路径排序）。
fn pick_keep(files: &[DedupeFile], max_rate: u32, max_depth: u32) -> usize {
    let mut best = 0usize;
    let mut best_score = files[0].score.total(max_rate, max_depth);
    let mut best_artifact = is_duplicate_artifact_name(&files[0].path);
    for (i, f) in files.iter().enumerate().skip(1) {
        let s = f.score.total(max_rate, max_depth);
        if s > best_score {
            best = i;
            best_score = s;
            best_artifact = is_duplicate_artifact_name(&f.path);
        } else if s == best_score {
            let artifact = is_duplicate_artifact_name(&f.path);
            if best_artifact && !artifact {
                // 同分：无 (N) 标记者胜出（files 已按路径升序，后续同级不覆盖先到者）
                best = i;
                best_artifact = artifact;
            }
        }
    }
    best
}

/// 文件名是否带 ` (N)` 复制产物标记（stem 以 ` (数字)` 结尾）。
fn is_duplicate_artifact_name(path: &Path) -> bool {
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    let Some(i) = stem.rfind(" (") else {
        return false;
    };
    let inner = &stem[i + 2..];
    stem.ends_with(')')
        && !inner.is_empty()
        && inner[..inner.len() - 1].chars().all(|c| c.is_ascii_digit())
}

fn stem_key(p: &Path) -> String {
    p.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default()
}

/// 同名候选分组键：**同目录** + stem（大小写不敏感）。
///
/// 跨目录同名不算同名候选——`A/track01.flac` 与 `B/track01.mp3` 几乎必然是
/// 不同歌曲（合辑编号），把它们判成同歌是数据破坏。同目录同名才是
/// 「同一首歌的不同编码」的典型形态。
fn same_name_key(p: &Path) -> String {
    let dir = p
        .parent()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("{dir}::{}", stem_key(p))
}

/// 取哈希：db 缓存（size+mtime 命中）→ 未命中流式计算并回写。
fn hash_of(it: &ScanItem, db: Option<&crate::db::Db>, rep: &mut DedupeReport) -> Option<String> {
    let key = it.path.to_string_lossy().into_owned();
    if let Some(db) = db {
        if let Some(mtime) = it.mtime {
            if let Ok(Some(h)) = db.cached_hash(&key, it.size as i64, mtime) {
                rep.cache_hits += 1;
                return Some(h);
            }
        }
    }
    let sha = crate::scan::sha256_file_stream(&it.path)?;
    rep.hashed_now += 1;
    if let Some(db) = db {
        if let Some(mtime) = it.mtime {
            let _ = db.upsert_file(
                &key,
                it.size as i64,
                Some(mtime),
                it.path.extension().and_then(|e| e.to_str()),
                Some(&sha),
            );
        }
    }
    Some(sha)
}

// ------------------------------------------------------ similar_cover --

/// 相似封面判定阈值：8×8 aHash 的汉明距离 ≤ 8/64 位（经验值；v0.7.0 AI 复核）。
pub const COVER_HAMMING_THRESHOLD: u32 = 8;

/// 相似封面组（**仅报告**——v2.4 §P4「分组报告」，换封面动作在 v0.7.0）。
#[derive(Debug, Clone)]
pub struct CoverGroup {
    /// 组代表（封面哈希的字典序最小成员路径）
    pub rep_path: PathBuf,
    pub rep_hash: u64,
    /// 成员：(路径, aHash, 与 rep 的汉明距离)；含 rep 自身（距离 0）
    pub members: Vec<(PathBuf, u64, u32)>,
}

/// 提取内嵌封面并计算 8×8 灰度 aHash（只用已嵌入封面字节，绝不联网）。
///
/// 无封面/封面解码失败 → None（进 skipped 计数，不报错——单个坏图
/// 不应让整库封面扫描失败）。
pub fn cover_ahash(path: &Path) -> Option<u64> {
    use lofty::prelude::*;
    let tagged = lofty::read_from_path(path).ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    let pic = tag.pictures().first()?;
    let img = image::load_from_memory(pic.data()).ok()?;
    Some(grayscale_ahash(&img))
}

/// 8×8 灰度均值 aHash（手写，无第三方哈希依赖）。
fn grayscale_ahash(img: &image::DynamicImage) -> u64 {
    let gray = img.to_luma8();
    let small = image::imageops::resize(&gray, 8, 8, image::imageops::FilterType::Lanczos3);
    let mean = small.pixels().map(|p| p.0[0] as u64).sum::<u64>() / 64;
    small.pixels().enumerate().fold(0u64, |acc, (i, p)| {
        if p.0[0] as u64 > mean {
            acc | (1u64 << i)
        } else {
            acc
        }
    })
}

fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// 相似封面扫描：内嵌封面 aHash 聚类（汉明 ≤ [`COVER_HAMMING_THRESHOLD`]）。
///
/// **只读、只报告**——产出的组绝不适配进清洗计划（换封面属 L2 能力）。
pub fn similar_cover_scan(root: &Path) -> Result<Vec<CoverGroup>, NcmError> {
    let scan = crate::scan::scan_library(root, &crate::scan::ScanOptions::default())?;
    let mut items: Vec<(PathBuf, u64)> = Vec::new();
    for item in scan.items.iter().filter(|i| i.category == Category::Audio) {
        if let Some(h) = cover_ahash(&item.path) {
            items.push((item.path.clone(), h));
        }
    }
    // 字典序稳定：聚类结果不依赖遍历顺序
    items.sort_by(|a, b| a.0.cmp(&b.0));

    // 并查集聚类
    let n = items.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], x: usize) -> usize {
        let mut x = x;
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    for i in 0..n {
        for j in (i + 1)..n {
            if hamming(items[i].1, items[j].1) <= COVER_HAMMING_THRESHOLD {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    // 小根合并：代表 = 下标更小者（= 路径更小者，已排序）
                    let (lo, hi) = if ri < rj { (ri, rj) } else { (rj, ri) };
                    parent[hi] = lo;
                }
            }
        }
    }

    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..n {
        groups.entry(find(&mut parent, i)).or_default().push(i);
    }
    let mut out = Vec::new();
    for (_, members) in groups {
        if members.len() < 2 {
            continue;
        }
        let (rep_idx, rep_hash) = (members[0], items[members[0]].1);
        let ms = members
            .iter()
            .map(|&i| {
                (
                    items[i].0.clone(),
                    items[i].1,
                    hamming(items[i].1, rep_hash),
                )
            })
            .collect();
        out.push(CoverGroup {
            rep_path: items[rep_idx].0.clone(),
            rep_hash,
            members: ms,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------- 计划 --

/// 把 exact 组牺牲项（可选：同名候选）构建为清洗计划——复用 P3 回收站机制。
///
/// 传入 `include_same_name=true` 时，同名候选组的非保留成员一并纳入
/// （默认 false：同名≠同歌，牺牲决策留给用户/AI）。
/// 逐项牺牲理由经 [`DupGroup::sacrifice_reason`] / [`SameNameGroup::candidate_reason`]
/// 获取（CLI/GUI 报告时展开）；回滚清单 `rule` 字段用稳定规则 ID。
pub fn build_dedupe_plan(
    report: &DedupeReport,
    trash_root: &Path,
    scan_root: &Path,
    include_same_name: bool,
) -> crate::scan::CleanPlan {
    let mut plan = crate::scan::CleanPlan {
        trash_root: trash_root.to_path_buf(),
        scan_root: scan_root.to_path_buf(),
        ..Default::default()
    };
    for g in &report.groups {
        for f in g.sacrifices() {
            if f.path.exists() {
                plan.actions.push(crate::scan::CleanAction {
                    path: f.path.clone(),
                    rule_id: "MF-DUP-EXACT",
                });
            }
        }
    }
    if include_same_name {
        for g in &report.same_name {
            for (i, f) in g.files.iter().enumerate() {
                if i != g.keep_index && f.path.exists() {
                    plan.actions.push(crate::scan::CleanAction {
                        path: f.path.clone(),
                        rule_id: "MF-DUP-SAME-NAME",
                    });
                }
            }
        }
    }
    plan
}
