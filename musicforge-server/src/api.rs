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
    async fn wizard_status_reports_writable_data_dir() {
        let app = build_router(state_with(None));
        let res = app
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
