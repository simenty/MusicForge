//! mock-ai-plugin（P6a 夹具）：协议 v0.1 全方法实现 + 对抗行为开关。
//!
//! 正常行为（方法集：`docs/plugin-protocol.md` §5）：
//! - `plugin.init` → `InitResult{api_version, manifest}`（P6a-R 握手）；
//! - `plugin.manifest` → 自我声明（`MOCK_API_VERSION` 可覆盖，D20 测试钩子）；
//! - `plugin.health` / `plugin.shutdown`（应答后退出）；
//! - `ai.identify_track` / `ai.generate_filename_regex` / `ai.review_duplicate_group`
//!   / `lyrics.verify`（红线：绝不产出歌手/歌名替换）/ `cover.search` / `cover.generate`；
//! - `format.migrate`（demo）：写 work_dir 产物 + 发 event.progress + artifacts 出站；
//! - `test.sleep` / `test.progress` / `test.emit-event`（协议测试钩子）；
//! - 其他方法 → `MF-PLUGIN-METHOD-UNKNOWN`。
//!
//! 对抗开关（`MOCK_BEHAVIOR`，e2e 对抗套件用）：
//! - `legacy`：init 回 METHOD-UNKNOWN（Host 降级基础信封）；
//! - `no-response`：读请求但不响应（init 10s 超时）；
//! - `bad-init-response`：init 返回畸形 result（Handshake 失败 → 崩溃计数）；
//! - `id-mismatch`：响应 id 错位；
//! - `spurious-event`：每次响应后（无进行中请求时）发 event → Host 违规计数；
//! - `stderr-log`：启动时向 stderr 打印含密钥形态的行（P1-2 脱敏断言）；
//! - `escape-artifacts`：migrate artifacts 返回 `../evil.txt`（X41 逃逸拒绝）。

use std::io::{BufRead, Write};
use std::path::PathBuf;

use musicforge_plugin_api::{
    events, methods, v1, CoverCandidate, CoverResult, DuplicateGroupParams, DuplicateReviewResult,
    FilenameRegexParams, FilenameRegexResult, FormatMigrateParams, FormatMigrateResult,
    HealthResult, IdentifySuggestion, InitParams, InitResult, LyricsVerdict, LyricsVerifyResult,
    MigrateVerification, PluginError, PluginKind, PluginManifest, Request, Response,
    ShutdownResult,
};

fn manifest_for(api_version: &str) -> PluginManifest {
    PluginManifest {
        name: "mock-ai".into(),
        api_version: api_version.to_string(),
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
            "header_hex".into(),
            "tail_hex".into(),
        ],
        data_not_sent: vec![
            "absolute_path".into(),
            "audio_bytes".into(),
            "cover_bytes".into(),
        ],
        ack_required: false,
        extensions: vec![],
        permissions: Default::default(),
    }
}

/// 对抗/演示：发一条无 id 事件（progress）。
fn emit_progress(out: &mut impl Write, job_id: &str, percent: u8, stage: &str) {
    let line = serde_json::json!({
        "method": events::PROGRESS,
        "params": {"job_id": job_id, "percent": percent, "stage": stage}
    });
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

fn main() {
    let api_version = std::env::var("MOCK_API_VERSION").unwrap_or_else(|_| "1.0.0".into());
    let behavior = std::env::var("MOCK_BEHAVIOR").unwrap_or_default();
    let started = std::time::Instant::now();
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();

    if behavior == "stderr-log" {
        eprintln!("apikey=SECRET12345 token=ABCDEF client_password=hunter2");
    }

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if behavior == "no-response" {
            continue; // 读请求永不响应（超时路径）
        }
        let Ok(req) = serde_json::from_str::<Request>(&line) else {
            let resp = Response::err("unknown", "MF-PLUGIN-MANIFEST-INVALID", "请求行无法解析");
            let _ = writeln!(out, "{}", serde_json::to_string(&resp).unwrap());
            let _ = out.flush();
            continue;
        };
        let resp = match req.method.as_str() {
            methods::PLUGIN_INIT => {
                if behavior == "legacy" {
                    // 对抗：旧插件不认识 init → Host 降级基础信封
                    Response::err(
                        &req.id,
                        "MF-PLUGIN-METHOD-UNKNOWN",
                        "legacy mock: no plugin.init",
                    )
                } else if behavior == "bad-init-response" {
                    // 对抗：init 成功但 result 畸形 → Host Handshake 失败（崩溃计数）
                    Response::ok(&req.id, serde_json::json!({"answer": 42}))
                } else {
                    let init: InitParams =
                        serde_json::from_value(req.params.clone()).unwrap_or(InitParams {
                            protocol_version: 1,
                            work_dir: String::new(),
                            locale: String::new(),
                            host_capabilities: Default::default(),
                        });
                    // P6a-R：声明 v1 能力（events/artifacts 依赖 work_dir 已授权）
                    let _ = v1::PROTOCOL_VERSION;
                    let result = InitResult {
                        api_version: api_version.clone(),
                        manifest: Some(manifest_for(&api_version)),
                    };
                    let _ = init;
                    Response::ok(&req.id, serde_json::to_value(&result).unwrap())
                }
            }
            methods::PLUGIN_MANIFEST => Response::ok(
                &req.id,
                serde_json::to_value(manifest_for(&api_version)).unwrap(),
            ),
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
            // ---- P6a-R demo：format.migrate（写 work_dir + progress + artifacts）----
            methods::FORMAT_MIGRATE => {
                let params: FormatMigrateParams =
                    serde_json::from_value(req.params).unwrap_or(FormatMigrateParams {
                        job_id: "job-0".into(),
                        input_path: String::new(),
                        output_path: String::new(),
                        work_dir: String::new(),
                        options: Default::default(),
                    });
                let wd = PathBuf::from(&params.work_dir);
                let artifact_rel = "out.flac";
                let _ = std::fs::create_dir_all(&wd);
                let _ = std::fs::write(wd.join(artifact_rel), b"MOCK-FLAC-BYTES");
                emit_progress(&mut out, params.job_id.as_str(), 50, "decrypt");
                let artifacts: Vec<String> = if behavior == "escape-artifacts" {
                    vec!["../evil.txt".into()] // X41 逃逸（Host 必须拒绝）
                } else {
                    vec![artifact_rel.to_string()]
                };
                let result = FormatMigrateResult {
                    status: "success".into(),
                    output_format: "flac".into(),
                    artifacts: artifacts.clone(),
                    output_path: wd.join(artifact_rel).display().to_string(),
                    verification: MigrateVerification {
                        magic: "fLaC".into(),
                        sample_rate: Some(44100),
                        channels: Some(2),
                        duration_s: Some(1.0),
                    },
                    audit: musicforge_plugin_api::MigrateAudit {
                        source_sha256: "a".repeat(64),
                        output_sha256: "b".repeat(64),
                        quarantined: false,
                    },
                    warnings: vec![],
                };
                Response::ok(&req.id, serde_json::to_value(&result).unwrap())
            }
            // ---- 协议测试钩子 ----
            "test.sleep" => {
                let ms = req.params.get("ms").and_then(|v| v.as_u64()).unwrap_or(0);
                std::thread::sleep(std::time::Duration::from_millis(ms));
                Response::ok(&req.id, serde_json::json!({"sleptMs": ms}))
            }
            "test.progress" => {
                // 请求进行中发事件（合法路径）→ Host drain_events 应收到
                emit_progress(&mut out, "job-test", 42, "demo");
                Response::ok(&req.id, serde_json::json!({"emitted": true}))
            }
            other => {
                let resp = Response::err(
                    &req.id,
                    "MF-PLUGIN-METHOD-UNKNOWN",
                    format!("未知方法: {other}"),
                );
                resp
            }
        };
        let resp = if behavior == "id-mismatch" {
            let mut r = resp;
            r.id = "wrong-id".into();
            r
        } else {
            resp
        };
        let _ = writeln!(out, "{}", serde_json::to_string(&resp).unwrap());
        let _ = out.flush();
        // 对抗：响应**之后**（无进行中请求）乱发事件 → Host 违规计数 +1。
        // 先睡 200ms 确保 Host 已 poll 到响应并清除 in-flight（消除事件/响应竞态）。
        if behavior == "spurious-event" {
            std::thread::sleep(std::time::Duration::from_millis(200));
            emit_progress(&mut out, "job-spurious", 1, "violation");
        }
    }
}

// 抑制未使用告警（PluginError 在错误路径经 Response::err 的字符串构造使用）
#[allow(dead_code)]
fn _type_witness(_: PluginError) {}
