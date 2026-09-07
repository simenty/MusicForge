//! mock-ai-plugin（P6a 预研夹具）：最小 NDJSON AI 插件。
//!
//! 行为：
//! - `plugin.manifest` → 自我声明（api_version 1.0.0 / kind=ai / network=false）；
//!   环境变量 `MOCK_API_VERSION` 可覆盖（D20 不兼容路径的测试钩子）；
//! - `ai.identify_track` → 固定 Suggestion（confidence 0.93 + field_confidence）；
//! - `test.sleep` → 按 `params.ms` 睡眠后 ok（超时 kill 契约测试用）；
//! - 其他方法 → `MF-PLUGIN-METHOD-UNKNOWN`。
//!
//! 逐行 NDJSON：一行请求 → 一行响应。stderr 不写协议内容。

use std::io::{BufRead, Write};

use musicforge_plugin_api::{PluginError, PluginManifest, Request, Response};

fn main() {
    let api_version = std::env::var("MOCK_API_VERSION").unwrap_or_else(|_| "1.0.0".into());
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(req) = serde_json::from_str::<Request>(&line) else {
            let resp = Response::err("unknown", "MF-PLUGIN-MANIFEST-INVALID", "请求行无法解析");
            let _ = writeln!(out, "{}", serde_json::to_string(&resp).unwrap());
            let _ = out.flush();
            continue;
        };
        let resp = match req.method.as_str() {
            "plugin.manifest" => {
                let manifest = PluginManifest {
                    name: "mock-ai".into(),
                    api_version: api_version.clone(),
                    kind: musicforge_plugin_api::PluginKind::Ai,
                    network: false,
                    data_sent: vec![
                        "normalized_filename".into(),
                        "title".into(),
                        "artists".into(),
                        "album".into(),
                        "duration_ms".into(),
                        "format".into(),
                        "language_hint".into(),
                    ],
                    data_not_sent: vec![
                        "absolute_path".into(),
                        "audio_bytes".into(),
                        "cover_bytes".into(),
                    ],
                };
                Response::ok(&req.id, serde_json::to_value(&manifest).unwrap())
            }
            "ai.identify_track" => {
                let mut field_confidence = std::collections::BTreeMap::new();
                field_confidence.insert("title".to_string(), 0.99);
                field_confidence.insert("artists".to_string(), 0.95);
                field_confidence.insert("album".to_string(), 0.86);
                let suggestion = musicforge_plugin_api::IdentifySuggestion {
                    title: "借墨".into(),
                    artists: vec!["王铮亮".into(), "风华音纪".into()],
                    album: Some("借墨".into()),
                    confidence: 0.93,
                    field_confidence,
                    reason: "文件名与内嵌标签一致，feat. 结构识别为合作艺人".into(),
                };
                Response::ok(&req.id, serde_json::to_value(&suggestion).unwrap())
            }
            "test.sleep" => {
                let ms = req.params.get("ms").and_then(|v| v.as_u64()).unwrap_or(0);
                std::thread::sleep(std::time::Duration::from_millis(ms));
                Response::ok(&req.id, serde_json::json!({"sleptMs": ms}))
            }
            other => Response::err(
                &req.id,
                "MF-PLUGIN-METHOD-UNKNOWN",
                format!("未知方法: {other}"),
            ),
        };
        let _ = writeln!(out, "{}", serde_json::to_string(&resp).unwrap());
        let _ = out.flush();
    }
}

// 抑制未使用告警（PluginError 在错误路径经 Response::err 的字符串构造使用）
#[allow(dead_code)]
fn _type_witness(_: PluginError) {}
