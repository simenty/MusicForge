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
//!
//! 两个命令：`style_code`（单曲，卡片用）/ `style_codes`（批量，列表用——避免
//! 虚拟列表每行一次 IPC）。

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::Serialize;
use tauri_plugin_dialog::DialogExt;

use musicforge_core::stylecode::StyleCode;

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

/// 加载 codebook（可选）。传了但读不到 / 格式非法 → 报错（显式失败，
/// 让界面提示路径有问题，而不是静默退回原始码让人以为"没有译名"）。
fn load_codebook(codebook_path: Option<&str>) -> Result<BTreeMap<String, String>, String> {
    match codebook_path {
        Some(p) if !p.trim().is_empty() => {
            musicforge_core::stylecode::load_genre_map(Path::new(p))
        }
        _ => Ok(BTreeMap::new()),
    }
}

/// 结构化数据 → DTO；`labels` 只含**本风格码实际出现且查到**的码
/// （不把整个 codebook 塞给前端）。
fn to_dto(sc: StyleCode, map: &BTreeMap<String, String>) -> StyleCodeDto {
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
    StyleCodeDto {
        year: sc.year,
        style: sc.style,
        mood: sc.mood,
        scenes: sc.scenes,
        version: sc.version,
        other: sc.other,
        labels,
    }
}

/// 解析路径中的风格码；**无前导 `[...]` 码块 → `Ok(None)`**（不是错误：多数曲目无码）。
#[tauri::command]
pub fn style_code(
    path: String,
    codebook_path: Option<String>,
) -> Result<Option<StyleCodeDto>, String> {
    let map = load_codebook(codebook_path.as_deref())?;
    let Some(sc) = musicforge_core::stylecode::parse_style_code(Path::new(&path)) else {
        return Ok(None);
    };
    Ok(Some(to_dto(sc, &map)))
}

// ------------------------------------------------------- genre 写回（X15）--

/// 规划项：单文件的 genre 写入决策。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenrePlanItemDto {
    pub path: String,
    /// `will-write` / `has-genre` / `no-code` / `no-label`（与 CLI `--json` 同形）
    pub status: String,
    /// 仅 `will-write` 有值：将写入的 genre 串
    pub genre: Option<String>,
}

/// genre 写入规划（只读，不落盘）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenrePlanDto {
    pub items: Vec<GenrePlanItemDto>,
    pub will: usize,
    pub has_genre: usize,
    pub no_code: usize,
    pub no_label: usize,
}

/// genre 写入结果。写失败**显式计数**——绝不谎报成功。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenreApplyDto {
    pub written: usize,
    pub failed: usize,
}

fn to_plan_dto(plan: &musicforge_core::stylecode::GenrePlan) -> GenrePlanDto {
    let (will, has_genre, no_code, no_label) = plan.counts();
    let items = plan
        .items
        .iter()
        .map(|(p, d)| match d {
            musicforge_core::stylecode::GenreDecision::WillWrite { genre } => GenrePlanItemDto {
                path: p.display().to_string(),
                status: "will-write".into(),
                genre: Some(genre.clone()),
            },
            musicforge_core::stylecode::GenreDecision::HasGenre => GenrePlanItemDto {
                path: p.display().to_string(),
                status: "has-genre".into(),
                genre: None,
            },
            musicforge_core::stylecode::GenreDecision::NoCode => GenrePlanItemDto {
                path: p.display().to_string(),
                status: "no-code".into(),
                genre: None,
            },
            musicforge_core::stylecode::GenreDecision::NoLabel => GenrePlanItemDto {
                path: p.display().to_string(),
                status: "no-label".into(),
                genre: None,
            },
        })
        .collect();
    GenrePlanDto {
        items,
        will,
        has_genre,
        no_code,
        no_label,
    }
}

/// 规划 genre 写入（**只读**：扫描 → 解析文件名风格码 → 判定，不落盘）。
///
/// `replace_all = false`（FillMissingOnly，与 CLI 默认档一致）时已有非空 genre 的文件
/// 跳过——**绝不覆盖用户已有数据**。
#[tauri::command]
pub async fn genre_plan(
    dir: String,
    codebook_path: Option<String>,
    replace_all: bool,
) -> Result<GenrePlanDto, String> {
    let map = load_codebook(codebook_path.as_deref())?;
    // 全库扫描属重 IO：放阻塞池，避免卡住 UI 线程（同 cue_split）
    let plan = tauri::async_runtime::spawn_blocking(move || {
        musicforge_core::stylecode::plan_genre_writes(Path::new(&dir), &map, replace_all)
    })
    .await
    .map_err(|e| format!("genre plan task failed: {e}"))?
    .map_err(|e| e.to_string())?;
    Ok(to_plan_dto(&plan))
}

/// 执行 genre 写入：先规划（同一 `dir`/`codebook`/`replace_all`，与 CLI 同序）再落盘。
///
/// 只写 `will-write` 项；失败计数继续，返回 `(written, failed)`。
#[tauri::command]
pub async fn genre_apply(
    dir: String,
    codebook_path: Option<String>,
    replace_all: bool,
) -> Result<GenreApplyDto, String> {
    let map = load_codebook(codebook_path.as_deref())?;
    let plan = tauri::async_runtime::spawn_blocking(move || {
        musicforge_core::stylecode::plan_genre_writes(Path::new(&dir), &map, replace_all)
    })
    .await
    .map_err(|e| format!("genre plan task failed: {e}"))?
    .map_err(|e| e.to_string())?;
    let (written, failed) = tauri::async_runtime::spawn_blocking(move || {
        musicforge_core::stylecode::apply_genre_writes(&plan)
    })
    .await
    .map_err(|e| format!("genre apply task failed: {e}"))?;
    Ok(GenreApplyDto { written, failed })
}

/// 读取该音频文件**当前**的 genre 标签（无标签 / 读不了 → `Ok(None)`）。
///
/// 用途：卡片里把「文件名解析出的风格码」与「文件里现有的 genre」并排显示——
/// 写回前先看现状，避免盲改。
#[tauri::command]
pub async fn track_genre(path: String) -> Result<Option<String>, String> {
    // 读标签是文件 IO：放阻塞池，避免卡 UI 线程（同 genre_plan / cue_split）
    tauri::async_runtime::spawn_blocking(move || {
        musicforge_core::stylecode::read_genre_tag(Path::new(&path))
    })
    .await
    .map_err(|e| format!("genre read task failed: {e}"))
}

/// 库中已有的风格码及其曲目数（筛选下拉的数据源；按类别→码稳定排序）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleCodeItemDto {
    /// 码 token（如 `S01` / `Y23`）
    pub code: String,
    /// 含该码的曲目数
    pub count: i64,
}

/// 列出库中全部风格码（供曲库列表的筛选下拉——用户不必先知道有哪些码）。
#[tauri::command]
pub fn style_code_list() -> Result<Vec<StyleCodeItemDto>, String> {
    let db = crate::commands::library_db::open_db()?;
    let rows = db.style_code_counts().map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|(code, count)| StyleCodeItemDto { code, count })
        .collect())
}

/// 原生选择 codebook JSON 文件；用户取消 → `Ok(None)`。
///
/// `title` 由**前端按 UI 语言**传入（I18N-7：服务端/命令层不产出中文显示文案）——
/// 既存的 `cue_pick` 把中文标题写死在 Rust 里，那在英文界面下就是中文弹窗。
#[tauri::command]
pub async fn style_codebook_pick(app: tauri::AppHandle, title: String) -> Option<String> {
    app.dialog()
        .file()
        .add_filter("JSON", &["json"])
        .set_title(title)
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
}

/// **批量**解析（列表用）：返回 `路径 → 风格码`，**只含有码块的路径**
/// （查不到即无码，前端按缺失处理即可，无需为每个无码曲目回传 null）。
///
/// 目的是把虚拟列表的 N 次 IPC 压成 1 次——列表里多数曲目无码，回传集很小。
#[tauri::command]
pub fn style_codes(
    paths: Vec<String>,
    codebook_path: Option<String>,
) -> Result<HashMap<String, StyleCodeDto>, String> {
    let map = load_codebook(codebook_path.as_deref())?;
    let mut out = HashMap::new();
    for p in paths {
        if let Some(sc) = musicforge_core::stylecode::parse_style_code(Path::new(&p)) {
            out.insert(p.clone(), to_dto(sc, &map));
        }
    }
    Ok(out)
}
