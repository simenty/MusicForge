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
    let mut lower: u64 = 0;
    let mut upper: Option<u64> = None;
    for part in range.split(',') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix(">=") {
            lower = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = part.strip_prefix('<') {
            upper = v.trim().parse().ok();
        }
    }
    plugin_major >= lower && upper.map(|u| plugin_major < u).unwrap_or(true)
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
