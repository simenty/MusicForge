//! `/api` 业务面（P8.2.1 首波：scan / version / wizard-status）。
//!
//! **统一信封**：成功 `{ok:true, data}`；失败 `{ok:false, code:"MF-*", message}`
//! ——业务码 = `NcmError::mf_code()`（跨端一致，UI/日志/失败清单同码）。
//!
//! **零写操作**：本波只读（scan 本身只读；wizard 只探测）——破坏类操作
//! （convert/organize/trash）随 P8.2 后续迭代接线并强制 `safety` 分级。

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::Json as JsonBody;
use serde_json::{json, Value};
use std::path::PathBuf;

use musicforge_core::scan::{scan_library, ScanOptions};

use super::ServerState;

/// 成功信封。
pub fn ok(data: Value) -> Response {
    (StatusCode::OK, Json(json!({ "ok": true, "data": data }))).into_response()
}

/// 失败信封（稳定码 + 人类文案）。
pub fn err(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(json!({ "ok": false, "code": code, "message": message.into() })),
    )
        .into_response()
}

/// `NcmError` → 4xx 信封（mf_code 透传，跨端同码）。
fn err_from(e: musicforge_core::NcmError) -> Response {
    err(StatusCode::BAD_REQUEST, e.mf_code(), format!("{e}"))
}

/// `Category` → 稳定字符串（JSON 形态；不暴露枚举内部）。
fn category_str(c: musicforge_core::scan::Category) -> &'static str {
    match c {
        musicforge_core::scan::Category::Audio => "audio",
        musicforge_core::scan::Category::Lyrics => "lyrics",
        musicforge_core::scan::Category::Cover => "cover",
        musicforge_core::scan::Category::Junk => "junk",
        musicforge_core::scan::Category::Other => "other",
    }
}

/// `ScanReport` → JSON（server 侧手动映射——core 不引 serde derive，依赖面最小）。
fn scan_report_json(r: musicforge_core::scan::ScanReport) -> Value {
    json!({
        "counts": {
            "audio": r.audio,
            "lyrics": r.lyrics,
            "covers": r.covers,
            "junk": r.junk,
            "other": r.other,
            "scanned_files": r.scanned_files,
            "scanned_dirs": r.scanned_dirs,
        },
        "items": r.items.iter().map(|i| json!({
            "path": i.path.display().to_string(),
            "category": category_str(i.category),
            "rule_id": i.rule_id,
            "size": i.size,
            "mtime": i.mtime,
        })).collect::<Vec<_>>(),
        "empty_dirs": r.empty_dirs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        "rule_hits": r.rule_hits,
        // P8：未授权目录（fnOS 授权模型）——UI 聚合呈现 MF-DIR-NOT-AUTHORIZED
        "unauthorized_dirs": r.unauthorized_dirs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
    })
}

/// `POST /api/scan`：库扫描（只读）。
///
/// 请求 `{ "dir": "...", "max_depth": 64 }`（dir 缺省 = 服务端配置的音乐库目录
/// `MUSICFORGE_LIBRARY_DIR`；两者皆缺 → `MF-API-BAD-REQUEST`）。
pub async fn scan(State(state): State<ServerState>, body: Option<JsonBody<Value>>) -> Response {
    let dir = body
        .as_ref()
        .and_then(|b| b.0.get("dir"))
        .and_then(|v| v.as_str())
        .map(std::path::PathBuf::from)
        .or_else(|| state.library_dir.clone());
    let Some(dir) = dir else {
        return err(
            StatusCode::BAD_REQUEST,
            "MF-API-BAD-REQUEST",
            "缺 dir 且服务端未配置 MUSICFORGE_LIBRARY_DIR",
        );
    };
    let max_depth = body
        .as_ref()
        .and_then(|b| b.0.get("max_depth"))
        .and_then(|v| v.as_u64())
        .map(|v| v.min(512) as usize)
        .unwrap_or(64);
    let options = ScanOptions {
        max_depth,
        ..ScanOptions::default()
    };
    match scan_library(&dir, &options) {
        Ok(report) => ok(scan_report_json(report)),
        Err(e) => err_from(e),
    }
}

/// `GET /api/version`：版本与形态（生命周期/前端握手用）。
pub async fn version() -> Response {
    ok(json!({
        "name": "musicforge-server",
        "version": env!("CARGO_PKG_VERSION"),
        "api_surface": "p8.2.1",
    }))
}

/// `GET /api/wizard/status`：首启向导状态探测（只读）。
///
/// - `token_ready`：token 已就绪（服务启动即满足）
/// - `data_dir_writable`：数据目录可写（X16 位置铁律的服务端自检）
pub async fn wizard_status(State(state): State<ServerState>) -> Response {
    // 数据目录自动创建（cmd/main 也 mkdir——双保险；X16：本地盘自有空间）
    let _ = std::fs::create_dir_all(&state.data_dir);
    let probe = state.data_dir.join(".wizard-probe");
    let data_dir_writable = std::fs::write(&probe, b"ok").is_ok();
    if data_dir_writable {
        let _ = std::fs::remove_file(&probe);
    }
    ok(json!({
        "token_ready": !state.token.is_empty(),
        "data_dir_writable": data_dir_writable,
        "data_dir": state.data_dir.display().to_string(),
        "library_dir": state.library_dir.as_ref().map(|p| p.display().to_string()),
    }))
}

/// `POST /api/convert`：格式迁移（P8.2.2，经 cli 桥接 + plugin-host）。
///
/// 请求 `{ "plugin", "source", "output_dir"?, "ekey"? }`——与 GUI IPC 同源
/// （musicforge_cli::format_migrate；ACK 闸/崩溃计数/并发槽位全部内建）。
/// output_dir 缺省 = 源父目录。ekey 透传（QMC STag 变体，本地传递零网络）。
pub async fn convert(JsonBody(body): JsonBody<Value>) -> Response {
    let Some(plugin) = body.get("plugin").and_then(|v| v.as_str()) else {
        return err(
            StatusCode::BAD_REQUEST,
            "MF-API-BAD-REQUEST",
            "缺 plugin（如 qmc-migration / kwm-migration）",
        );
    };
    let Some(source) = body.get("source").and_then(|v| v.as_str()) else {
        return err(StatusCode::BAD_REQUEST, "MF-API-BAD-REQUEST", "缺 source");
    };
    let output_dir = body
        .get("output_dir")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let ekey = body.get("ekey").and_then(|v| v.as_str());
    // output_dir 缺省 = 源父目录（与 GUI IPC 同语义）
    let out_dir = if output_dir.trim().is_empty() {
        std::path::Path::new(source)
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    } else {
        output_dir.to_string()
    };
    match musicforge_cli::format_migrate(plugin, source, &out_dir, None, ekey) {
        Ok(output_path) => ok(json!({ "outputPath": output_path })),
        Err(e) => err(StatusCode::BAD_REQUEST, e.mf_code(), format!("{e}")),
    }
}

// ---------------------------------------------------------------- organize --

use musicforge_core::organize::{
    apply_organize_plan, plan_organize, ConflictStrategy, OrganizeOptions, OrganizeStatus,
};

const DEFAULT_TEMPLATE: &str = "{artist} - {title}";

/// organize 请求参数提取（plan/apply 共用）。
struct OrganizeReq {
    dir: PathBuf,
    template: String,
    target_root: PathBuf,
    conflict: ConflictStrategy,
}

#[allow(clippy::result_large_err)] // Response 直返（单调用点，Box 化噪音大于收益）
fn parse_organize_req(body: &Value) -> Result<OrganizeReq, Response> {
    let Some(dir) = body.get("dir").and_then(|v| v.as_str()) else {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "MF-API-BAD-REQUEST",
            "缺 dir（整理根目录）",
        ));
    };
    let template = body
        .get("template")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_TEMPLATE)
        .to_string();
    let target_root = body
        .get("target_root")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(dir)); // 缺省 = 原地整理
    let conflict = body
        .get("strategy")
        .and_then(|v| v.as_str())
        .map(|s| {
            ConflictStrategy::parse(s).ok_or_else(|| {
                err(
                    StatusCode::BAD_REQUEST,
                    "MF-API-BAD-REQUEST",
                    format!("未知 strategy: {s}（skip/suffix/overwrite-never）"),
                )
            })
        })
        .transpose()?
        .unwrap_or(ConflictStrategy::Skip);
    Ok(OrganizeReq {
        dir: PathBuf::from(dir),
        template,
        target_root,
        conflict,
    })
}

fn plan_json(
    items: &[musicforge_core::organize::OrganizeItem],
    tpl: &str,
    strategy: &str,
) -> Value {
    json!({
        "items": items.iter().map(|i| json!({
            "source": i.source.display().to_string(),
            "target": i.target.display().to_string(),
            "status": match i.status {
                OrganizeStatus::Planned => "planned",
                OrganizeStatus::AlreadyInPlace => "in_place",
                OrganizeStatus::SkippedConflict => "skipped_conflict",
                OrganizeStatus::ConflictNever => "conflict_never",
            },
            "note": i.note,
        })).collect::<Vec<_>>(),
        "template": tpl,
        "strategy": strategy,
    })
}

fn build_options(r: &OrganizeReq) -> OrganizeOptions<'_> {
    OrganizeOptions {
        template: &r.template,
        target_root: &r.target_root,
        conflict: r.conflict,
    }
}

/// `POST /api/organize/plan`：整理计划预览（**只读**，绝不移动）。
///
/// 请求 `{ "dir", "template"?, "target_root"?, "strategy"? }`。
pub async fn organize_plan(JsonBody(body): JsonBody<Value>) -> Response {
    let r = match parse_organize_req(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match plan_organize(&r.dir, &build_options(&r)) {
        Ok(plan) => {
            let c = plan.counts();
            ok(json!({
                "counts": {
                    "planned": c.planned,
                    "in_place": c.in_place,
                    "skipped_conflict": c.skipped_conflict,
                    "conflict_never": c.conflict_never,
                },
                "plan": plan_json(&plan.items, &plan.template, plan.strategy.as_str()),
            }))
        }
        Err(e) => err_from(e),
    }
}

/// `POST /api/organize/apply`：执行整理计划（**破坏类**——safety 分级强制）。
///
/// - `confirm !== true` → `403 MF-OP-NEEDS-YES`（P2 安全分级语义的 API 版：
///   必须先 plan 预览、再显式 confirm 才落盘）；
/// - 绝不覆盖（apply 间隙目标被外部创建 → 该项失败）；
/// - 回滚清单落 target_root/.musicforge/。
pub async fn organize_apply(JsonBody(body): JsonBody<Value>) -> Response {
    let confirm = body
        .get("confirm")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !confirm {
        return err(
            StatusCode::FORBIDDEN,
            "MF-OP-NEEDS-YES",
            "破坏类操作需显式确认：先调 /api/organize/plan 预览，再以 confirm:true 执行",
        );
    }
    let r = match parse_organize_req(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    let plan = match plan_organize(&r.dir, &build_options(&r)) {
        Ok(p) => p,
        Err(e) => return err_from(e),
    };
    let task_id = format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        std::process::id()
    );
    match apply_organize_plan(&plan, &task_id) {
        Ok(outcome) => ok(json!({
            "moved": outcome.moved,
            "skipped": outcome.skipped,
            "failed": outcome.failed,
            "rollback_manifest": outcome.rollback_manifest.map(|p| p.display().to_string()),
        })),
        Err(e) => err_from(e),
    }
}

// ---------------------------------------------------------------- clean/trash --

use musicforge_core::scan::{apply_clean_plan, build_clean_plan, restore_from_trash, RULE_CARDS};

/// 规则启用集提取（rules 缺省 = 全部 RULE_CARDS；逗号分隔 ID）。
fn enabled_rules(body: &Value) -> std::collections::HashSet<&'static str> {
    match body.get("rules").and_then(|v| v.as_str()) {
        Some(list) => {
            let wanted: std::collections::HashSet<&str> =
                list.split(',').map(|s| s.trim()).collect();
            RULE_CARDS
                .iter()
                .map(|c| c.id)
                .filter(|id| wanted.contains(*id))
                .collect()
        }
        None => RULE_CARDS.iter().map(|c| c.id).collect(),
    }
}

/// `POST /api/clean/plan`：垃圾清洗计划预览（**只读**——dry-run 语义）。
///
/// 请求 `{ "dir", "rules"? }`（rules 缺省 = 全部 RULE_CARDS）。
pub async fn clean_plan(JsonBody(body): JsonBody<Value>) -> Response {
    let Some(dir) = body.get("dir").and_then(|v| v.as_str()) else {
        return err(StatusCode::BAD_REQUEST, "MF-API-BAD-REQUEST", "缺 dir");
    };
    let dir = PathBuf::from(dir);
    let report = match scan_library(&dir, &ScanOptions::default()) {
        Ok(r) => r,
        Err(e) => return err_from(e),
    };
    let trash_root = dir.join(".musicforge").join("trash");
    let plan = build_clean_plan(&report, &enabled_rules(&body), &trash_root, &dir);
    ok(json!({
        "actions": plan.actions.iter().map(|a| json!({
            "path": a.path.display().to_string(),
            "rule_id": a.rule_id,
        })).collect::<Vec<_>>(),
        "empty_dirs": plan.empty_dirs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        "trash_root": plan.trash_root.display().to_string(),
    }))
}

/// `POST /api/clean/apply`：执行清洗（**破坏类**——全部进回收站可整体还原，
/// 但仍强制 confirm；P2 语义：dry-run 先行）。
///
/// 请求 `{ "dir", "rules"?, "confirm" }`。回收站 = `<dir>/.musicforge/trash/<task>/`。
pub async fn clean_apply(JsonBody(body): JsonBody<Value>) -> Response {
    let confirm = body
        .get("confirm")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !confirm {
        return err(
            StatusCode::FORBIDDEN,
            "MF-OP-NEEDS-YES",
            "清洗为破坏类操作：先调 /api/clean/plan 预览，再以 confirm:true 执行（可整体还原）",
        );
    }
    let Some(dir) = body.get("dir").and_then(|v| v.as_str()) else {
        return err(StatusCode::BAD_REQUEST, "MF-API-BAD-REQUEST", "缺 dir");
    };
    let dir = PathBuf::from(dir);
    let report = match scan_library(&dir, &ScanOptions::default()) {
        Ok(r) => r,
        Err(e) => return err_from(e),
    };
    let trash_root = dir.join(".musicforge").join("trash");
    let plan = build_clean_plan(&report, &enabled_rules(&body), &trash_root, &dir);
    let task_id = format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        std::process::id()
    );
    match apply_clean_plan(&plan, &task_id) {
        Ok(outcome) => ok(json!({
            "moved": outcome.moved,
            "dirs_removed": outcome.dirs_removed,
            "rollback_manifest": outcome.rollback_manifest.map(|p| p.display().to_string()),
        })),
        Err(e) => err_from(e),
    }
}

/// `POST /api/trash/restore`：从回滚清单整体还原（**破坏类**——confirm 强制）。
///
/// 请求 `{ "manifest": "<rollback.jsonl 路径>", "confirm" }`。
pub async fn trash_restore(JsonBody(body): JsonBody<Value>) -> Response {
    let confirm = body
        .get("confirm")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !confirm {
        return err(
            StatusCode::FORBIDDEN,
            "MF-OP-NEEDS-YES",
            "还原会覆盖恢复路径上的同名文件（如存在）：请以 confirm:true 确认",
        );
    }
    let Some(manifest) = body.get("manifest").and_then(|v| v.as_str()) else {
        return err(StatusCode::BAD_REQUEST, "MF-API-BAD-REQUEST", "缺 manifest");
    };
    match restore_from_trash(std::path::Path::new(manifest)) {
        Ok(n) => ok(json!({ "restored": n })),
        Err(e) => err_from(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{build_router, ServerState};
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use std::path::PathBuf;
    use tower::ServiceExt;

    fn state_with(lib: Option<PathBuf>) -> ServerState {
        ServerState {
            token: "tok-test".to_string(),
            ui_dir: PathBuf::from("ui"),
            data_dir: std::env::temp_dir().join(format!("mf-api-test-{}", std::process::id())),
            library_dir: lib,
        }
    }

    fn req(method: &str, uri: &str, body: Option<String>) -> Request<Body> {
        let b = Request::builder()
            .method(method)
            .uri(uri)
            .header("x-token", "tok-test");
        match body {
            Some(s) => b
                .header("content-type", "application/json")
                .body(Body::from(s))
                .unwrap(),
            None => b.body(Body::empty()).unwrap(),
        }
    }

    async fn body_json(res: Response) -> Value {
        let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn version_returns_name_and_version() {
        let app = build_router(state_with(None));
        let res = app.oneshot(req("GET", "/api/version", None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let v = body_json(res).await;
        assert_eq!(v["ok"], true);
        assert_eq!(v["data"]["name"], "musicforge-server");
    }

    #[tokio::test]
    async fn scan_empty_dir_reports_counts_and_no_unauthorized() {
        let dir = std::env::temp_dir().join(format!("mf-api-scan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let app = build_router(state_with(None));
        let res = app
            .clone()
            .oneshot(req(
                "POST",
                "/api/scan",
                Some(json!({"dir": dir.display().to_string()}).to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let v = body_json(res).await;
        assert_eq!(v["ok"], true);
        assert!(v["data"]["counts"]["scanned_dirs"].as_u64().unwrap() >= 1);
        assert_eq!(v["data"]["unauthorized_dirs"], json!([]));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn scan_without_dir_and_library_config_is_bad_request() {
        let app = build_router(state_with(None));
        let res = app
            .clone()
            .oneshot(req("POST", "/api/scan", Some(json!({}).to_string())))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let v = body_json(res).await;
        assert_eq!(v["code"], "MF-API-BAD-REQUEST");
    }

    #[tokio::test]
    async fn scan_nonexistent_dir_surfaces_stable_code() {
        let app = build_router(state_with(None));
        let res = app
            .clone()
            .oneshot(req(
                "POST",
                "/api/scan",
                Some(json!({"dir": "Z:/definitely/not/here"}).to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let v = body_json(res).await;
        assert!(
            v["code"].as_str().unwrap().starts_with("MF-"),
            "业务码透传: {v}"
        );
    }

    #[tokio::test]
    async fn convert_rejects_missing_fields_loudly() {
        let app = build_router(state_with(None));
        let res = app
            .clone()
            .oneshot(req(
                "POST",
                "/api/convert",
                Some(json!({"source": "x"}).to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let v = body_json(res).await;
        assert_eq!(v["code"], "MF-API-BAD-REQUEST");
    }

    #[tokio::test]
    async fn convert_unknown_plugin_surfaces_stable_code() {
        let app = build_router(state_with(None));
        let res = app
            .clone()
            .oneshot(req(
                "POST",
                "/api/convert",
                Some(
                    json!({"plugin": "no-such-plugin", "source": "C:/x/song.qmcflac"}).to_string(),
                ),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let v = body_json(res).await;
        assert!(
            v["code"].as_str().unwrap().starts_with("MF-PLUGIN-"),
            "cli 桥接稳定码透传: {v}"
        );
    }

    #[tokio::test]
    async fn organize_apply_requires_explicit_confirm() {
        let dir = std::env::temp_dir().join(format!("mf-org-1-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let app = build_router(state_with(None));
        let res = app
            .clone()
            .oneshot(req(
                "POST",
                "/api/organize/apply",
                Some(json!({"dir": dir.display().to_string()}).to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "无 confirm 必须 403");
        let v = body_json(res).await;
        assert_eq!(v["code"], "MF-OP-NEEDS-YES");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn organize_plan_is_read_only_and_apply_moves_with_confirm() {
        let dir = std::env::temp_dir().join(format!("mf-org-2-{}", std::process::id()));
        let lib = dir.join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("a.flac"), b"fLaC-stub-for-plan-test").unwrap();

        let app = build_router(state_with(None));
        let res = app
            .clone()
            .oneshot(req(
                "POST",
                "/api/organize/plan",
                Some(json!({"dir": lib.display().to_string()}).to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let v = body_json(res).await;
        assert_eq!(v["ok"], true);
        assert!(lib.join("a.flac").exists(), "plan 绝不动文件");

        let res = app
            .clone()
            .oneshot(req(
                "POST",
                "/api/organize/apply",
                Some(json!({"dir": lib.display().to_string()}).to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        let res = app
            .clone()
            .oneshot(req(
                "POST",
                "/api/organize/apply",
                Some(json!({"dir": lib.display().to_string(), "confirm": true}).to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "apply 执行成功");
        let v = body_json(res).await;
        let processed = v["data"]["moved"].as_u64().unwrap()
            + v["data"]["skipped"].as_u64().unwrap()
            + v["data"]["failed"].as_u64().unwrap();
        assert!(processed >= 1, "至少处理一项");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn wizard_status_reports_writable_data_dir() {
        let app = build_router(state_with(None));
        let res = app
            .clone()
            .oneshot(req("GET", "/api/wizard/status", None))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let v = body_json(res).await;
        assert_eq!(v["ok"], true);
        assert_eq!(v["data"]["token_ready"], true);
        assert_eq!(v["data"]["data_dir_writable"], true);
    }
}
