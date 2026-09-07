//! MusicForge 插件协议 crate（P6a 预研，D18/D20/X8）。
//!
//! 设计冻结来源：`docs/p6a-ai-interface.md` + 方案 v2.7.2。
//!
//! - **协议正典（X8）**：stdio 上的强类型 NDJSON——
//!   请求 `{"id","method","params"}`，响应 `{"id","ok","result"|"error":{code,message}}`；
//! - **D20**：`api_version` semver 区间协商（本预研实现 `>=M,<N` 与精确 `M` 子集）；
//! - **最小请求模型**：类型层面不存在 `absolute_path`/`audio_bytes`/`cover_bytes`
//!   字段——违反即编译不过，而非运行时审查；
//! - **三条红线**（P6a 硬验收）：AI 只建议不执行 / 最小请求 / 降级完整性。
//!
//! 本 crate **零网络、零进程管理**——那是 [`musicforge-plugin-host`] 的职责
//! （D12：能力分层，跨层下沉 = 架构缺陷）。

use serde::{Deserialize, Serialize};

/// 稳定错误码（插件域，P6a 起生效；详见 docs/result-codes.md）。
pub mod codes {
    pub const API_INCOMPATIBLE: &str = "MF-PLUGIN-API-INCOMPATIBLE";
    pub const TIMEOUT: &str = "MF-PLUGIN-TIMEOUT";
    pub const FAILED: &str = "MF-PLUGIN-FAILED";
    pub const METHOD_UNKNOWN: &str = "MF-PLUGIN-METHOD-UNKNOWN";
    pub const MANIFEST_INVALID: &str = "MF-PLUGIN-MANIFEST-INVALID";
    /// Host 侧支持的最大协议主版本（D20 区间的上界来源）
    pub const HOST_API_MAJOR: u64 = 1;
}

/// 冻结方法集（`docs/p6a-ai-interface.md` §2；P6a 施工清单第 2 项）。
///
/// 命名空间约定：`plugin.*` = 生命周期管理；`ai.*` / `lyrics.*` / `cover.*` = 能力域。
pub mod methods {
    pub const PLUGIN_MANIFEST: &str = "plugin.manifest";
    pub const PLUGIN_HEALTH: &str = "plugin.health";
    pub const PLUGIN_SHUTDOWN: &str = "plugin.shutdown";
    pub const AI_IDENTIFY_TRACK: &str = "ai.identify_track";
    /// 规则文本交回 core 执行（AI 只建议，执行权在 core）
    pub const AI_GENERATE_FILENAME_REGEX: &str = "ai.generate_filename_regex";
    /// D24：重复组保留建议（质量画像之外的语义判断）
    pub const AI_REVIEW_DUPLICATE_GROUP: &str = "ai.review_duplicate_group";
    /// 红线：绝不改歌手/歌名
    pub const LYRICS_VERIFY: &str = "lyrics.verify";
    /// D22：封面来源优先级最末
    pub const COVER_SEARCH: &str = "cover.search";
    pub const COVER_GENERATE: &str = "cover.generate";
}

// ---------------------------------------------------------------- 信封 --

/// 请求信封（X8 强类型 NDJSON）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Request {
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// 错误体。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginError {
    pub code: String,
    pub message: String,
}

impl PluginError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }
}

/// 响应信封：`ok=true` 携带 `result`；`ok=false` 携带 `error`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Response {
    pub id: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<PluginError>,
}

impl Response {
    pub fn ok(id: &str, result: serde_json::Value) -> Self {
        Self {
            id: id.to_string(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: &str, code: &str, message: impl Into<String>) -> Self {
        Self {
            id: id.to_string(),
            ok: false,
            result: None,
            error: Some(PluginError::new(code, message)),
        }
    }
}

// ---------------------------------------------------------------- 清单 --

/// 插件种类（P6a 首发只有 ai；format-adapter 归 P6b）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PluginKind {
    Ai,
    FormatAdapter,
}

/// plugin.json 权限清单（v2 契约）。
///
/// 准入规则（方案 §4.10）：`network == false` 对 format-adapter 强制；
/// `data_not_sent` 必须显式列出（最小请求模型的对偶声明）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginManifest {
    pub name: String,
    /// 插件实现的协议版本，如 `"1.0.0"`（D20 区间协商的插件侧输入）
    pub api_version: String,
    pub kind: PluginKind,
    pub network: bool,
    #[serde(default)]
    pub data_sent: Vec<String>,
    #[serde(default)]
    pub data_not_sent: Vec<String>,
}

// ---------------------------------------------------------------- AI 域类型 --

/// 逐字段置信度（v2.7.2 细化：阈值判定可精确到字段级）。
pub type FieldConfidence = std::collections::BTreeMap<String, f32>;

/// `ai.identify_track` 的建议产物（X13：只建议，执行权在 core 的 Plan 层）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IdentifySuggestion {
    pub title: String,
    #[serde(default)]
    pub artists: Vec<String>,
    #[serde(default)]
    pub album: Option<String>,
    pub confidence: f32,
    #[serde(default)]
    pub field_confidence: FieldConfidence,
    pub reason: String,
}

// ---------------------------------------------------------------- 方法参数/结果 --

/// `ai.identify_track` 请求参数（协议形状冻结于 p6a-ai-interface.md §3）。
///
/// 最小请求模型：类型层面不存在 `absolute_path`/`audio_bytes`/`cover_bytes`。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct IdentifyTrackParams {
    pub normalized_filename: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub artists: Vec<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub language_hint: Option<String>,
}

/// `ai.generate_filename_regex` 请求参数：文件名样例集。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FilenameRegexParams {
    pub samples: Vec<String>,
}

/// `ai.generate_filename_regex` 结果：规则文本（执行权在 core，插件不碰文件系统）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FilenameRegexResult {
    pub rule: String,
    pub confidence: f32,
    pub reason: String,
}

/// 重复组成员画像（D24 语义判断的最小上下文；不含路径，仅元数据）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DuplicateMember {
    /// 文件名（含扩展名；禁发绝对路径）
    pub filename: String,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub bitrate_kbps: Option<u64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub size_bytes: Option<u64>,
}

/// `ai.review_duplicate_group` 请求参数。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DuplicateGroupParams {
    pub members: Vec<DuplicateMember>,
}

/// `ai.review_duplicate_group` 结果：建议保留项（下标）+ 理由。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DuplicateReviewResult {
    pub keep_index: usize,
    pub reason: String,
}

/// `lyrics.verify` 请求参数（红线：绝不改歌手/歌名——本方法只核验，不产出替换标题）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LyricsVerifyParams {
    pub title: String,
    pub artists: Vec<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// 歌词文本抽样（前 N 行），供语义比对
    pub lyrics_excerpt: String,
}

/// 歌词核验结论。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LyricsVerdict {
    Match,
    Mismatch,
    Uncertain,
}

/// `lyrics.verify` 结果：核验结论 + 候选（候选仅为歌词来源描述，不含标题/艺人替换）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LyricsVerifyResult {
    pub verdict: LyricsVerdict,
    pub confidence: f32,
    #[serde(default)]
    pub candidates: Vec<String>,
}

/// 封面候选（`cover.search` / `cover.generate` 共用）。
///
/// `image_ref` 为远端引用或本地缓存描述——禁发 `cover_bytes` 本体。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CoverCandidate {
    pub source: String,
    pub image_ref: String,
    #[serde(default)]
    pub width_px: Option<u32>,
    #[serde(default)]
    pub height_px: Option<u32>,
    pub confidence: f32,
}

/// 封面域查询参数（D22：来源优先级最末，仅在无达标内嵌时由 core 发起）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CoverQueryParams {
    pub title: String,
    #[serde(default)]
    pub artists: Vec<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub style_hint: Option<String>,
}

/// `cover.search` / `cover.generate` 结果。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CoverResult {
    pub candidates: Vec<CoverCandidate>,
}

/// `plugin.health` 结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthResult {
    pub status: String,
    #[serde(default)]
    pub uptime_ms: Option<u64>,
}

/// `plugin.shutdown` 结果（插件应答后自行退出）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShutdownResult {
    pub accepted: bool,
}

// ---------------------------------------------------------------- D20 协商 --

/// D20：插件 `api_version` 是否落在 Host 支持区间内。
///
/// 本预研支持的区间语法子集：`">=M,<N"`（标准形态）与精确 `"M"`。
/// 语义：插件主版本 M 满足 `lower <= M < upper`（缺上界视为无上界）。
/// 完整 semver 表达式在 P6a 正式期评估 `semver` crate（MIT，需入白名单）。
pub fn api_compatible(plugin_api_version: &str, host_range: &str) -> bool {
    let plugin_major: u64 = match plugin_api_version.split('.').next().unwrap_or("").parse() {
        Ok(m) => m,
        Err(_) => return false,
    };
    let range = host_range.trim();
    if let Ok(major) = range.parse::<u64>() {
        return plugin_major == major;
    }
    // 稳定审计修复（2026-09-08）：此前约束解析失败（如 "1.0.0"/"garbage"）时
    // lower 默认 0 且无上界 → 隐式放行一切插件。修复为：未解析出任何合法约束
    // 或出现未知形态 → 显式不兼容（拒绝优于放行）。
    let mut lower: Option<u64> = None;
    let mut upper: Option<u64> = None;
    let mut saw_constraint = false;
    for part in range.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue; // 容忍尾部逗号的空白片段
        }
        if let Some(v) = part.strip_prefix(">=") {
            match v.trim().parse::<u64>() {
                Ok(n) => {
                    lower = Some(n);
                    saw_constraint = true;
                }
                Err(_) => return false,
            }
        } else if let Some(v) = part.strip_prefix('<') {
            match v.trim().parse::<u64>() {
                Ok(n) => {
                    upper = Some(n);
                    saw_constraint = true;
                }
                Err(_) => return false,
            }
        } else {
            return false; // 未知约束形态 → 显式不兼容
        }
    }
    if !saw_constraint {
        return false;
    }
    plugin_major >= lower.unwrap_or(0) && upper.map(|u| plugin_major < u).unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_roundtrip() {
        let req = Request {
            id: "req-1".into(),
            method: "ai.identify_track".into(),
            params: serde_json::json!({"normalized_filename": "x.wav"}),
        };
        let line = serde_json::to_string(&req).unwrap();
        let back: Request = serde_json::from_str(&line).unwrap();
        assert_eq!(back, req);

        let resp = Response::err("req-1", codes::METHOD_UNKNOWN, "no such method");
        let line = serde_json::to_string(&resp).unwrap();
        assert!(line.contains(r#""ok":false"#));
        let back: Response = serde_json::from_str(&line).unwrap();
        assert_eq!(back.error.unwrap().code, codes::METHOD_UNKNOWN);
    }

    #[test]
    fn d20_range_negotiation() {
        assert!(api_compatible("1.0.0", ">=1,<2"));
        assert!(api_compatible("1.9.9", ">=1,<2"));
        assert!(!api_compatible("2.0.0", ">=1,<2"));
        assert!(!api_compatible("0.9.0", ">=1,<2"));
        assert!(api_compatible("1.0.0", "1"));
        assert!(!api_compatible("2.0.0", "1"));
        assert!(!api_compatible("garbage", ">=1,<2"));
        // 稳定审计 B4：host_range 本身不可解析 → 必须显式拒绝（此前隐式放行一切）
        assert!(!api_compatible("1.0.0", "garbage"), "垃圾区间不得放行");
        assert!(
            !api_compatible("1.0.0", "1.0.0"),
            "完整 semver 误入区间位 → 拒绝而非放行"
        );
        assert!(!api_compatible("1.0.0", ">=x,<2"), "约束数值非法 → 拒绝");
        assert!(
            api_compatible("1.9.0", ">=1"),
            "仅下界（无上界）仍是合法形态"
        );
        assert!(
            api_compatible("2.0.0", ">=1"),
            "仅下界（无上界）→ 2.x 合法（与原实现语义一致）"
        );
    }

    #[test]
    fn manifest_parses() {
        let m: PluginManifest = serde_json::from_str(
            r#"{"name":"mock-ai","api_version":"1.0.0","kind":"ai","network":false,
                "data_sent":["normalized_filename","title","artists","album","duration_ms","format","language_hint"],
                "data_not_sent":["absolute_path","audio_bytes","cover_bytes"]}"#,
        )
        .unwrap();
        assert_eq!(m.kind, PluginKind::Ai);
        assert!(!m.network);
        assert!(m.data_not_sent.contains(&"absolute_path".to_string()));
    }
}
