//! 曲库扫描与清洗（P3）。
//!
//! 设计要点（对齐 ROADMAP §5 P3 与治理 §4.13）：
//!
//! - **只读扫描**：[`scan_library`] 递归遍历目录树并分类，不改动任何文件；
//! - **规则卡**：每条清洗规则有 ID/描述/风险/默认启停/可逆性（`RULE_CARDS`），
//!   与 GUI/文档共用一份定义（规则即数据）；
//! - **清洗计划**：[`build_clean_plan`] 依据启用的规则生成动作清单；
//!   执行动作 = **移入回收站目录**（保留相对结构），可整体还原——绝不直接删除；
//! - **D17 增量哈希缓存**：[`refresh_hash_cache`] 以 size+mtime（L1）判定
//!   缓存命中（零文件读取），未命中才流式重算 sha256（L2）并回写 db
//!   （复用 [`crate::db::Db::cached_hash`] 原语）——二次扫描不重算。
//!
//! 刻意不做的事：不引入 walkdir/ignore/rayon（依赖面最小）；walker 为
//! 有界深度的迭代实现，符号链接一律不跟随（防环）。
//!
//! X25（2026-09-10，主理人裁决「并行」）：标准库 `thread::scope` 有界 worker 池
//! 实现**目录级并行**——**零新依赖**（原「不引入 rayon」约束保持）；目录间完全
//! 独立，`scan_one_dir` 为纯函数单元；输出 items/empty_dirs/unauthorized_dirs
//! 全局按路径排序（确定性跨运行一致）。

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::error::NcmError;

// ---------------------------------------------------------------- 规则卡 --

/// 清洗规则风险等级。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    Low,
    Medium,
    High,
}

/// 清洗规则卡（规则即数据：GUI/文档/执行器共用一份定义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleCard {
    /// 稳定规则 ID（如 `MF-CLEAN-001`）
    pub id: &'static str,
    pub description: &'static str,
    pub risk: Risk,
    /// 默认是否启用
    pub default_enabled: bool,
    /// 执行后可否还原（进回收站即可还原）
    pub reversible: bool,
}

/// 内置清洗规则卡（v0.3.0 全集；新增规则必须先在此登记）。
pub const RULE_CARDS: &[RuleCard] = &[
    RuleCard {
        id: "MF-CLEAN-001",
        description: "系统垃圾文件：Thumbs.db / .DS_Store / desktop.ini",
        risk: Risk::Low,
        default_enabled: true,
        reversible: true,
    },
    RuleCard {
        id: "MF-CLEAN-002",
        description: "临时/未完成下载：*.tmp / *.part / *.download / *.crdownload",
        risk: Risk::Low,
        default_enabled: true,
        reversible: true,
    },
    RuleCard {
        id: "MF-CLEAN-003",
        description: "零字节文件",
        risk: Risk::Low,
        default_enabled: true,
        reversible: true,
    },
    RuleCard {
        id: "MF-CLEAN-004",
        description: "空目录（扫描结束后统一收集，清洗阶段最后删除）",
        risk: Risk::Low,
        default_enabled: true,
        reversible: true,
    },
    RuleCard {
        id: "MF-CLEAN-005",
        description: "孤立歌词：.lrc 无同名音频文件",
        risk: Risk::Medium,
        default_enabled: true,
        reversible: true,
    },
    RuleCard {
        id: "MF-CLEAN-006",
        description: "孤立封面：图片文件所在目录无任何音频文件",
        risk: Risk::Medium,
        default_enabled: true,
        reversible: true,
    },
    RuleCard {
        id: "MF-CLEAN-007",
        description: "文件名含 Windows 非法字符或控制字符",
        risk: Risk::Medium,
        default_enabled: true,
        reversible: true,
    },
    RuleCard {
        id: "MF-CLEAN-008",
        description: "路径过长（> 260 字符，Windows MAX_PATH 风险）",
        risk: Risk::Medium,
        default_enabled: true,
        reversible: true,
    },
    RuleCard {
        id: "MF-CLEAN-009",
        description: "疑似乱码：文件名含 U+FFFD 替换符（GBK 转码失败的典型残留）",
        risk: Risk::Medium,
        default_enabled: true,
        reversible: true,
    },
];

pub fn rule_card(id: &str) -> Option<&'static RuleCard> {
    RULE_CARDS.iter().find(|c| c.id == id)
}

// ---------------------------------------------------------------- 扫描 --

/// 扫描选项。
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// 路径长度告警阈值（字符数）
    pub max_path_chars: usize,
    /// 递归深度上限（防符号链接环与异常深树）
    pub max_depth: usize,
    /// 是否递归子目录（false = 只扫根目录一层）
    pub recursive: bool,
    /// X25：并行 walker 工作线程数（0 = 自动 `min(cpus, 8)`；1 = 单线程等价）
    pub parallel_jobs: usize,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            max_path_chars: 260,
            max_depth: 64,
            recursive: true,
            parallel_jobs: 0,
        }
    }
}

/// 文件类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Audio,
    Lyrics,
    Cover,
    Junk,
    Other,
}

/// 单条扫描发现。
#[derive(Debug, Clone)]
pub struct ScanItem {
    pub path: PathBuf,
    pub category: Category,
    /// 命中的清洗规则 ID（仅 Junk/异常项有）
    pub rule_id: Option<&'static str>,
    pub size: u64,
    /// 修改时间（UNIX 秒；不可得为 `None`——D17 缓存对 `None` 永远 miss）
    pub mtime: Option<i64>,
}

/// 扫描报告：分类明细 + 计数汇总。
#[derive(Debug, Default)]
pub struct ScanReport {
    pub items: Vec<ScanItem>,
    pub audio: usize,
    pub lyrics: usize,
    pub covers: usize,
    pub junk: usize,
    pub other: usize,
    pub empty_dirs: Vec<PathBuf>,
    pub scanned_files: usize,
    pub scanned_dirs: usize,
    /// 各规则命中数（仅启用的规则）
    pub rule_hits: BTreeMap<&'static str, usize>,
    /// P8（fnOS 授权模型）：权限被拒（未授权/只读不足）而无法进入的目录清单。
    /// 扫描不中断（部分授权语义：其余目录照常），由调用方聚合呈现
    /// `MF-DIR-NOT-AUTHORIZED` + 授权引导文案。
    pub unauthorized_dirs: Vec<PathBuf>,
}

impl ScanReport {
    /// 按规则 ID 取命中明细。
    pub fn items_by_rule(&self, id: &str) -> Vec<&ScanItem> {
        self.items
            .iter()
            .filter(|i| i.rule_id == Some(id))
            .collect()
    }

    /// 汇总为 (类别, 数量) 表（报告输出用）。
    pub fn summary(&self) -> BTreeMap<String, usize> {
        let mut m = BTreeMap::new();
        m.insert("audio".into(), self.audio);
        m.insert("lyrics".into(), self.lyrics);
        m.insert("covers".into(), self.covers);
        m.insert("junk".into(), self.junk);
        m.insert("other".into(), self.other);
        m.insert("empty_dirs".into(), self.empty_dirs.len());
        m
    }
}

fn is_audio_ext(ext: &str) -> bool {
    matches!(
        ext,
        "mp3" | "flac" | "m4a" | "aac" | "ogg" | "opus" | "wav" | "ape" | "wv" | "wma"
    )
}

fn is_cover_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "cover.jpg"
        || n == "cover.png"
        || n == "folder.jpg"
        || n == "folder.png"
        || n == "albumart.jpg"
        || n == "front.jpg"
        || n.ends_with(".jpg")
        || n.ends_with(".jpeg")
        || n.ends_with(".png")
}

fn is_junk_name(name: &str) -> Option<&'static str> {
    let n = name.to_ascii_lowercase();
    if n == "thumbs.db" || n == ".ds_store" || n == "desktop.ini" {
        return Some("MF-CLEAN-001");
    }
    for ext in [".tmp", ".part", ".download", ".crdownload"] {
        if n.ends_with(ext) {
            return Some("MF-CLEAN-002");
        }
    }
    None
}

fn has_illegal_chars(name: &str) -> bool {
    name.chars()
        .any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') || (c as u32) < 0x20)
}

fn has_replacement_char(name: &str) -> bool {
    name.contains('\u{FFFD}')
}

/// 单目录扫描产物（X25：目录间完全独立——并行化的纯函数单元）。
struct DirOutcome {
    dir: PathBuf,
    scanned_files: usize,
    audio: usize,
    lyrics: usize,
    covers: usize,
    junk: usize,
    other: usize,
    rule_hits: BTreeMap<&'static str, usize>,
    items: Vec<ScanItem>,
    empty_dir: bool,
    unauthorized: bool,
    /// 本目录音频 stems（孤儿歌词判定用）
    dir_audio: HashSet<String>,
    child_dirs: Vec<PathBuf>,
}

/// 处理单个目录：readdir + 逐文件分类 + 收集子目录（X25 抽取自原 while 体内，
/// 纯函数——除 fs 读取外无共享态）。
fn scan_one_dir(dir: PathBuf, root: &Path, options: &ScanOptions) -> DirOutcome {
    let mut out = DirOutcome {
        dir: dir.clone(),
        scanned_files: 0,
        audio: 0,
        lyrics: 0,
        covers: 0,
        junk: 0,
        other: 0,
        rule_hits: BTreeMap::new(),
        items: Vec::new(),
        empty_dir: false,
        unauthorized: false,
        dir_audio: HashSet::new(),
        child_dirs: Vec::new(),
    };
    // 约定目录剪枝：`.musicforge/`（回收站/清单/回滚清单等工具自身状态）
    // 绝不进入扫描结果——否则去重会把回收站副本当新重复组、organize 会
    // 把待还原文件搬走（真机实测发现，G5「兜底伪装」同族教训）。
    if dir.file_name().and_then(|n| n.to_str()) == Some(".musicforge") {
        return out;
    }
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            // P8（fnOS 授权模型）：未授权目录显式上报（部分授权语义：
            // 不中断扫描）；调用方按 `MF-DIR-NOT-AUTHORIZED` 聚合呈现
            out.unauthorized = true;
            return out;
        }
        Err(_) => return out,
    };
    let mut file_count = 0usize;
    for entry in rd.flatten() {
        // 不跟随符号链接（symlink_metadata 只取链接本身）
        let Ok(md) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if md.is_dir() {
            out.child_dirs.push(entry.path());
            continue;
        }
        if !md.is_file() {
            continue; // 符号链接等非常规条目跳过
        }
        file_count += 1;
        out.scanned_files += 1;

        let name = entry.file_name().to_string_lossy().into_owned();
        let size = md.len();
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);
        let ext = Path::new(&name)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());

        let (category, rule): (Category, Option<&'static str>) =
            if let Some(rule) = is_junk_name(&name) {
                (Category::Junk, Some(rule))
            } else if size == 0 {
                (Category::Junk, Some("MF-CLEAN-003"))
            } else if ext.as_deref() == Some("lrc") {
                (Category::Lyrics, None)
            } else if ext.as_deref().map(is_audio_ext).unwrap_or(false) {
                (Category::Audio, None)
            } else if is_cover_name(&name) {
                (Category::Cover, None)
            } else {
                (Category::Other, None)
            };

        // 记录音频 stem（孤儿判定用）
        if category == Category::Audio {
            if let Some(stem) = Path::new(&name).file_stem().and_then(|s| s.to_str()) {
                out.dir_audio.insert(stem.to_string());
            }
        }

        let full = dir.join(&name);
        let path_chars = full.to_string_lossy().chars().count();

        let mut rule_final = rule;
        if category != Category::Junk {
            // 非垃圾文件也可命中异常规则（优先级：文件名异常 > 零字节已判）
            if has_illegal_chars(&name) {
                rule_final = Some("MF-CLEAN-007");
            } else if has_replacement_char(&name) {
                rule_final = Some("MF-CLEAN-009");
            } else if path_chars > options.max_path_chars {
                rule_final = Some("MF-CLEAN-008");
            } else if size == 0 {
                rule_final = Some("MF-CLEAN-003");
            }
        }

        match category {
            Category::Audio => out.audio += 1,
            Category::Lyrics => out.lyrics += 1,
            Category::Cover => out.covers += 1,
            Category::Junk => out.junk += 1,
            Category::Other => out.other += 1,
        }
        if let Some(rid) = rule_final {
            *out.rule_hits.entry(rid).or_default() += 1;
        }
        out.items.push(ScanItem {
            path: full,
            category,
            rule_id: rule_final,
            size,
            mtime,
        });
    }
    // 空目录（无文件也无子目录）
    out.empty_dir = file_count == 0 && out.child_dirs.is_empty() && dir != root;
    out
}

/// X25 并行 walker 调度态（队列 + 未完成任务计数共享一把锁；Condvar 免忙等）。
struct WalkState {
    queue: VecDeque<(PathBuf, usize)>,
    /// 队列中 + 处理中的任务总数（归零 = 扫描完成）
    remaining: usize,
    outcomes: Vec<DirOutcome>,
}

/// 递归扫描 `root`：分类文件、收集垃圾与异常项、记录空目录。
///
/// 只读，不改动任何文件；符号链接一律不跟随；超深目录按 `options.max_depth` 截断。
///
/// X25 并行化：目录间完全独立 → 标准库 `thread::scope` 有界 worker 池
///（`parallel_jobs`，0 = 自动 `min(cpus, 8)`；1 = 单线程等价）——**零新依赖**。
/// 输出确定性：`items`/`empty_dirs`/`unauthorized_dirs` 全局按路径排序
///（原 DFS 栈序依赖实现细节，排序后跨运行/跨并行度逐字节一致）。
pub fn scan_library(root: &Path, options: &ScanOptions) -> Result<ScanReport, NcmError> {
    if !root.is_dir() {
        return Err(NcmError::Db(format!(
            "scan: 目录不存在或不可读: {}",
            root.display()
        )));
    }
    let jobs = if options.parallel_jobs == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .min(8)
    } else {
        options.parallel_jobs
    };
    let state = std::sync::Mutex::new(WalkState {
        queue: VecDeque::from(vec![(root.to_path_buf(), 0usize)]),
        remaining: 1,
        outcomes: Vec::new(),
    });
    let cv = std::sync::Condvar::new();

    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                loop {
                    // 取任务（队列空且 remaining>0 → 等；remaining==0 → 退出）
                    let task = {
                        let mut st = state.lock().unwrap();
                        loop {
                            if let Some(t) = st.queue.pop_front() {
                                break Some(t);
                            }
                            if st.remaining == 0 {
                                break None;
                            }
                            st = cv.wait(st).unwrap();
                        }
                    };
                    let Some((dir, depth)) = task else { break };
                    let outcome = scan_one_dir(dir.clone(), root, options);
                    let mut st = state.lock().unwrap();
                    st.remaining -= 1;
                    if depth < options.max_depth && options.recursive {
                        for d in outcome.child_dirs.clone() {
                            st.queue.push_back((d, depth + 1));
                            st.remaining += 1;
                        }
                    }
                    st.outcomes.push(outcome);
                    cv.notify_all();
                }
            });
        }
    });

    let st = state.into_inner().unwrap();
    let mut report = ScanReport::default();
    // dir path -> (audio stems set, has_audio)（孤儿歌词判定，跨目录合并）
    let mut dir_audio: HashMap<PathBuf, HashSet<String>> = HashMap::new();
    for o in &st.outcomes {
        report.scanned_dirs += 1;
        report.scanned_files += o.scanned_files;
        report.audio += o.audio;
        report.lyrics += o.lyrics;
        report.covers += o.covers;
        report.junk += o.junk;
        report.other += o.other;
        for (rid, n) in &o.rule_hits {
            *report.rule_hits.entry(rid).or_default() += n;
        }
        if !o.dir_audio.is_empty() {
            dir_audio.insert(o.dir.clone(), o.dir_audio.clone());
        }
        if o.empty_dir {
            report.empty_dirs.push(o.dir.clone());
        }
        if o.unauthorized {
            report.unauthorized_dirs.push(o.dir.clone());
        }
    }
    // X25 输出确定性：items / empty_dirs / unauthorized_dirs 全局按路径排序
    report.items = st
        .outcomes
        .iter()
        .flat_map(|o| o.items.iter().cloned())
        .collect::<Vec<_>>();
    report.items.sort_by(|a, b| a.path.cmp(&b.path));
    report.empty_dirs.sort();
    report.unauthorized_dirs.sort();
    let _ = &mut dir_audio; // 下方孤儿判定沿用

    // 孤儿歌词：.lrc 的 stem 在同目录无音频
    let audio_stems =
        |dir: &Path| -> HashSet<String> { dir_audio.get(dir).cloned().unwrap_or_default() };
    let mut orphan_lyrics: Vec<ScanItem> = Vec::new();
    for item in report
        .items
        .iter()
        .filter(|i| i.category == Category::Lyrics)
    {
        let dir = item.path.parent().unwrap_or(Path::new(""));
        let stem = item.path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if !audio_stems(dir).contains(stem) {
            orphan_lyrics.push(ScanItem {
                path: item.path.clone(),
                category: Category::Junk,
                rule_id: Some("MF-CLEAN-005"),
                size: item.size,
                mtime: item.mtime,
            });
        }
    }
    for _ in &orphan_lyrics {
        *report.rule_hits.entry("MF-CLEAN-005").or_default() += 1;
        report.junk += 1;
    }
    report.items.extend(orphan_lyrics);

    // 孤立封面：封面所在目录无任何音频
    let mut orphan_covers: Vec<ScanItem> = Vec::new();
    for item in report
        .items
        .iter()
        .filter(|i| i.category == Category::Cover)
    {
        let dir = item.path.parent().unwrap_or(Path::new(""));
        if audio_stems(dir).is_empty() && dir_audio.get(dir).map(|s| s.is_empty()).unwrap_or(true) {
            orphan_covers.push(ScanItem {
                path: item.path.clone(),
                category: Category::Junk,
                rule_id: Some("MF-CLEAN-006"),
                size: item.size,
                mtime: item.mtime,
            });
        }
    }
    for _ in &orphan_covers {
        *report.rule_hits.entry("MF-CLEAN-006").or_default() += 1;
        report.junk += 1;
    }
    report.items.extend(orphan_covers);

    // 零字节与非法字符等规则的 rule_hits 已在分类时计入；空目录单独计入
    if !report.empty_dirs.is_empty() {
        report
            .rule_hits
            .entry("MF-CLEAN-004")
            .insert_entry(report.empty_dirs.len());
    }

    Ok(report)
}

// ------------------------------------------------------- D17 增量哈希缓存 --

/// [`refresh_hash_cache`] 的统计结果。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HashRefreshStats {
    /// 参与判定的音频文件数
    pub considered: usize,
    /// size+mtime 命中缓存，复用已有 sha256（零文件读取）
    pub cache_hits: usize,
    /// 未命中，流式重算 sha256 并回写缓存
    pub hashed: usize,
    /// 无法缓存（mtime 不可得 / 文件读取失败）
    pub skipped: usize,
}

/// D17 增量指纹接线：把音频文件的 sha256 哈希缓存刷新进状态库。
///
/// - **命中**（size+mtime 与缓存行一致，见 [`crate::db::Db::cached_hash`]）
///   → 直接复用，零文件读取——二次扫描的成本只剩元数据遍历；
/// - **未命中** → 流式读取一次计算 sha256 并回写 `files` 表；
/// - mtime 不可得的文件退化为占位索引行（与旧行为一致），永远 miss。
///
/// 只读音乐文件、只写可再生缓存（db）；db 读写失败**不 panic 不传播**
/// （命中判定失败按未命中处理、回写失败忽略）——缓存失败绝不影响扫描结论。
pub fn refresh_hash_cache(db: &crate::db::Db, items: &[ScanItem]) -> HashRefreshStats {
    let mut st = HashRefreshStats::default();
    for item in items {
        if item.category != Category::Audio {
            continue;
        }
        st.considered += 1;
        let key = item.path.to_string_lossy().into_owned();
        let ext = item.path.extension().and_then(|e| e.to_str());
        let Some(mtime) = item.mtime else {
            // 退化：占位索引（mtime None 的行永远不可作缓存依据）
            let _ = db.upsert_file(&key, item.size as i64, None, ext, None);
            st.skipped += 1;
            continue;
        };
        let hit = matches!(db.cached_hash(&key, item.size as i64, mtime), Ok(Some(_)));
        if hit {
            st.cache_hits += 1;
            continue;
        }
        match sha256_file_stream(&item.path) {
            Some(sha) => {
                st.hashed += 1;
                let _ = db.upsert_file(&key, item.size as i64, Some(mtime), ext, Some(&sha));
            }
            None => st.skipped += 1,
        }
    }
    st
}

/// 流式计算文件 sha256（大文件友好）；读取失败返回 `None`。
pub fn sha256_file_stream(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let Ok(mut f) = std::fs::File::open(path) else {
        return None;
    };
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h).ok()?;
    Some(format!("{:x}", h.finalize()))
}

// ---------------------------------------------------------------- 清洗计划 --

/// 清洗动作：把目标移入回收站（保留相对结构），可整体还原。
#[derive(Debug, Clone)]
pub struct CleanAction {
    pub path: PathBuf,
    pub rule_id: &'static str,
}

/// 清洗计划（dry-run 产物；apply 阶段逐条执行）。
#[derive(Debug, Default)]
pub struct CleanPlan {
    pub actions: Vec<CleanAction>,
    pub empty_dirs: Vec<PathBuf>,
    /// 回收站根目录（apply 时创建 `<trash>/<task_id>/...`）
    pub trash_root: PathBuf,
    /// 扫描根目录（回收站内按相对结构保留，便于还原）
    pub scan_root: PathBuf,
}

/// 依据报告与启用的规则集生成清洗计划（不改动任何文件）。
pub fn build_clean_plan(
    report: &ScanReport,
    enabled_rules: &HashSet<&'static str>,
    trash_root: &Path,
    scan_root: &Path,
) -> CleanPlan {
    let mut plan = CleanPlan {
        trash_root: trash_root.to_path_buf(),
        scan_root: scan_root.to_path_buf(),
        ..Default::default()
    };
    for item in &report.items {
        if let Some(rid) = item.rule_id {
            if enabled_rules.contains(rid) && item.path.exists() {
                plan.actions.push(CleanAction {
                    path: item.path.clone(),
                    rule_id: rid,
                });
            }
        }
    }
    plan.empty_dirs = report.empty_dirs.clone();
    plan
}

/// 清洗执行结果。
#[derive(Debug, Default)]
pub struct CleanOutcome {
    pub moved: usize,
    pub dirs_removed: usize,
    /// 回滚清单路径（`<trash>/<task_id>/rollback.jsonl`；from↔to 可整体还原）
    pub rollback_manifest: Option<PathBuf>,
}

/// 执行清洗计划：把动作目标**移入回收站**（保留相对结构），
/// 写回滚清单，最后自浅至深移除空目录。绝不直接删除任何文件。
pub fn apply_clean_plan(plan: &CleanPlan, task_id: &str) -> Result<CleanOutcome, NcmError> {
    let mut outcome = CleanOutcome::default();
    let trash = plan.trash_root.join(task_id);
    std::fs::create_dir_all(&trash)?;
    let rollback = trash.join("rollback.jsonl");
    let mut rb = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&rollback)?;

    for action in &plan.actions {
        let rel = action
            .path
            .strip_prefix(&plan.scan_root)
            .unwrap_or(action.path.as_path());
        let dest = trash.join(rel);
        // 稳定审计 B3（2026-09-08）：回收站内同名碰撞（同名文件二次清洗）曾使
        // rename 失败 → 整批中断、部分移动。修复：冲突时追加 " (n)" 后缀落位
        // （审计行 from=实际落位，还原语义不受影响）。
        let dest = if dest.exists() {
            let stem = dest
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("file")
                .to_string();
            let ext = dest
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            let mut n = 2usize;
            let mut cand = dest.with_file_name(format!("{stem} ({n}){ext}"));
            while cand.exists() {
                n += 1;
                cand = dest.with_file_name(format!("{stem} ({n}){ext}"));
            }
            cand
        } else {
            dest
        };
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&action.path, &dest)?;
        let line = serde_json::json!({
            "from": dest.display().to_string(),
            "to": action.path.display().to_string(),
            "rule": action.rule_id,
        });
        writeln!(rb, "{}", serde_json::to_string(&line)?)?;
        outcome.moved += 1;
    }
    outcome.rollback_manifest = Some(rollback);

    // 空目录：自深至浅移除（只删本次扫描确认的空目录）
    let mut dirs = plan.empty_dirs.clone();
    dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for d in dirs {
        if std::fs::remove_dir(&d).is_ok() {
            outcome.dirs_removed += 1;
        }
    }
    Ok(outcome)
}

/// 从回收站回滚：按 rollback.jsonl 逆向搬回（用于误清洗恢复）。
///
/// 稳定审计 B2（2026-09-08）：原实现目标被同名文件占用（用户重建）时 rename
/// 失败 → 整个还原**中途失败**（部分还原状态混乱）。修复：
/// - `from` 已不存在（回收站内被手动清理）→ 跳过该行，不再中断后续还原；
/// - `to` 已存在 → 还原为邻位 `name (restored[-n]).ext`，**绝不覆盖占用者**，
///   数据零丢失，整体还原语义保持。
pub fn restore_from_trash(rollback_manifest: &Path) -> Result<usize, NcmError> {
    let text = std::fs::read_to_string(rollback_manifest)?;
    let mut n = 0usize;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: serde_json::Value = serde_json::from_str(line)?;
        let from = v["from"].as_str().unwrap_or_default();
        let to = v["to"].as_str().unwrap_or_default();
        if to.is_empty() {
            continue;
        }
        let from = Path::new(from);
        if !from.is_file() {
            continue; // 回收站内已被清理：跳过，不中断整体还原
        }
        let to = Path::new(to);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let final_to = if to.exists() {
            let stem = to
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("file")
                .to_string();
            let ext = to
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            let mut n = 1usize;
            let mut cand = to.with_file_name(format!("{stem} (restored){ext}"));
            while cand.exists() {
                n += 1;
                cand = to.with_file_name(format!("{stem} (restored-{n}){ext}"));
            }
            cand
        } else {
            to.to_path_buf()
        };
        std::fs::rename(from, &final_to)?;
        n += 1;
    }
    Ok(n)
}
