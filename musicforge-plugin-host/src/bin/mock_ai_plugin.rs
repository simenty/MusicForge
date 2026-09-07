//! mock-ai-plugin（P6a 夹具）：冻结方法集全量实现的最小 NDJSON AI 插件。
//!
//! 行为（方法集冻结于 `docs/p6a-ai-interface.md` §2）：
//! - `plugin.manifest` → 自我声明（api_version 1.0.0 / kind=ai / network=false）；
//!   环境变量 `MOCK_API_VERSION` 可覆盖（D20 不兼容路径的测试钩子）；
//! - `plugin.health` → `{"status":"ok"}`；
//! - `plugin.shutdown` → `{"accepted":true}` 后进程退出；
//! - `ai.identify_track` → 固定 Suggestion（confidence 0.93 + field_confidence）；
//! - `ai.generate_filename_regex` → 由样例确定性生成规则文本（执行权在 core）；
//! - `ai.review_duplicate_group` → 确定性保留建议（按码率/体积取最优成员）；
//! - `lyrics.verify` → 固定核验结论（红线：绝不产出歌手/歌名替换）；
//! - `cover.search` / `cover.generate` → mock 候选（D22：来源优先级最末）；
//! - `test.sleep` → 按 `params.ms` 睡眠后 ok（超时 kill 契约测试用）；
//! - 其他方法 → `MF-PLUGIN-METHOD-UNKNOWN`。
//!
//! 逐行 NDJSON：一行请求 → 一行响应。stderr 不写协议内容。

use std::io::{BufRead, Write};

use musicforge_plugin_api::{
    methods, CoverCandidate, CoverResult, DuplicateGroupParams, DuplicateReviewResult,
    FilenameRegexParams, FilenameRegexResult, HealthResult, IdentifySuggestion, LyricsVerdict,
    LyricsVerifyResult, PluginError, PluginKind, PluginManifest, Request, Response, ShutdownResult,
};

fn main() {
    let api_version = std::env::var("MOCK_API_VERSION").unwrap_or_else(|_| "1.0.0".into());
    let started = std::time::Instant::now();
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
            methods::PLUGIN_MANIFEST => {
                let manifest = PluginManifest {
                    name: "mock-ai".into(),
                    api_version: api_version.clone(),
                    kind: PluginKind::Ai,
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
                    ack_required: false,
                    extensions: vec![],
                };
                Response::ok(&req.id, serde_json::to_value(&manifest).unwrap())
            }
            methods::PLUGIN_HEALTH => {
                let health = HealthResult {
                    status: "ok".into(),
                    uptime_ms: Some(started.elapsed().as_millis() as u64),
                };
                Response::ok(&req.id, serde_json::to_value(&health).unwrap())
            }
            methods::PLUGIN_SHUTDOWN => {
                let accepted = ShutdownResult { accepted: true };
                let resp = Response::ok(&req.id, serde_json::to_value(&accepted).unwrap());
                let _ = writeln!(out, "{}", serde_json::to_string(&resp).unwrap());
                let _ = out.flush();
                // 已应答即退出——host 侧子进程探测应在短窗内观察到退出
                std::process::exit(0);
            }
            methods::AI_IDENTIFY_TRACK => {
                let mut field_confidence = std::collections::BTreeMap::new();
                field_confidence.insert("title".to_string(), 0.99);
                field_confidence.insert("artists".to_string(), 0.95);
                field_confidence.insert("album".to_string(), 0.86);
                let suggestion = IdentifySuggestion {
                    title: "借墨".into(),
                    artists: vec!["王铮亮".into(), "风华音纪".into()],
                    album: Some("借墨".into()),
                    confidence: 0.93,
                    field_confidence,
                    reason: "文件名与内嵌标签一致，feat. 结构识别为合作艺人".into(),
                };
                Response::ok(&req.id, serde_json::to_value(&suggestion).unwrap())
            }
            methods::AI_GENERATE_FILENAME_REGEX => {
                let params: FilenameRegexParams =
                    serde_json::from_value(req.params).unwrap_or_default();
                let rule = if params.samples.is_empty() {
                    "^(?P<artist>.+) - (?P<title>.+)$".to_string()
                } else {
                    // 确定性规则：统一 `艺人 - 标题 [修饰].扩展` 形状
                    "^(?P<artist>.+) - (?P<title>.+?)\\s*\\[[^\\]]+\\]\\.[^.]+$".to_string()
                };
                let result = FilenameRegexResult {
                    rule,
                    confidence: 0.9,
                    reason: format!("基于 {} 个样例归纳", params.samples.len()),
                };
                Response::ok(&req.id, serde_json::to_value(&result).unwrap())
            }
            methods::AI_REVIEW_DUPLICATE_GROUP => {
                let params: DuplicateGroupParams =
                    serde_json::from_value(req.params).unwrap_or_default();
                // 确定性语义判断：码率优先，其次体积（与 core 质量画像互为补充，D24）
                let keep = params
                    .members
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, m)| (m.bitrate_kbps.unwrap_or(0), m.size_bytes.unwrap_or(0)))
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                let result = DuplicateReviewResult {
                    keep_index: keep,
                    reason: "高码率成员优先保留（语义判断，D24）".into(),
                };
                Response::ok(&req.id, serde_json::to_value(&result).unwrap())
            }
            methods::LYRICS_VERIFY => {
                let result = LyricsVerifyResult {
                    verdict: LyricsVerdict::Match,
                    confidence: 0.97,
                    candidates: vec!["mock-lrc-cache".into()],
                };
                Response::ok(&req.id, serde_json::to_value(&result).unwrap())
            }
            methods::COVER_SEARCH => {
                let result = CoverResult {
                    candidates: vec![CoverCandidate {
                        source: "mock".into(),
                        image_ref: "mock://cover/search-1".into(),
                        width_px: Some(1000),
                        height_px: Some(1000),
                        confidence: 0.88,
                    }],
                };
                Response::ok(&req.id, serde_json::to_value(&result).unwrap())
            }
            methods::COVER_GENERATE => {
                let result = CoverResult {
                    candidates: vec![CoverCandidate {
                        source: "mock-generate".into(),
                        image_ref: "mock://cover/generated-1".into(),
                        width_px: Some(1024),
                        height_px: Some(1024),
                        confidence: 0.75,
                    }],
                };
                Response::ok(&req.id, serde_json::to_value(&result).unwrap())
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
