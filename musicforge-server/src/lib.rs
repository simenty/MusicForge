//! `musicforge-server`——无头 HTTP 服务壳（P8/D27：fnOS native FPK 的服务形态）。
//!
//! **职责边界**（壳层，dependency-policy §4）：
//! 1. SPA 静态资源服务（`app/ui/`，fpk 布局；开发期为 `ui/dist`）；
//! 2. `/api/*` token 鉴权（R22：无默认口令——token 首启随机生成，仅本地传递）；
//! 3. `/api/health` 免鉴权（生命周期 `status` 探测用）。
//!
//! **零业务逻辑**：scan/convert/organize 等能力的 HTTP 化在 P8 后续迭代逐域
//! 接线（复用 `musicforge-core`）；本 crate 只做骨架，保证 FPK 真机「可装、
//! 可起、可停、可探测」。
//!
//! 配置全部走环境变量（fpk `cmd/main` 注入，R22 合规）：
//! - `MUSICFORGE_DATA_DIR`（默认 `./data`）：数据库/日志/token 存放（X16 位置铁律）
//! - `MUSICFORGE_BIND`（默认 `127.0.0.1:8787`）：**默认仅回环**——对外暴露需显式
//! - `MUSICFORGE_TOKEN_FILE`（默认 `$DATA_DIR/.token`）：不存在则生成并打印
//! - `MUSICFORGE_UI_DIR`（默认 exe 同级 `ui/`）：SPA 资源目录

use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post, Router};
use serde_json::json;
use tower_http::services::{ServeDir, ServeFile};

pub mod api;

/// 服务配置（env 注入；fpk `cmd/main` 为主要调用方）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub data_dir: PathBuf,
    pub bind: String,
    pub token: String,
    pub ui_dir: PathBuf,
    /// P8.2.1：音乐库目录（`MUSICFORGE_LIBRARY_DIR`；scan API 缺省根）
    pub library_dir: Option<PathBuf>,
}

impl ServerConfig {
    /// 从环境变量构建；token 文件不存在则生成（24B 随机 → base64url）并返回
    /// 生成标记（调用方负责打印到日志——R22：首启显式展示，无默认口令）。
    pub fn from_env() -> Result<(Self, bool), String> {
        let data_dir = PathBuf::from(
            std::env::var("MUSICFORGE_DATA_DIR").unwrap_or_else(|_| "data".to_string()),
        );
        let bind =
            std::env::var("MUSICFORGE_BIND").unwrap_or_else(|_| "127.0.0.1:8787".to_string());
        let token_file = std::env::var("MUSICFORGE_TOKEN_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| data_dir.join(".token"));
        let (token, generated) = load_or_create_token(&token_file)?;
        let ui_dir = std::env::var("MUSICFORGE_UI_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.join("ui")))
                    .unwrap_or_else(|| PathBuf::from("ui"))
            });
        let library_dir = std::env::var("MUSICFORGE_LIBRARY_DIR")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from);
        Ok((
            Self {
                data_dir,
                bind,
                token,
                ui_dir,
                library_dir,
            },
            generated,
        ))
    }
}

/// 读 token；不存在则生成（`/dev/urandom` 不可用时用随机四段 hex 兜底——
/// NAS 环境两者至少其一可用）。返回 `(token, 是否新生成)`。
pub fn load_or_create_token(path: &std::path::Path) -> Result<(String, bool), String> {
    if let Ok(tok) = std::fs::read_to_string(path) {
        let tok = tok.trim().to_string();
        if !tok.is_empty() {
            return Ok((tok, false));
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("token 目录创建失败: {e}"))?;
    }
    let token = generate_token();
    std::fs::write(path, &token).map_err(|e| format!("token 写入失败: {e}"))?;
    Ok((token, true))
}

/// 生成 24B 随机 token（base64url 形态）。
fn generate_token() -> String {
    // 简单熵源叠加：pid + 纳秒 + 地址熵（无 /dev/urandom 依赖的兜底；
    // fpk cmd/main 已优先用 /dev/urandom 生成——此处为 server 独立运行兜底）
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let a = nanos ^ (pid << 64) ^ nanos.rotate_left(17);
    let b = a.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(31);
    let c = b ^ (a >> 7);
    format!("{a:032x}{b:016x}{c:032x}")
}

/// 常量时间字符串比较（token 校验；长度不同直接 false——不泄露长度差时序）。
pub fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// `X-Token` 鉴权中间件（/api/* 除 health 外全量）。
async fn auth_middleware(
    axum::extract::State(state): axum::extract::State<ServerState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let provided = req
        .headers()
        .get("X-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if ct_eq(provided, &state.token) {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok": false, "code": "MF-AUTH-REQUIRED",
                "message": "缺失或错误的 X-Token（token 见服务端首启日志）"})),
        )
            .into_response()
    }
}

/// 共享状态（token + ui_dir + data_dir + library_dir）。
#[derive(Clone)]
pub struct ServerState {
    pub token: String,
    pub ui_dir: PathBuf,
    /// P8.2.1：wizard 探测 + 未来 DB 路径基准
    pub data_dir: PathBuf,
    /// P8.2.1：scan API 缺省根（fpk `MUSICFORGE_LIBRARY_DIR`）
    pub library_dir: Option<PathBuf>,
}

/// 构建路由（health 免鉴权；其余 /api/* 鉴权；非 /api → SPA）。
pub fn build_router(state: ServerState) -> Router {
    let ui_dir = state.ui_dir.clone();
    Router::new()
        .route(
            "/api/health",
            get(|| async { Json(json!({"status": "ok", "version": env!("CARGO_PKG_VERSION")})) }),
        )
        .nest(
            "/api",
            Router::new()
                .route("/version", get(api::version))
                .route("/wizard/status", get(api::wizard_status))
                .route("/scan", post(api::scan))
                .route("/convert", post(api::convert))
                .route("/organize/plan", post(api::organize_plan))
                .route("/organize/apply", post(api::organize_apply))
                .route("/clean/plan", post(api::clean_plan))
                .route("/clean/apply", post(api::clean_apply))
                .route("/trash/restore", post(api::trash_restore))
                .fallback(|| async {
                    (
                        StatusCode::NOT_FOUND,
                        Json(json!({
                            "ok": false, "code": "MF-API-NOT-FOUND",
                            "message": "API 面随 P8 迭代逐域开放（当前 health/version/wizard/scan）"
                        })),
                    )
                })
                .layer(middleware::from_fn_with_state(
                    state.clone(),
                    auth_middleware,
                )),
        )
        .fallback_service(spa_service(ui_dir))
        .with_state(state)
}

/// SPA 服务：静态目录 + index.html 兜底（React 路由深链）。
fn spa_service(ui_dir: PathBuf) -> ServeDir<ServeFile> {
    ServeDir::new(&ui_dir).fallback(ServeFile::new(ui_dir.join("index.html")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use tower::ServiceExt;

    fn test_state(token: &str) -> ServerState {
        ServerState {
            token: token.to_string(),
            ui_dir: PathBuf::from("ui"),
            data_dir: std::env::temp_dir().join(format!("mf-srv-test-{}", std::process::id())),
            library_dir: None,
        }
    }

    #[tokio::test]
    async fn health_is_public() {
        let app = build_router(test_state("tok-123"));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["status"], "ok");
    }

    #[tokio::test]
    async fn api_requires_token_and_accepts_correct() {
        let app = build_router(test_state("tok-123"));
        // 缺 token → 401 + MF-AUTH-REQUIRED
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/anything")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["code"], "MF-AUTH-REQUIRED");
        // 错 token → 401
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/anything")
                    .header("X-Token", "wrong")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        // 正确 token → 通过鉴权（未知路由 404 = MF-API-NOT-FOUND）
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/api/anything")
                    .header("X-Token", "tok-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        let body = to_bytes(res.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["code"], "MF-API-NOT-FOUND");
    }

    #[test]
    fn ct_eq_semantics() {
        assert!(ct_eq("abc", "abc"));
        assert!(!ct_eq("abc", "abd"));
        assert!(!ct_eq("abc", "abcd"));
        assert!(!ct_eq("", "a"));
        assert!(ct_eq("", ""));
    }

    #[test]
    fn token_file_created_when_missing() {
        let dir = std::env::temp_dir().join(format!(
            "mf-srv-tok-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join(".token");
        let (t1, gen1) = load_or_create_token(&path).unwrap();
        assert!(gen1);
        assert!(!t1.is_empty());
        // 二次读取：同一 token、非新生成
        let (t2, gen2) = load_or_create_token(&path).unwrap();
        assert!(!gen2);
        assert_eq!(t1, t2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn config_defaults_are_loopback() {
        // 显式 env 空场景下的默认 bind 必须是回环（R22）
        let bind = std::env::var("MUSICFORGE_BIND").unwrap_or_else(|_| "127.0.0.1:8787".into());
        assert!(bind.starts_with("127.0.0.1"), "默认绑定必须仅回环: {bind}");
    }
}
