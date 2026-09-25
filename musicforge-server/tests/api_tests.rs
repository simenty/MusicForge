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
        // 鉴权保持开启（安全默认）；开关行为见 lib.rs 专项测试
        auth_disabled: false,
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

/// P9 审计回归（Top1 越权写）：配置 `allowed_roots` 时，`/api/batch` 的
/// **`out_dir`** 必须与其它端点一样受路径域约束。
///
/// 原实现只校验 `inputs`，`out_dir`（及由其派生的回滚清单路径）未校验 →
/// fnOS 默认 `AUTH=off` + 空白名单场景下，同网段调用方可把转码产物写到
/// 白名单之外的任意路径。
#[tokio::test]
async fn batch_out_dir_outside_allowed_roots_is_403() {
    let dir = std::env::temp_dir().join(format!("mf-batch-{}", std::process::id()));
    let allowed = dir.join("allowed");
    let outside = dir.join("outside");
    std::fs::create_dir_all(&allowed).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let src = allowed.join("a.flac");
    std::fs::write(&src, b"fake").unwrap();

    let batch_body = |out: &std::path::Path| {
        json!({
            "inputs": [{"path": src.display().to_string()}],
            "out_dir": out.display().to_string(),
            "dry_run": true
        })
        .to_string()
    };

    // 1) out_dir 在白名单外 → 403 MF-PATH-NOT-ALLOWED
    let mut st = state_with();
    st.allowed_roots = vec![allowed.clone()];
    let app = build_router(st);
    let res = app
        .clone()
        .oneshot(req("POST", "/api/batch", Some(batch_body(&outside))))
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::FORBIDDEN,
        "out_dir 在白名单外必须拒绝（否则可越权写入任意路径）"
    );
    let v = body_json(res).await;
    assert_eq!(v["code"], "MF-PATH-NOT-ALLOWED");

    // 2) 正向对照：out_dir 在白名单内 → 不得因路径域被拒（不误伤合法调用）
    let res_ok = app
        .oneshot(req("POST", "/api/batch", Some(batch_body(&allowed))))
        .await
        .unwrap();
    assert_ne!(
        res_ok.status(),
        StatusCode::FORBIDDEN,
        "白名单内的 out_dir 不得被路径域拒绝"
    );

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
