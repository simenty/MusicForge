//! 曲库整理（P4.2）：按命名模板把音频文件归位到规范目录结构。
//!
//! 设计要点（对齐 ROADMAP P4 与 v2.4 方案 §P4）：
//!
//! - **复用 [`crate::template::render_filename`]**：渲染语义与 convert 完全
//!   同源——段清洗/保留设备名/长度双上限/空段回退全部一致，绝不另写一套；
//! - **元数据来源 = lofty 内嵌标签**（title/artist/album/track），无标签 →
//!   `None` → 模板回退（`未知艺术家 - <stem>`），与 convert 的无元数据路径同语义；
//! - **冲突策略**（目标已存在且不是同一文件）：`skip`（默认，报告并跳过）/
//!   `suffix`（` (2)`、` (3)`…，与 convert 同形）/ `overwrite-never`
//!   （该项计失败，稳定码 `MF-PATH-CONFLICT`）——**任何策略都绝不覆盖目标**；
//! - **已在位**（目标解析后与源是同一文件，Windows 大小写不敏感盘上按
//!   canonical 路径判定）→ 跳过不动，二次运行零改动；
//! - apply = `rename` 移动（跨盘 EXDX 显式失败，不做 copy+delete 兜底——
//!   兜底删除源文件是数据破坏面）+ rollback.jsonl（from=新位置 → to=原位置），
//!   复用 [`crate::scan::restore_from_trash`] 即可整体还原。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::NcmError;
use crate::scan::{scan_library, Category, ScanOptions};

/// 冲突策略（v2.4 §P4：默认 skip）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConflictStrategy {
    /// 目标已存在 → 报告并跳过（默认）
    #[default]
    Skip,
    /// 目标已存在 → 追加 ` (2)`、` (3)`… 直到可用（与 convert 同形）
    Suffix,
    /// 目标已存在 → 该项计失败，稳定码 `MF-PATH-CONFLICT`
    OverwriteNever,
}

impl ConflictStrategy {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "skip" => Some(Self::Skip),
            "suffix" => Some(Self::Suffix),
            "overwrite-never" => Some(Self::OverwriteNever),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Suffix => "suffix",
            Self::OverwriteNever => "overwrite-never",
        }
    }
}

/// organize 计划项状态（规划期确定；apply 只执行 `Planned`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrganizeStatus {
    /// 将移动到 target
    Planned,
    /// 源与目标是同一文件（已在规范位置）→ 永不动
    AlreadyInPlace,
    /// 冲突，按 skip 策略跳过
    SkippedConflict,
    /// 冲突，按 overwrite-never 策略计失败（`MF-PATH-CONFLICT`）
    ConflictNever,
}

/// 单条整理计划项。
#[derive(Debug, Clone)]
pub struct OrganizeItem {
    pub source: PathBuf,
    pub target: PathBuf,
    pub status: OrganizeStatus,
    /// 说明（冲突/已在位/失败原因；给人看）
    pub note: Option<String>,
}

/// 整理计划。
#[derive(Debug, Default)]
pub struct OrganizePlan {
    pub items: Vec<OrganizeItem>,
    pub target_root: PathBuf,
    pub strategy: ConflictStrategy,
    pub template: String,
}

/// 计划项状态计数（报告用）。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OrganizeCounts {
    pub planned: usize,
    pub in_place: usize,
    pub skipped_conflict: usize,
    pub conflict_never: usize,
}

impl OrganizePlan {
    pub fn counts(&self) -> OrganizeCounts {
        let mut c = OrganizeCounts::default();
        for i in &self.items {
            match i.status {
                OrganizeStatus::Planned => c.planned += 1,
                OrganizeStatus::AlreadyInPlace => c.in_place += 1,
                OrganizeStatus::SkippedConflict => c.skipped_conflict += 1,
                OrganizeStatus::ConflictNever => c.conflict_never += 1,
            }
        }
        c
    }
}

/// 整理执行结果。
#[derive(Debug, Default)]
pub struct OrganizeOutcome {
    pub moved: usize,
    pub skipped: usize,
    pub failed: usize,
    /// 回滚清单（from=新位置 → to=原位置）；整体还原走
    /// [`crate::scan::restore_from_trash`]。
    pub rollback_manifest: Option<PathBuf>,
}

/// organize 选项。
#[derive(Debug, Clone)]
pub struct OrganizeOptions<'a> {
    pub template: &'a str,
    /// 目标根目录（与源相同 = 原地整理）
    pub target_root: &'a Path,
    pub conflict: ConflictStrategy,
}

/// 从音频文件的内嵌标签提取元数据（organize 渲染输入）。
///
/// 无标签/解析失败 → `None`（模板走回退分支，与 convert 无元数据同语义）。
/// 这里的解析失败**不是错误**：organize 的对象是任意健康音频，缺标签只影响命名。
fn audio_metadata_of(path: &Path, ext: &str) -> Option<crate::metadata::Metadata> {
    use lofty::prelude::*;
    let tagged = lofty::read_from_path(path).ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    let props = tagged.properties();
    Some(crate::metadata::Metadata {
        name: tag
            .get_string(lofty::tag::ItemKey::TrackTitle)
            .map(str::to_string),
        artist: tag
            .get_string(lofty::tag::ItemKey::TrackArtist)
            .map(str::to_string),
        album: tag
            .get_string(lofty::tag::ItemKey::AlbumTitle)
            .map(str::to_string),
        format: Some(ext.to_string()),
        track: tag.track().map(|t| t as u64),
        bitrate: props.audio_bitrate().map(|b| b as u64),
        duration: Some(props.duration().as_millis() as u64),
        album_pic_url: None,
    })
}

/// 计划整理：只读扫描 + 渲染目标 + 冲突判定（不改任何文件）。
pub fn plan_organize(root: &Path, options: &OrganizeOptions) -> Result<OrganizePlan, NcmError> {
    if !root.is_dir() {
        return Err(NcmError::Db(format!(
            "organize: 源目录不存在或不可读: {}",
            root.display()
        )));
    }
    let scan = scan_library(root, &ScanOptions::default())?;

    // 同 stem 的冲突后缀计数（suffix 策略下与 convert 同形地续编号）
    let mut suffix_used: BTreeMap<PathBuf, u32> = BTreeMap::new();
    let mut plan = OrganizePlan {
        target_root: options.target_root.to_path_buf(),
        strategy: options.conflict,
        template: options.template.to_string(),
        items: Vec::new(),
    };

    for item in scan.items.iter().filter(|i| i.category == Category::Audio) {
        let source = &item.path;
        if !source.exists() {
            continue; // 规划只描述当前磁盘状态
        }
        let ext = source
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        let stem = source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let meta = audio_metadata_of(source, &ext);
        let rendered = crate::template::render_filename(options.template, meta.as_ref(), &stem);
        let file_name = if ext.is_empty() {
            rendered
        } else {
            format!("{rendered}.{ext}")
        };
        let target = options.target_root.join(&file_name);

        // 已在位：目标解析后与源为同一文件（Windows 大小写不敏感盘的
        // canonical 判定；unix 上不同大小写是两个文件，不会误判）
        if target == *source || same_file(source, &target) {
            plan.items.push(OrganizeItem {
                source: source.clone(),
                target,
                status: OrganizeStatus::AlreadyInPlace,
                note: Some("已在规范位置，无需移动".to_string()),
            });
            continue;
        }

        // 已在目标根内且当前名 = 渲染名（可带历史分配的 (N) 后缀）→ 已在位。
        // 真机实测发现的幂等性缺陷：suffix 策略落位的 "name (2).ext" 在二次
        // 规划时渲染回 "name.ext" ≠ 当前名 → 冲突 → 再次后缀 → 无限膨胀。
        // 语义：文件已在目标根内、且名字符合本次渲染（含既有后缀）即视为归位。
        if source
            .parent()
            .map(|p| p == options.target_root)
            .unwrap_or(false)
            && name_matches_with_optional_suffix(source, &target)
        {
            plan.items.push(OrganizeItem {
                source: source.clone(),
                target,
                status: OrganizeStatus::AlreadyInPlace,
                note: Some("已在规范位置（含既有后缀），无需移动".to_string()),
            });
            continue;
        }

        if target.exists() {
            match options.conflict {
                ConflictStrategy::Skip => plan.items.push(OrganizeItem {
                    source: source.clone(),
                    target,
                    status: OrganizeStatus::SkippedConflict,
                    note: Some("目标已存在（skip）".to_string()),
                }),
                ConflictStrategy::OverwriteNever => plan.items.push(OrganizeItem {
                    source: source.clone(),
                    target,
                    status: OrganizeStatus::ConflictNever,
                    note: Some(
                        "MF-PATH-CONFLICT: 目标已存在（overwrite-never，绝不覆盖）".to_string(),
                    ),
                }),
                ConflictStrategy::Suffix => {
                    // (2)、(3)… 编号全库连续（同名目标共享计数器，与 convert 同形；
                    // x.wav 本身即隐式 (1)，故从 (2) 起）
                    let counter = suffix_used.entry(target.clone()).or_insert(1);
                    let stem2 = target
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let e2 = target
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| format!(".{e}"))
                        .unwrap_or_default();
                    let mut num = *counter + 1;
                    let suffixed = loop {
                        let cand = target.with_file_name(format!("{stem2} ({num}){e2}"));
                        if !cand.exists() {
                            *counter = num;
                            break cand;
                        }
                        num += 1;
                    };
                    plan.items.push(OrganizeItem {
                        source: source.clone(),
                        note: Some(format!(
                            "目标已存在，按 suffix 策略改名为 {}",
                            suffixed.display()
                        )),
                        target: suffixed,
                        status: OrganizeStatus::Planned,
                    });
                }
            }
        } else {
            plan.items.push(OrganizeItem {
                source: source.clone(),
                target,
                status: OrganizeStatus::Planned,
                note: None,
            });
        }
    }
    Ok(plan)
}

/// `rename` 语义下的「同一文件」判定：目标存在且 canonical 路径相等。
fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

/// 源文件名是否等于渲染目标名，或等于其历史后缀形态 `stem (N).ext`（N≥1）。
///
/// 仅用于「源已在目标根内」的幂等判定——文件名本来就是工具上一轮分配的，
/// 视作已归位，防止 suffix 策略无限膨胀。
fn name_matches_with_optional_suffix(source: &Path, target: &Path) -> bool {
    let (Some(s), Some(t)) = (
        source.file_name().and_then(|f| f.to_str()),
        target.file_name().and_then(|f| f.to_str()),
    ) else {
        return false;
    };
    if s == t {
        return true;
    }
    let (t_stem, t_ext) = match t.rsplit_once('.') {
        Some((st, e)) if !st.is_empty() => (st, format!(".{e}")),
        _ => (t, String::new()),
    };
    let Some(rest) = s.strip_prefix(t_stem) else {
        return false;
    };
    let Some(inner) = rest.strip_prefix(" (") else {
        return false;
    };
    let Some(num) = inner.strip_suffix(&format!("){t_ext}")) else {
        return false;
    };
    !num.is_empty() && num.chars().all(|c| c.is_ascii_digit())
}

/// 执行整理计划：`Planned` 项 rename 移动 + 写回滚清单。
///
/// - **绝不覆盖**：目标存在（含 apply 间隙被外部创建）→ 该项失败，绝不 rename 覆盖；
/// - 跨盘移动（EXDEV）→ 该项失败（不做 copy+delete 兜底，删除源文件是破坏面）；
/// - 失败计入 failed 并继续（批处理语义：命令成功、逐项可失败）。
pub fn apply_organize_plan(
    plan: &OrganizePlan,
    task_id: &str,
) -> Result<OrganizeOutcome, NcmError> {
    let mut outcome = OrganizeOutcome::default();
    let rollback_dir = plan.target_root.join(".musicforge");
    std::fs::create_dir_all(&rollback_dir)?;
    let rollback = rollback_dir.join(format!("rollback-organize-{task_id}.jsonl"));
    let mut rb = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&rollback)?;

    for item in &plan.items {
        match item.status {
            OrganizeStatus::Planned => {
                if item.target.exists() {
                    // apply 间隙被外部创建：绝不覆盖
                    outcome.failed += 1;
                    continue;
                }
                if let Some(parent) = item.target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                match std::fs::rename(&item.source, &item.target) {
                    Ok(()) => {
                        let line = serde_json::json!({
                            "from": item.target.display().to_string(),
                            "to": item.source.display().to_string(),
                            "rule": "MF-ORGANIZE",
                        });
                        use std::io::Write as _;
                        writeln!(rb, "{}", serde_json::to_string(&line)?)?;
                        outcome.moved += 1;
                    }
                    Err(_) => {
                        // EXDEV（跨盘）/权限等：显式失败计数，绝不删源兜底
                        outcome.failed += 1;
                    }
                }
            }
            OrganizeStatus::AlreadyInPlace | OrganizeStatus::SkippedConflict => {
                outcome.skipped += 1;
            }
            OrganizeStatus::ConflictNever => {
                outcome.failed += 1;
            }
        }
    }
    outcome.rollback_manifest = Some(rollback);
    Ok(outcome)
}
