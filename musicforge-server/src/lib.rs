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
use axum::extract::DefaultBodyLimit;
use serde_json::json;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

pub mod api;

/// 默认绑定地址（R22：仅回环；B22——常量化供测试断言，测试不再读环境变量）
pub const DEFAULT_BIND: &str = "127.0.0.1:8787";

/// 请求体上限（B11 修复）。原先 32MB 只在**签名分支**里生效（`to_bytes`），
/// 而 `auth_disabled` 分支在读 body 之前便 `return next.run(req)` → 无鉴权
/// 形态（fnOS FPK 默认注入）下**完全无闸**。提到 Router 层后与鉴权解耦。
pub const SERVER_BODY_LIMIT: usize = 32 * 1024 * 1024;

/// 全局并发闸（B12 修复）。`/api/batch` 单次可在 `spawn_blocking` 内起到 10 个
/// OS 线程，无闸时 N 个并发请求 = 最多 10N 线程且无界排队。超出配额的请求
/// **排队**（tower `ConcurrencyLimitLayer` 语义）而非无界增长。
pub const SERVER_MAX_CONCURRENCY: usize = 8;

/// P1-2 可观测性：初始化结构化日志（此前 server 零日志，线上问题只能靠猜）。
///
/// - 输出目标：**stdout**（fpk `cmd/main` 已把 stdout 重定向到 `data/logs/server.log`，
///   因此无需额外文件 writer，即可自然落盘）；
/// - 级别：`MUSICFORGE_LOG`（默认 `info`；排障时可设 `debug`）；
/// - 幂等：`try_init()` —— 测试内多次调用不会 panic。
pub fn init_logging() {
    let filter = std::env::var("MUSICFORGE_LOG").unwrap_or_else(|_| "info".to_string());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .with_target(false)
        .try_init();
}

/// 服务配置（env 注入；fpk `cmd/main` 为主要调用方）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub data_dir: PathBuf,
    pub bind: String,
    pub token: String,
    /// token 文件路径（P9：首启提示用户到此处取完整值——完整 token 不再落日志）。
    pub token_path: PathBuf,
    pub ui_dir: PathBuf,
    /// P8.2.1：音乐库目录（`MUSICFORGE_LIBRARY_DIR`；scan API 缺省根）
    pub library_dir: Option<PathBuf>,
    /// P9：可选路径白名单（`MUSICFORGE_ALLOWED_ROOTS`，`:`/`,` 分隔）。
    /// **空 = 不约束**（家庭内网 + token 闸的默认姿态）；配置后，端点内的
    /// 目录/文件参数必须落在某个 root 内，否则 `403 MF-PATH-NOT-ALLOWED`。
    pub allowed_roots: Vec<PathBuf>,
    /// M2：是否要求请求签名（默认 true；`MUSICFORGE_AUTH_LEGACY=1` → false）。
    pub auth_require_sign: bool,
    /// 鉴权总开关（2026-09-12 产品决策）：`MUSICFORGE_AUTH=off` 时**完全跳过 token 与签名校验**。
    ///
    /// 适用场景：家庭内网 NAS（fnOS 形态由 `cmd/main` 默认注入 off——开箱即用，无需取/填 token）。
    /// **裸跑 server 保持默认 on**（对外暴露场景的安全默认不降级）。
    ///
    /// 关闭鉴权时仍然生效的防线：
    /// - 路径域约束（`MUSICFORGE_ALLOWED_ROOTS`）不变；
    /// - 所有破坏性操作**可回滚**（产物进回收站 + 回滚清单）；
    /// - 启动日志显式 warning + 界面显式提示（降级可见，绝不静默）。
    pub auth_disabled: bool,
}

impl ServerConfig {
    /// 从环境变量构建；token 文件不存在则生成（24B 随机 → base64url）并返回
    /// 生成标记（调用方负责打印到日志——R22：首启显式展示，无默认口令）。
    pub fn from_env() -> Result<(Self, bool), String> {
        let data_dir = PathBuf::from(
            std::env::var("MUSICFORGE_DATA_DIR").unwrap_or_else(|_| "data".to_string()),
        );
        let bind = std::env::var("MUSICFORGE_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_string());
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
        // P9 路径域（宽松可配）：未配置 = 不约束（向后兼容）
        let allowed_roots: Vec<PathBuf> = std::env::var("MUSICFORGE_ALLOWED_ROOTS")
            .unwrap_or_default()
            .split([',', ';', ':'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect();
        // M2：签名校验默认开启；MUSICFORGE_AUTH_LEGACY=1 退回 legacy（迁移逃生门）
        let auth_require_sign = std::env::var("MUSICFORGE_AUTH_LEGACY")
            .map(|v| v.trim() != "1")
            .unwrap_or(true);
        // 鉴权总开关（2026-09-12 产品决策）：MUSICFORGE_AUTH=off → 完全跳过鉴权。
        // fnOS 形态由 cmd/main 默认注入 off；裸跑保持默认 on。
        let auth_disabled = std::env::var("MUSICFORGE_AUTH")
            .map(|v| v.trim().eq_ignore_ascii_case("off"))
            .unwrap_or(false);
        Ok((
            Self {
                data_dir,
                bind,
                token,
                token_path: token_file,
                ui_dir,
                library_dir,
                allowed_roots,
                auth_require_sign,
                auth_disabled,
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
    // B18: 0o600 -- token 属敏感凭据，默认 umask 的 0644 会泄露给同机其他用户
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| format!("token 写入失败: {e}"))?;
        f.write_all(token.as_bytes())
            .map_err(|e| format!("token 写入失败: {e}"))?;
    }
    #[cfg(not(unix))]
    std::fs::write(path, &token).map_err(|e| format!("token 写入失败: {e}"))?;
    Ok((token, true))
}

/// 生成 24B 随机 token（48 位十六进制表示；B17 修复）。
///
/// 熵源优先级：
/// 1. unix `/dev/urandom`（read_exact 精确读 24B——OS CSPRNG）；
/// 2. 兜底（win/其它）：`RandomState` 进程随机种子（SipHash keys 来自 OS
///    CSPRNG，进程内每次 new 都不同）多轮叠加 + 时间/pid 混淆——远强于
///    修复前的纯时间+pid 可预测熵。
fn generate_token() -> String {
    #[cfg(unix)]
    {
        use std::io::Read;
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            let mut buf = [0u8; 24];
            if f.read_exact(&mut buf).is_ok() {
                return buf.iter().map(|b| format!("{b:02x}")).collect();
            }
        }
    }
    #[cfg(windows)]
    {
        // Windows：BCryptGenRandom（OS CSPRNG）。
        //
        // 原 Windows 路径只有「RandomState + 时间 + pid」兜底，而 std 明确声明
        // RandomState 的输出**不适合密码学用途**（P9 审计 Top6）。
        // 零新增依赖：直接链接系统 bcrypt.dll（与 plugin-host 的 raw Win32 FFI
        // 同法），失败才继续落到下面的兜底。
        #[allow(non_snake_case)]
        #[link(name = "bcrypt")]
        extern "system" {
            fn BCryptGenRandom(
                h_algorithm: *mut std::ffi::c_void,
                buffer: *mut u8,
                length: u32,
                flags: u32,
            ) -> i32;
        }
        // 算法句柄传 NULL 时必须带此标志（使用系统首选 RNG）
        const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;
        let mut buf = [0u8; 24];
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                buf.as_mut_ptr(),
                buf.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if status == 0 {
            // STATUS_SUCCESS
            return buf.iter().map(|b| format!("{b:02x}")).collect();
        }
    }
    // 兜底熵：RandomState 种子叠加（每轮 hasher 状态不同）
    use std::hash::{BuildHasher as _, Hasher as _};
    let mut acc: u128 = std::process::id() as u128;
    for i in 0..6u128 {
        let h = std::collections::hash_map::RandomState::new();
        let mut hasher = h.build_hasher();
        hasher.write_u128(acc ^ (i << 96));
        acc = acc.rotate_left(29)
            ^ (hasher.finish() as u128)
            ^ (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as u128)
                .unwrap_or(0)
                << (i * 13 % 96));
    }
    format!("{acc:032x}{:016x}", acc as u64 ^ (acc >> 64) as u64)
}

/// 遮蔽 token：保留首尾各 4 位，其余以 `*` 代替（首启展示用）。
///
/// P9 审计修复：完整 token 曾直接 `println!` 到 stdout，而 fpk 把 stdout
/// 重定向进 `data/logs/server.log` → **凭据明文落进日志文件**（日志常被随手
/// 分享、打包进 bug report）。首启仍需给用户可辨识的线索，故只显示遮蔽
/// 形态 + 文件绝对路径（由 [`ServerConfig::token_path`] 提供）。
pub fn mask_token(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    if chars.len() <= 8 {
        return "*".repeat(chars.len());
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}****{tail}")
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

// ---------------------------------------------------------------- M2：请求签名 --
///
/// M2（RFC-0003 §4.2）：请求签名——防御**重放**（现状静态 X-Token 被抓包后可无限重放）。
///
/// 规范（与前端 `lib/hmac.ts` 逐字节一致，两侧测试共用同一组向量）：
/// ```text
/// canonical = METHOD \n path \n sha256hex(body) \n ts \n nonce
/// sign      = hex(hmac_sha256(key = token, msg = canonical))
/// ```
/// - METHOD 大写；path 不含 query；无 body 时按空串取 sha256；
/// - ts 为 unix 秒；nonce 为 16 字节 hex（32 字符）；签名 64 hex。
pub mod sign {
    use hmac::{Hmac, Mac};
    use sha2::{Digest, Sha256};

    /// 时间窗（秒）：`|now - ts| > WINDOW` 即 `MF-AUTH-STALE`（容忍两端轻微时钟差）。
    pub const WINDOW_SECS: u64 = 60;
    /// nonce 记忆窗口（秒）：取时间窗两倍——过窗的 nonce 已不可能通过 STALE 检查，无需再记。
    pub const NONCE_TTL_SECS: u64 = 120;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// sha256 → 小写 hex。
    pub fn sha256_hex(data: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(data);
        hex(&h.finalize())
    }

    /// 规范化串（签名输入）。
    pub fn canonical(method: &str, path: &str, body: &[u8], ts: &str, nonce: &str) -> String {
        format!(
            "{}\n{}\n{}\n{}\n{}",
            method.to_uppercase(),
            path,
            sha256_hex(body),
            ts,
            nonce
        )
    }

    /// 计算签名（hex）——测试/工具复用。
    pub fn compute(
        token: &str,
        method: &str,
        path: &str,
        body: &[u8],
        ts: &str,
        nonce: &str,
    ) -> String {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(token.as_bytes()).expect("HMAC 接受任意长度 key");
        mac.update(canonical(method, path, body, ts, nonce).as_bytes());
        hex(&mac.finalize().into_bytes())
    }

    /// 一次签名校验的输入（取自请求头与请求体）。
    /// 打包成结构体：避免 `too_many_arguments`（本项目 clippy 零 warning 门禁），也更可读。
    pub struct SignedRequest<'a> {
        pub method: &'a str,
        pub path: &'a str,
        pub body: &'a [u8],
        pub ts: &'a str,
        pub nonce: &'a str,
        pub sig: &'a str,
    }

    /// 校验（时间窗 → 格式 → 签名常量时间比较）。`Err(稳定码)` 由调用方转 401。
    pub fn verify(token: &str, r: SignedRequest<'_>, now: u64) -> Result<(), &'static str> {
        let ts_num: u64 = r.ts.parse().map_err(|_| "MF-AUTH-STALE")?;
        if ts_num.abs_diff(now) > WINDOW_SECS {
            return Err("MF-AUTH-STALE");
        }
        let ok_hex = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_hexdigit());
        if !ok_hex(r.nonce, 32) || !ok_hex(r.sig, 64) {
            return Err("MF-AUTH-SIG-INVALID");
        }
        let expect = compute(token, r.method, r.path, r.body, r.ts, r.nonce);
        if crate::ct_eq(&expect, &r.sig.to_lowercase()) {
            Ok(())
        } else {
            Err("MF-AUTH-SIG-INVALID")
        }
    }
}

/// `/api/*` 鉴权中间件（除 health 外全量）。
///
/// 两种模式（M2，RFC-0003 §4.2）：
/// - **签名模式（默认严格）**：校验 `x-mf-ts / x-mf-nonce / x-mf-sign` —— 时间窗 + nonce 防重放 + HMAC；
/// - **legacy 模式**（`MUSICFORGE_AUTH_LEGACY=1`）：仅静态 token 比较（M2 前行为，迁移逃生门）。
async fn auth_middleware(
    axum::extract::State(state): axum::extract::State<ServerState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    // 鉴权总开关（2026-09-12 产品决策）：`MUSICFORGE_AUTH=off` → 直接放行。
    // 仅适用于受控内网（fnOS 家庭 NAS）；启动日志与界面均显式提示（降级可见）。
    // 注意：路径域（ALLOWED_ROOTS）与破坏类操作的 confirm/回滚**不受影响**。
    if state.auth_disabled {
        return next.run(req).await;
    }

    // B21: 空 token 的 ServerState 属误配置——拒绝服务而非放行（ct_eq("","")=true）
    if state.token.is_empty() {
        // P1-2：误配置（空 token）显式 error——此前完全静默
        tracing::error!("server token is empty (misconfiguration): refusing all /api requests");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"ok": false, "code": "MF-AUTH-REQUIRED",
                "message": "服务端 token 未初始化（误配置）"})),
        )
            .into_response();
    }

    if !req.headers().contains_key("x-mf-sign") {
        // 严格模式：缺签名 → 明确拒绝（提示客户端升级）
        if state.auth_require_sign {
            let delay_ms = state.auth_guard.on_fail();
            tracing::warn!(path = %req.uri().path(), code = "MF-AUTH-SIG-MISSING", delay_ms, "auth rejected");
            if delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"ok": false, "code": "MF-AUTH-SIG-MISSING",
                    "message": "缺少请求签名（请升级客户端：x-mf-ts / x-mf-nonce / x-mf-sign）"})),
            )
                .into_response();
        }
        // legacy 模式（MUSICFORGE_AUTH_LEGACY=1）：仅静态 token 比较（M2 前行为）
        let provided = req
            .headers()
            .get("X-Token")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if ct_eq(provided, &state.token) {
            state.auth_guard.on_ok();
            return next.run(req).await;
        }
        let delay_ms = state.auth_guard.on_fail();
        tracing::warn!(path = %req.uri().path(), code = "MF-AUTH-REQUIRED", delay_ms, "auth rejected");
        if delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok": false, "code": "MF-AUTH-REQUIRED",
                "message": "缺失或错误的 X-Token（token 见服务端首启日志）"})),
        )
            .into_response();
    }

    // 取签名头（在消耗请求体之前）
    let (ts, nonce, sig) = {
        let h = req.headers();
        let g = |k: &str| {
            h.get(k)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string()
        };
        (g("x-mf-ts"), g("x-mf-nonce"), g("x-mf-sign"))
    };

    // body 参与签名（防"换体重放"）→ 缓冲后重建请求；本产品请求体为小 JSON，32MB 为防御上限
    let (parts, body) = req.into_parts();
    let bytes = match axum::body::to_bytes(body, 32 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(json!({"ok": false, "code": "MF-BODY-TOO-LARGE", "message": "请求体过大"})),
            )
                .into_response()
        }
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let method = parts.method.as_str().to_string();
    // 关键：本中间件挂在 `nest("/api")` 内——`parts.uri.path()` 已被剥离前缀（`/version`），
    // 而客户端签的是**完整路径**（`/api/version`）。故优先取 `OriginalUri`（axum 在 nest 时注入）。
    let path = parts
        .extensions
        .get::<axum::extract::OriginalUri>()
        .map(|u| u.0.path().to_string())
        .unwrap_or_else(|| parts.uri.path().to_string());

    let verdict = sign::verify(
        &state.token,
        sign::SignedRequest {
            method: &method,
            path: &path,
            body: &bytes,
            ts: &ts,
            nonce: &nonce,
            sig: &sig,
        },
        now,
    );
    match verdict {
        Ok(()) => {
            if state.nonce_replay(&nonce, now) {
                let delay_ms = state.auth_guard.on_fail();
                tracing::warn!(path = %path, code = "MF-AUTH-REPLAY", delay_ms, "auth rejected");
                if delay_ms > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                }
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"ok": false, "code": "MF-AUTH-REPLAY",
                        "message": "重复的请求签名（nonce 已被使用）"})),
                )
                    .into_response();
            }
            state.auth_guard.on_ok();
            next.run(Request::from_parts(parts, Body::from(bytes)))
                .await
        }
        Err(code) => {
            let delay_ms = state.auth_guard.on_fail();
            tracing::warn!(path = %path, code, delay_ms, "auth rejected: bad request signature");
            if delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            let msg = if code == "MF-AUTH-STALE" {
                "请求时间戳超出窗口（请校准系统时间后重试）"
            } else {
                "请求签名无效"
            };
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"ok": false, "code": code, "message": msg})),
            )
                .into_response()
        }
    }
}

// ---------------------------------------------------------------- P2-3 认证限流 --

/// 认证失败阈值：连续失败达到该次数后，后续失败响应附加延迟。
const AUTH_FAIL_DELAY_THRESHOLD: u32 = 10;
/// 附加延迟（毫秒）——把暴力猜测压到 ~1 次/秒；正常用户打错 1-2 次无感。
const AUTH_FAIL_DELAY_MS: u64 = 1000;
/// 计数衰减窗口（秒）：距上次失败超过该窗口则重新计数（避免永久惩罚）。
const AUTH_FAIL_WINDOW_SECS: u64 = 60;

/// 认证失败限流器（P2-3，防暴力猜 token）。
///
/// 设计取舍（无新依赖，纯 std 原子量）：
/// - **只延迟、不封禁**——NAS 家庭内网里误封自己（如忘记 token）比被猜更常见；
/// - 阈值内零延迟，正常交互不受影响；
/// - 成功一次即清零；跨窗口自动衰减。
#[derive(Debug)]
pub struct AuthGuard {
    fails: std::sync::atomic::AtomicU32,
    /// 上次失败时刻（unix 秒；0 = 从未失败）
    last_fail_secs: std::sync::atomic::AtomicU64,
}

impl AuthGuard {
    pub fn new() -> Self {
        Self {
            fails: std::sync::atomic::AtomicU32::new(0),
            last_fail_secs: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// 记录一次失败，返回本次响应应附加的延迟（毫秒）。
    pub fn on_fail(&self) -> u64 {
        use std::sync::atomic::Ordering;
        let now = Self::now_secs();
        let prev = self.last_fail_secs.swap(now, Ordering::Relaxed);
        let n = if now.saturating_sub(prev) > AUTH_FAIL_WINDOW_SECS {
            self.fails.store(1, Ordering::Relaxed);
            1
        } else {
            self.fails.fetch_add(1, Ordering::Relaxed) + 1
        };
        if n >= AUTH_FAIL_DELAY_THRESHOLD {
            AUTH_FAIL_DELAY_MS
        } else {
            0
        }
    }

    /// 认证成功：清零计数。
    pub fn on_ok(&self) {
        use std::sync::atomic::Ordering;
        self.fails.store(0, Ordering::Relaxed);
    }
}

impl Default for AuthGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// 共享状态（token + ui_dir + data_dir + library_dir）。
#[derive(Clone)]
pub struct ServerState {
    pub token: String,
    /// P2-3：认证失败限流（Arc 共享，跨请求累积）
    pub auth_guard: std::sync::Arc<AuthGuard>,
    pub ui_dir: PathBuf,
    /// P8.2.1：wizard 探测 + 未来 DB 路径基准
    pub data_dir: PathBuf,
    /// P8.2.1：scan API 缺省根（fpk `MUSICFORGE_LIBRARY_DIR`）
    pub library_dir: Option<PathBuf>,
    /// P9：路径域白名单（空 = 不约束）
    pub allowed_roots: Vec<PathBuf>,
    /// M2（RFC-0003 §4.2）：是否要求请求签名（**默认 true**）。
    /// `MUSICFORGE_AUTH_LEGACY=1` 时关闭（退回仅静态 token 比较）——迁移逃生门。
    pub auth_require_sign: bool,
    /// 鉴权总开关（2026-09-12 决策）：true = **完全跳过 token 与签名校验**（直接放行）。
    /// fnOS 形态由 `cmd/main` 默认注入 `MUSICFORGE_AUTH=off`；裸跑保持 false（安全默认）。
    pub auth_disabled: bool,
    /// M2：已见 nonce（防重放）——nonce → 过期 unix 秒
    pub nonce_seen: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, u64>>>,
}

impl ServerState {
    /// M2：nonce 去重（返回 `true` = 重放）。
    /// 检查与插入在同一临界区完成（原子）；顺带清理过期项，避免 map 无限增长。
    pub fn nonce_replay(&self, nonce: &str, now: u64) -> bool {
        let mut m = self.nonce_seen.lock().unwrap_or_else(|e| e.into_inner());
        m.retain(|_, exp| *exp > now);
        if m.contains_key(nonce) {
            return true;
        }
        m.insert(nonce.to_string(), now + sign::NONCE_TTL_SECS);
        false
    }
}

/// B12：在途 `/api` 请求计数（并发闸状态）。
static API_INFLIGHT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// B12 并发闸：`/api/*` 最多同时放行 [`SERVER_MAX_CONCURRENCY`] 个请求，
/// 超出者**立即 503**（而非排队——无上界的等待队列本身就是一个 DoS 面）。
///
/// 背景：`/api/batch` 单次可在 `spawn_blocking` 内起到 10 个 OS 线程，无闸时
/// N 个并发请求 = 最多 10N 个线程 + 阻塞池排队。
///
/// **作用域**：只挂在 `/api` nest 上——`/api/health`（生命周期探测）与静态
/// 资源在外层 Router，不参与计数，业务繁忙时不会误报“服务不可用”。
/// 自实现计数而非引入 `tower::limit`，是为了不动依赖 feature。
async fn concurrency_limit(req: Request<Body>, next: Next) -> Response {
    use std::sync::atomic::Ordering;
    // 配额用尽时**排队等待**而非拒绝：硬拒会凭空制造一种客户端必须处理的
    // 新失败模式（随机 503），与既有行为不兼容（tower ConcurrencyLimit 亦
    // 采用队列语义）。CAS 而非「load + fetch_add」，避免并发下略微超限。
    loop {
        let cur = API_INFLIGHT.load(Ordering::Acquire);
        if cur < SERVER_MAX_CONCURRENCY
            && API_INFLIGHT
                .compare_exchange_weak(cur, cur + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    let res = next.run(req).await;
    API_INFLIGHT.fetch_sub(1, Ordering::AcqRel);
    res
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
                .route("/library/refresh", post(api::library_refresh))
                .route("/convert", post(api::convert))
                .route("/batch", post(api::batch))
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
                            "message": "API 面随 P8 迭代逐域开放（当前 health/version/wizard/scan/convert/organize/clean/trash）"
                        })),
                    )
                })
                .layer(middleware::from_fn_with_state(
                    state.clone(),
                    auth_middleware,
                ))
                // B12：并发闸在 nest 最外层 → 无论鉴权开关如何都生效
                .layer(middleware::from_fn(concurrency_limit)),
        )
        .fallback_service(spa_service(ui_dir))
        // 静态资源缓存策略（2026-09-12 真机问题根治）：`no-cache` = 每次协商（304 极廉价），
        // 但**杜绝"升级 fpk 后浏览器仍运行旧 SPA"**——旧前端不带请求签名，
        // 会让所有 API 报 MF-AUTH-SIG-MISSING（M2 上线后升级必现）。
        // assets 文件名带内容 hash 本可长缓存；局域网下协商成本可忽略，统一策略换"升级即生效"。
        .layer(SetResponseHeaderLayer::if_not_present(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-cache"),
        ))
        // B11（P0）：体量闸提到 Router 层，与鉴权**解耦**——原先 32MB 上限只在
        // 签名分支内生效（`to_bytes`），`auth_disabled` 分支在读 body 之前便
        // `return next.run(req)` → fnOS 默认形态下完全无闸。
        .layer(DefaultBodyLimit::max(SERVER_BODY_LIMIT))
        // P1-2：请求日志（method / path / status / 耗时）——只记路径，**不含
        // query 与 header**，因此 token 不会进入日志。
        .layer(
            tower_http::trace::TraceLayer::new_for_http()
                .make_span_with(|req: &Request<Body>| {
                    tracing::info_span!(
                        "req",
                        method = %req.method(),
                        path = %req.uri().path()
                    )
                })
                .on_response(
                    |res: &Response, latency: std::time::Duration, _span: &tracing::Span| {
                        let ms = latency.as_millis();
                        let status = res.status().as_u16();
                        if status >= 500 {
                            tracing::error!(status, ms, "request failed");
                        } else if status >= 400 {
                            tracing::warn!(status, ms, "request rejected");
                        } else {
                            tracing::info!(status, ms, "request done");
                        }
                    },
                ),
        )
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
            auth_guard: std::sync::Arc::new(AuthGuard::new()),
            ui_dir: PathBuf::from("ui"),
            data_dir: std::env::temp_dir().join(format!("mf-srv-test-{}", std::process::id())),
            library_dir: None,
            allowed_roots: Vec::new(),
            // 既有集成测试聚焦业务逻辑（不带签名）→ legacy 模式；
            // M2 签名路径由下方专项测试覆盖。
            auth_require_sign: false,
            // 默认保持鉴权开启（安全默认）；开关行为由专项测试覆盖。
            auth_disabled: false,
            nonce_seen: std::sync::Arc::new(
                std::sync::Mutex::new(std::collections::HashMap::new()),
            ),
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

    /// P2-3 回归：阈值内零延迟（正常误输无感）→ 达阈值后附加延迟 → 成功清零
    #[test]
    fn auth_guard_delays_only_after_threshold_and_resets() {
        let g = AuthGuard::new();
        for i in 1..AUTH_FAIL_DELAY_THRESHOLD {
            assert_eq!(g.on_fail(), 0, "第 {i} 次失败不应延迟（阈值内）");
        }
        assert!(
            g.on_fail() > 0,
            "达到阈值（{AUTH_FAIL_DELAY_THRESHOLD}）后应附加延迟"
        );
        assert!(g.on_fail() > 0, "阈值之上持续延迟");
        g.on_ok();
        assert_eq!(g.on_fail(), 0, "认证成功一次即清零计数");
    }

    #[test]
    fn ct_eq_semantics() {
        assert!(ct_eq("abc", "abc"));
        assert!(!ct_eq("abc", "abd"));
        assert!(!ct_eq("abc", "abcd"));
        assert!(!ct_eq("", "a"));
        assert!(ct_eq("", ""));
    }

    /// B17 回归：token 熵质量——两次生成必不同、长度恒 48 hex（24B）、非零
    #[test]
    fn generate_token_entropy_quality() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 48, "24B hex 表示恒 48 字符: {a}");
        assert_eq!(b.len(), 48);
        assert_ne!(a, b, "连续两次生成必须不同");
        assert!(a.chars().all(|ch| ch.is_ascii_hexdigit()));
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

    /// P9 审计修复：首启展示用遮蔽——中段不得出现原字符，长度保持可辨识。
    #[test]
    fn mask_token_hides_middle() {
        let t = "0123456789abcdef0123456789abcdef0123456789abcdef";
        let m = mask_token(t);
        assert!(m.starts_with("0123"), "保留前 4 位便于核对");
        assert!(m.ends_with("cdef"), "保留后 4 位便于核对");
        assert!(!m.contains("456789"), "中段必须被遮蔽（原 token 不得完整落日志）");
        assert!(m.len() < t.len(), "中段被压缩为固定 4 个 *");
        // 过短 token：全遮蔽，不泄露任何字符
        assert_eq!(mask_token("abc"), "***");
    }

    /// token 熵：24B → 48 位十六进制；两次生成不得相同。
    /// （Windows 走 BCryptGenRandom = OS CSPRNG，取代原非密码学的兜底熵）
    #[test]
    fn generate_token_is_48_hex_and_unique() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 48, "24B → 48 位十六进制");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b, "两次生成不得相同");
    }

    #[test]
    fn config_defaults_are_loopback() {
        // B22：常量断言（原实现读环境变量——CI 设置该 env 时断言失效）
        assert!(
            DEFAULT_BIND.starts_with("127.0.0.1"),
            "默认绑定必须仅回环: {DEFAULT_BIND}"
        );
    }

    // ------------------------------------------------------------ M2：请求签名 --
    // 跨语言测试向量（与前端 `src/lib/hmac.test.ts` 共用同一组；由 node:crypto 生成）——
    // 任一侧的 canonical/HMAC 实现发生漂移，本组测试会立即变红。

    const VEC_TOKEN: &str = "test-token";
    const VEC_TS: &str = "1760000000";
    const VEC_NONCE: &str = "00112233445566778899aabbccddeeff";
    const VEC_SIG_POST: &str = "0a759dd225ade9ae7d3fdda28cfa090f0854d05347aba5886be5d6f2c55db3f4";
    const VEC_SIG_GET: &str = "fe03de00ac0eaecc8faf137667db7097d61b8243e572e25ed26f5ea58df5edc4";

    #[test]
    fn sign_matches_cross_language_vector() {
        assert_eq!(
            sign::compute(
                VEC_TOKEN,
                "POST",
                "/api/batch",
                br#"{"a":1}"#,
                VEC_TS,
                VEC_NONCE
            ),
            VEC_SIG_POST
        );
        assert_eq!(
            sign::compute(VEC_TOKEN, "GET", "/api/version", b"", VEC_TS, VEC_NONCE),
            VEC_SIG_GET
        );
    }

    /// 测试便捷封装：固定向量 token，把校验输入收敛为结构体。
    fn v(
        method: &str,
        path: &str,
        body: &[u8],
        ts: &str,
        nonce: &str,
        sig: &str,
        now: u64,
    ) -> Result<(), &'static str> {
        sign::verify(
            VEC_TOKEN,
            sign::SignedRequest {
                method,
                path,
                body,
                ts,
                nonce,
                sig,
            },
            now,
        )
    }

    #[test]
    fn sign_verify_window_format_and_tamper() {
        let body = br#"{"a":1}"#;
        let at = 1760000000u64;
        // 正确 + 窗口边界（±60s）
        assert!(v(
            "POST",
            "/api/batch",
            body,
            VEC_TS,
            VEC_NONCE,
            VEC_SIG_POST,
            at
        )
        .is_ok());
        assert!(v(
            "POST",
            "/api/batch",
            body,
            VEC_TS,
            VEC_NONCE,
            VEC_SIG_POST,
            at + 60
        )
        .is_ok());
        // 超窗 / ts 非数字 → STALE
        assert_eq!(
            v(
                "POST",
                "/api/batch",
                body,
                VEC_TS,
                VEC_NONCE,
                VEC_SIG_POST,
                at + 61
            ),
            Err("MF-AUTH-STALE")
        );
        assert_eq!(
            v(
                "POST",
                "/api/batch",
                body,
                "abc",
                VEC_NONCE,
                VEC_SIG_POST,
                at
            ),
            Err("MF-AUTH-STALE")
        );
        // body 篡改（换体重放）→ 签名不符
        assert_eq!(
            v(
                "POST",
                "/api/batch",
                br#"{"a":2}"#,
                VEC_TS,
                VEC_NONCE,
                VEC_SIG_POST,
                at
            ),
            Err("MF-AUTH-SIG-INVALID")
        );
        // path 篡改（同一签名挪到别的端点）→ 拒绝
        assert_eq!(
            v(
                "POST",
                "/api/clean/apply",
                body,
                VEC_TS,
                VEC_NONCE,
                VEC_SIG_POST,
                at
            ),
            Err("MF-AUTH-SIG-INVALID")
        );
        // token 不对 → 签名不符
        assert_eq!(
            sign::verify(
                "other-token",
                sign::SignedRequest {
                    method: "POST",
                    path: "/api/batch",
                    body,
                    ts: VEC_TS,
                    nonce: VEC_NONCE,
                    sig: VEC_SIG_POST,
                },
                at
            ),
            Err("MF-AUTH-SIG-INVALID")
        );
        // nonce 格式非法 → 拒绝（不进 nonce 表）
        assert_eq!(
            v("POST", "/api/batch", body, VEC_TS, "zz", VEC_SIG_POST, at),
            Err("MF-AUTH-SIG-INVALID")
        );
    }

    #[tokio::test]
    async fn strict_mode_rejects_missing_signature() {
        let mut st = test_state("tok-123");
        st.auth_require_sign = true;
        let app = build_router(st);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/api/version")
                    .header("x-token", "tok-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let v: serde_json::Value =
            serde_json::from_slice(&to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert_eq!(v["code"], "MF-AUTH-SIG-MISSING");
    }

    #[tokio::test]
    async fn legacy_mode_still_accepts_static_token() {
        // test_state 默认 auth_require_sign=false（legacy）——静态 token 应通过
        let app = build_router(test_state("tok-123"));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/api/version")
                    .header("x-token", "tok-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "legacy 模式静态 token 应通过");
    }

    #[tokio::test]
    async fn signed_request_accepted_and_replay_blocked() {
        let mut st = test_state("tok-123");
        st.auth_require_sign = true;
        let app = build_router(st);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let ts = now.to_string();
        let nonce = "aabbccddeeff00112233445566778899";
        let mk = |n: &str| {
            let s = sign::compute("tok-123", "GET", "/api/version", b"", &ts, n);
            Request::builder()
                .uri("/api/version")
                .header("x-token", "tok-123")
                .header("x-mf-ts", ts.clone())
                .header("x-mf-nonce", n)
                .header("x-mf-sign", s)
                .body(Body::empty())
                .unwrap()
        };
        let res = app.clone().oneshot(mk(nonce)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "带正确签名的请求应通过");
        // 同 nonce 重放 → MF-AUTH-REPLAY
        let res2 = app.oneshot(mk(nonce)).await.unwrap();
        assert_eq!(res2.status(), StatusCode::UNAUTHORIZED);
        let v: serde_json::Value =
            serde_json::from_slice(&to_bytes(res2.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert_eq!(v["code"], "MF-AUTH-REPLAY");
    }

    // ------------------------------------------------------ SPA 缓存策略 --
    // 2026-09-12 真机问题：升级 fpk 后浏览器仍运行**缓存的旧 SPA**，而旧前端不带
    // 请求签名 → 所有 API 报 MF-AUTH-SIG-MISSING。修复：SPA 响应统一 `no-cache`
    // （每次协商，304 极廉价），本断言防回归。
    #[tokio::test]
    async fn spa_responses_carry_no_cache() {
        let dir = std::env::temp_dir().join(format!("mf-spa-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), "<html>ok</html>").unwrap();
        let mut st = test_state("tok-123");
        st.ui_dir = dir.clone();
        let app = build_router(st);
        // 深链（走 fallback → index.html）与根路径都必须带 no-cache
        for uri in ["/", "/library/scan"] {
            let res = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(
                res.status(),
                StatusCode::OK,
                "SPA 请求 {uri} 应命中 index.html"
            );
            assert_eq!(
                res.headers()
                    .get("cache-control")
                    .map(|v| v.to_str().unwrap()),
                Some("no-cache"),
                "SPA 响应 {uri} 必须 no-cache（否则升级后浏览器运行旧 JS）"
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    // ---------------------------------------------- 鉴权总开关（2026-09-12）--
    // 产品决策：fnOS 形态默认 `MUSICFORGE_AUTH=off`（开箱即用）。
    // 两条对照断言：开关必须**真的生效**，且**默认安全不降级**。

    #[tokio::test]
    async fn auth_disabled_allows_requests_without_token() {
        let mut st = test_state("tok-123");
        st.auth_disabled = true;
        st.auth_require_sign = true; // 即使同时要求签名，off 也应完全放行
        let app = build_router(st);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/api/version")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "鉴权关闭时，无 token / 无签名也必须可访问（开箱即用）"
        );
    }

    #[tokio::test]
    async fn auth_enabled_still_rejects_without_token() {
        // 对照：默认（auth_disabled=false，legacy 签名模式）下无 token 仍应 401
        let app = build_router(test_state("tok-123"));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/api/version")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::UNAUTHORIZED,
            "鉴权开启时，无 token 必须拒绝（安全默认不因新增开关而降级）"
        );
    }

    // ------------------------------------------------- B11 / B12（P0 修复）--
    /// 体量闸必须**独立于鉴权开关**生效：原先 32MB 上限只在签名分支内
    /// （`to_bytes`），`auth_disabled`（fnOS 默认注入）分支在读 body 之前便
    /// `return next.run(req)` → 该形态下完全没有体量闸。
    #[tokio::test]
    async fn body_limit_applies_even_when_auth_disabled() {
        let mut st = test_state("tok-123");
        st.auth_disabled = true;
        let app = build_router(st);
        let big = vec![0u8; SERVER_BODY_LIMIT + 1];
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    // 必须挑**必填 body** 的端点：`/api/scan` 用的是
                    // `Option<Json<..>>`，它会吞掉 413 拒绝并退化成 None → 400，
                    // 观察不到体量闸是否生效。
                    .uri("/api/batch")
                    .header("content-type", "application/json")
                    .body(Body::from(big))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::PAYLOAD_TOO_LARGE,
            "无鉴权形态下，超过体量闸的请求也必须被拒"
        );
    }
}
