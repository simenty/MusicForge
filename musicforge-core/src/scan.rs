//! 曲库扫描与清洗（P3）。
//!
//! 设计要点（对齐 ROADMAP §5 P3 与治理 §4.13）：
//!
//! - **只读扫描**：[`scan_library`] 递归遍历目录树并分类，不改动任何文件；
//! - **规则卡**：每条清洗规则有 ID/描述/风险/默认启停/可逆性（`RULE_CARDS`），
//!   与 GUI/文档共用一份定义（规则即数据）；
//! - **清洗计划**：[`build_clean_plan`] 依据启用的规则生成动作清单；
//!   执行动作 = **移入回收站目录**（保留相对结构），可整体还原——绝不直接删除；
//! - **P4 增量指纹**：[`FileFingerprint`] + [`fingerprint`] 提供
//!   size+mtime（L1）→ 内容哈希（L2）的两级原语，供 dedupe 与 db 缓存复用。
//!
//! 刻意不做的事：不引入 walkdir/ignore/rayon（依赖面最小）；walker 为
//! 有界深度的迭代实现，符号链接一律不跟随（防环）。

use std::collections::{BTreeMap, HashMap, HashSet};
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
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            max_path_chars: 260,
            max_depth: 64,
            recursive: true,
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

/// 递归扫描 `root`：分类文件、收集垃圾与异常项、记录空目录。
///
/// 只读，不改动任何文件；符号链接一律不跟随；超深目录按 `options.max_depth` 截断。
pub fn scan_library(root: &Path, options: &ScanOptions) -> Result<ScanReport, NcmError> {
    let mut report = ScanReport::default();
    if !root.is_dir() {
        return Err(NcmError::Db(format!(
            "scan: 目录不存在或不可读: {}",
            root.display()
        )));
    }

    // dir path -> (audio stems set, has_audio)
    let mut dir_audio: HashMap<PathBuf, HashSet<String>> = HashMap::new();

    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        report.scanned_dirs += 1;
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut child_dirs: Vec<PathBuf> = Vec::new();
        let mut file_count = 0usize;

        for entry in rd.flatten() {
            // 不跟随符号链接（symlink_metadata 只取链接本身）
            let Ok(md) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if md.is_dir() {
                child_dirs.push(entry.path());
                continue;
            }
            if !md.is_file() {
                continue; // 符号链接等非常规条目跳过
            }
            file_count += 1;
            report.scanned_files += 1;

            let name = entry.file_name().to_string_lossy().into_owned();
            let size = md.len();
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
                    dir_audio
                        .entry(dir.clone())
                        .or_default()
                        .insert(stem.to_string());
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
                Category::Audio => report.audio += 1,
                Category::Lyrics => report.lyrics += 1,
                Category::Cover => report.covers += 1,
                Category::Junk => report.junk += 1,
                Category::Other => report.other += 1,
            }
            if let Some(rid) = rule_final {
                *report.rule_hits.entry(rid).or_default() += 1;
            }
            report.items.push(ScanItem {
                path: full,
                category,
                rule_id: rule_final,
                size,
            });
        }

        // 空目录（无文件也无子目录）
        if file_count == 0 && child_dirs.is_empty() && dir != root {
            report.empty_dirs.push(dir.clone());
        }

        if depth < options.max_depth && options.recursive {
            for d in child_dirs {
                stack.push((d, depth + 1));
            }
        }
    }

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
        if let Some(parent) = Path::new(to).parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(from, Path::new(to))?;
        n += 1;
    }
    Ok(n)
}
