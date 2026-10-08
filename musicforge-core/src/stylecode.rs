//! 风格代码解析器（X15 / 蓝图能力 #11）：文件名 `[Y23-S01-E01-C01-C02-V00]`
//! → year/style/mood/scene/version 结构化字段（卡片标签 **locale 中立**，渲染时按
//! UI 语言展开，见 [`StyleCode::display_with_locale`]——数据层不嵌中文，英文界面不弹中文）。
//!
//! 代码语义（蓝图 v3.0 §2.2 #64 权威定义）：
//! - `Y##` → 年份（Y23 = 2023）
//! - `S##` → 风格（style，genre 的主键）
//! - `E##` → 情绪（mood）
//! - `C##` → 场景（scene，**可多个**）
//! - `V##` → 版本（version）
//!
//! 索引 → 人类可读名的 codebook 属于用户配置（飞牛风格页），L1 层不做
//! 猜测映射：`genre_label` 只按用户提供的映射表翻译，查不到就回退原始码
//! （绝不编造）。AI 兜底语义映射在 v0.7.0（L2）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::dedupe::Locale;
use crate::error::NcmError;

/// 解析结果（原始码保留；展示标签见 [`Self::display_with_locale`]，locale 中立）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StyleCode {
    /// `Y23` → Some(2023)
    pub year: Option<i32>,
    /// `S01` → "S01"（原始码）
    pub style: Option<String>,
    /// `E01` → "E01"
    pub mood: Option<String>,
    /// `C01`/`C02` → ["C01","C02"]（可多个，保序）
    pub scenes: Vec<String>,
    /// `V00` → "V00"
    pub version: Option<String>,
    /// 无法归类的 token 原样保留（结构兼容：未来新增键不丢信息）
    pub other: Vec<String>,
}

impl StyleCode {
    /// genre 标签值：**风格**优先按映射表翻译（查不到回退原始码）；
    /// 场景码附加在后面（同样映射→回退），形如 `流行 / 学习`。
    /// 无任何 style/scene → None（没有可写的东西，绝不写空 genre）。
    pub fn genre_label(&self, map: &BTreeMap<String, String>) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if let Some(s) = &self.style {
            parts.push(lookup(map, s));
        }
        for sc in &self.scenes {
            parts.push(lookup(map, sc));
        }
        if parts.is_empty() {
            return None;
        }
        Some(parts.join(" / "))
    }

    /// 展示标签（卡片显示）：**locale 中立**，渲染时按语言展开——不把中文塞进数据层
    /// （与 `dedupe::Locale` 同款思路，英文界面不再弹中文标签）。
    ///
    /// `map` 为用户提供的码→名 codebook（飞牛风格页）；查不到回退原始码，绝不编造。
    /// 项间分隔统一用 ` · `（标点，语言中立）；场景内多个码在中文用 `、`、英文用 `, `。
    pub fn display_with_locale(&self, map: &BTreeMap<String, String>, locale: Locale) -> String {
        let (l_year, l_style, l_mood, l_scene, l_version, l_other, inner_sep) = match locale {
            Locale::Zh => ("年份", "风格", "情绪", "场景", "版本", "其他", "、"),
            Locale::En => ("Year", "Style", "Mood", "Scene", "Version", "Other", ", "),
        };
        let mut items: Vec<String> = Vec::new();
        if let Some(y) = self.year {
            items.push(format!("{l_year}: {y}"));
        }
        if let Some(s) = &self.style {
            items.push(format!("{l_style}: {}", lookup(map, s)));
        }
        if let Some(m) = &self.mood {
            items.push(format!("{l_mood}: {}", lookup(map, m)));
        }
        if !self.scenes.is_empty() {
            let sc: Vec<String> = self.scenes.iter().map(|c| lookup(map, c)).collect();
            items.push(format!("{l_scene}: {}", sc.join(inner_sep)));
        }
        if let Some(v) = &self.version {
            items.push(format!("{l_version}: {}", lookup(map, v)));
        }
        for o in &self.other {
            items.push(format!("{l_other}: {o}"));
        }
        items.join(" · ")
    }

    /// 中文展示标签（[`Self::display_with_locale`] 的 `Zh` 便捷别名；保持旧测试/调用兼容）。
    pub fn display_cn(&self, map: &BTreeMap<String, String>) -> String {
        self.display_with_locale(map, Locale::Zh)
    }
}

fn lookup(map: &BTreeMap<String, String>, code: &str) -> String {
    map.get(code).cloned().unwrap_or_else(|| code.to_string())
}

/// 从文件名解析风格代码块。
///
/// 识别**前导** `[...]` 块（`[Y23-S01-E01-C01-C02-V00] 歌名.flac`）——
/// 这是蓝图约定的放置位置；非前导的方括号不误伤（避免把 `(Live Version)`
/// 之类用户命名当风格码）。
pub fn parse_style_code(path: &Path) -> Option<StyleCode> {
    let stem = path.file_stem().and_then(|s| s.to_str())?;
    let inner = stem.strip_prefix('[')?;
    let end = inner.find(']')?;
    let body = &inner[..end];
    if body.is_empty() {
        return None;
    }
    let mut sc = StyleCode::default();
    for tok in body.split('-') {
        let tok = tok.trim();
        if tok.is_empty() {
            continue;
        }
        let mut chars = tok.chars();
        let kind = chars.next().unwrap_or('\0');
        let digits = chars.as_str();
        let is_digits = !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit());
        if !is_digits {
            sc.other.push(tok.to_string());
            continue;
        }
        match kind {
            'Y' | 'y' => {
                // Y23 → 2023；Y99 → 2099；两位年份窗口：00-79 → 20xx，80-99 → 19xx
                let n: i32 = digits.parse().ok()?;
                sc.year = Some(if n < 80 { 2000 + n } else { 1900 + n });
            }
            'S' | 's' => sc.style = Some(tok.to_string()),
            'E' | 'e' => sc.mood = Some(tok.to_string()),
            'C' | 'c' => sc.scenes.push(tok.to_string()),
            'V' | 'v' => sc.version = Some(tok.to_string()),
            _ => sc.other.push(tok.to_string()),
        }
    }
    Some(sc)
}

/// 从映射文件加载 codebook（JSON：`{"S01": "流行", "C01": "学习", ...}`）。
///
/// 错误为 String：codebook 是 CLI 参数级输入，加载失败属用法错误
/// （退出码 2），不进 NcmError 稳定码族。
pub fn load_genre_map(path: &Path) -> Result<BTreeMap<String, String>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("failed to read codebook: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("codebook is not valid JSON: {e}"))?;
    let Some(obj) = v.as_object() else {
        return Err("codebook top level must be a JSON object".to_string());
    };
    let mut m = BTreeMap::new();
    for (k, val) in obj {
        let Some(s) = val.as_str() else {
            return Err(format!("codebook value must be a string: {k}"));
        };
        m.insert(k.clone(), s.to_string());
    }
    Ok(m)
}

// ---------------------------------------------------------------- genre 写入 --

/// 单文件 genre 写入决策。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenreDecision {
    /// 将写入（写码 → genre 值）
    WillWrite { genre: String },
    /// 已有 genre 且未开启 replace_all（FillMissingOnly：绝不覆盖用户数据）
    HasGenre,
    /// 文件名无风格码块
    NoCode,
    /// 有码块但无 style/scene 可写
    NoLabel,
}

/// genre 写入计划（规划期只读；apply 才落盘）。
#[derive(Debug, Default)]
pub struct GenrePlan {
    pub items: Vec<(PathBuf, GenreDecision)>,
}

impl GenrePlan {
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        let mut will = 0;
        let mut has = 0;
        let mut no_code = 0;
        let mut no_label = 0;
        for (_, d) in &self.items {
            match d {
                GenreDecision::WillWrite { .. } => will += 1,
                GenreDecision::HasGenre => has += 1,
                GenreDecision::NoCode => no_code += 1,
                GenreDecision::NoLabel => no_label += 1,
            }
        }
        (will, has, no_code, no_label)
    }
}

/// 规划 genre 写入：扫描 → 解析文件名风格码 → 判定（只读）。
///
/// `replace_all = false`（FillMissingOnly，蓝图 P2 字段级写策略默认档）时，
/// 已有非空 genre 的文件跳过——**绝不覆盖用户已有数据**。
pub fn plan_genre_writes(
    root: &Path,
    map: &BTreeMap<String, String>,
    replace_all: bool,
) -> Result<GenrePlan, NcmError> {
    let scan = crate::scan::scan_library(root, &crate::scan::ScanOptions::default())?;
    let mut plan = GenrePlan::default();
    for item in scan
        .items
        .iter()
        .filter(|i| i.category == crate::scan::Category::Audio)
    {
        let decision = match parse_style_code(&item.path) {
            None => GenreDecision::NoCode,
            Some(code) => match code.genre_label(map) {
                None => GenreDecision::NoLabel,
                Some(genre) => {
                    if !replace_all && read_genre_tag(&item.path).is_some_and(|g| !g.is_empty()) {
                        GenreDecision::HasGenre
                    } else {
                        GenreDecision::WillWrite { genre }
                    }
                }
            },
        };
        plan.items.push((item.path.clone(), decision));
    }
    Ok(plan)
}

/// 读取现有 genre 标签（无标签/解析失败 → None）。
///
/// 公开供 GUI / CLI 展示「文件里现在是什么 genre」，与解析出的风格码对照
/// （写回前可先看现状；`plan_genre_writes` 内部也用它判 FillMissingOnly）。
pub fn read_genre_tag(path: &Path) -> Option<String> {
    use lofty::prelude::*;
    let tagged = lofty::read_from_path(path).ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    tag.get_string(lofty::tag::ItemKey::Genre)
        .map(str::to_string)
}

/// 执行 genre 写入计划（只写 `WillWrite` 项；失败计数继续）。
///
/// 返回 (written, failed)。写失败显式计数——绝不谎报成功。
pub fn apply_genre_writes(plan: &GenrePlan) -> (usize, usize) {
    use lofty::prelude::*;
    let mut written = 0usize;
    let mut failed = 0usize;
    for (path, decision) in &plan.items {
        let GenreDecision::WillWrite { genre } = decision else {
            continue;
        };
        let res = (|| -> Result<(), Box<dyn std::error::Error>> {
            let mut tagged = lofty::read_from_path(path)?;
            let ttype = tagged.primary_tag_type();
            if tagged.tag(ttype).is_none() {
                tagged.insert_tag(lofty::tag::Tag::new(ttype));
            }
            let Some(tag) = tagged.tag_mut(ttype) else {
                return Err("cannot obtain mutable tag".into());
            };
            tag.insert_text(lofty::tag::ItemKey::Genre, genre.clone());
            tagged.save_to_path(path, lofty::config::WriteOptions::default())?;
            Ok(())
        })();
        match res {
            Ok(()) => written += 1,
            Err(_) => failed += 1,
        }
    }
    (written, failed)
}
