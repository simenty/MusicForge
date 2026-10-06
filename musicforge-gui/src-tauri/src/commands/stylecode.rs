//! 风格码（X15 / 蓝图能力 #11）：解析文件名的前导 `[Y23-S01-E01-C01-C02-V00]` 块。
//!
//! 解析能力全部来自 `musicforge_core::stylecode`（纯字符串解析，不读文件不查库）。
//!
//! **I18N-7（服务端文本语言中立）**：本命令只回**结构化数据**（年份 / 原始码 / 码名），
//! 字段标签（年份 / Year、风格 / Style…）由前端按当前 UI 语言渲染——服务端不产出
//! 中文显示文案。这与 `stylecode::display_with_locale` 同源同理：后者供 CLI 直接出
//! 文案（CLI 有自己的 locale），GUI 走「结构化数据 + 前端 i18n」更合适，也免于把
//! locale 传过 IPC。
//!
//! 码名来自**用户提供的 codebook**（`{"S01": "流行", ...}`）：查到才有条目，查不到
//! 前端回退原始码——**绝不编造**。未配置 codebook 时 `labels` 为空，行为同纯原始码。

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

/// 风格码解析结果（前端按 UI 语言渲染标签）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleCodeDto {
    /// `Y23` → 2023
    pub year: Option<i32>,
    /// `S01`（原始码）
    pub style: Option<String>,
    /// `E01`（情绪）
    pub mood: Option<String>,
    /// `C01` / `C02`（场景，可多个，保序）
    pub scenes: Vec<String>,
    /// `V00`（版本）
    pub version: Option<String>,
    /// 无法归类的 token（原样保留，结构兼容未来新增键）
    pub other: Vec<String>,
    /// 码 → 码名（仅**查到**的条目）；查不到则该码不在表中，前端回退原始码。
    /// 未配置 codebook 时为空表。
    pub labels: BTreeMap<String, String>,
}

/// 解析路径中的风格码；**无前导 `[...]` 码块 → `Ok(None)`**（不是错误：多数曲目无码）。
///
/// `codebookPath` 为可选的用户 codebook（设置项）：
/// - 传入但**读不到 / 格式非法 → 返回 Err**（显式失败，让界面提示路径有问题，
///   而不是静默退回原始码让人以为"没有译名"）；
/// - 不传 → `labels` 为空，前端全按原始码显示。
///
/// 纯字符串解析（不读音频、不查库、不发网络），只在打开卡片时按当前曲目调一次。
#[tauri::command]
pub fn style_code(
    path: String,
    codebook_path: Option<String>,
) -> Result<Option<StyleCodeDto>, String> {
    let map: BTreeMap<String, String> = match codebook_path.as_deref() {
        Some(p) if !p.trim().is_empty() => {
            musicforge_core::stylecode::load_genre_map(Path::new(p))?
        }
        _ => BTreeMap::new(),
    };

    let Some(sc) = musicforge_core::stylecode::parse_style_code(Path::new(&path)) else {
        return Ok(None);
    };

    // 只为本风格码**实际出现**的码建索引——避免把整个 codebook 塞给前端。
    let mut labels = BTreeMap::new();
    for code in sc
        .style
        .iter()
        .chain(sc.mood.iter())
        .chain(sc.scenes.iter())
        .chain(sc.version.iter())
        .chain(sc.other.iter())
    {
        if let Some(name) = map.get(code) {
            labels.insert(code.clone(), name.clone());
        }
    }

    Ok(Some(StyleCodeDto {
        year: sc.year,
        style: sc.style,
        mood: sc.mood,
        scenes: sc.scenes,
        version: sc.version,
        other: sc.other,
        labels,
    }))
}
