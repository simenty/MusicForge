//! P8.2.4 API 集成测试：clean/plan 只读、clean/apply 与 trash/restore 的
//! safety 强制（confirm !== true → 403 MF-OP-NEEDS-YES）。

use axum::body::Body;
use axum::http::{Request, StatusCode};
use musicforge_server::{build_router, ServerState};
use serde_json::{json, Value};
use std::path::PathBuf;
use tower::ServiceExt;

fn state_with() -> ServerState {
    ServerState {
        token: "tok-test".to_string(),
        auth_guard: std::sync::Arc::new(musicforge_server::AuthGuard::new()),
        ui_dir: PathBuf::from("ui"),
        data_dir: std::env::temp_dir().join(format!("mf-int-{}", std::process::id())),
        library_dir: None,
        allowed_roots: Vec::new(),
        // 集成测试聚焦端点语义（不带签名）→ legacy 模式；M2 签名路径见 lib.rs 专项测试
        auth_require_sign: false,
        nonce_seen: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
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

async fn body_json(res: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn clean_plan_is_read_only() {
    let dir = std::env::temp_dir().join(format!("mf-cln-{}", std::process::id()));
    let lib = dir.join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("junk.txt"), b"x").unwrap();

    let app = build_router(state_with());
    let res = app
        .clone()
        .oneshot(req(
            "POST",
            "/api/clean/plan",
            Some(json!({"dir": lib.display().to_string()}).to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["ok"], true);
    assert!(lib.join("junk.txt").exists(), "plan 绝不动文件");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn clean_apply_without_confirm_is_403() {
    let dir = std::env::temp_dir().join(format!("mf-cln-a-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let app = build_router(state_with());
    let res = app
        .oneshot(req(
            "POST",
            "/api/clean/apply",
            Some(json!({"dir": dir.display().to_string()}).to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let v = body_json(res).await;
    assert_eq!(v["code"], "MF-OP-NEEDS-YES");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn trash_restore_without_confirm_is_403() {
    let app = build_router(state_with());
    let res = app
        .oneshot(req(
            "POST",
            "/api/trash/restore",
            Some(json!({}).to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let v = body_json(res).await;
    assert_eq!(v["code"], "MF-OP-NEEDS-YES");
}
