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

/// P9 路径域校验：配置 `MUSICFORGE_ALLOWED_ROOTS` 后，参数路径必须落在某个
/// root 内（自身或其子路径）；**白名单为空 = 不约束**（默认姿态，向后兼容）。
#[allow(clippy::result_large_err)] // Response 直返（与 parse_organize_req 同处理）
fn ensure_allowed(state: &ServerState, p: &std::path::Path) -> Result<(), Response> {
    if state.allowed_roots.is_empty() {
        return Ok(());
    }
    let allowed = state
        .allowed_roots
        .iter()
        .any(|root| p == root.as_path() || p.starts_with(root));
    if allowed {
        Ok(())
    } else {
        // P1-2：路径域拒绝 = 安全事件（可能是越权尝试或配置过窄）→ warn 留痕
        tracing::warn!(
            path = %p.display(),
            roots = state.allowed_roots.len(),
            "path rejected: outside MUSICFORGE_ALLOWED_ROOTS"
        );
        Err(err(
            StatusCode::FORBIDDEN,
            "MF-PATH-NOT-ALLOWED",
            format!(
                "路径不在允许根目录内: {}（已配置 MUSICFORGE_ALLOWED_ROOTS）",
                p.display()
            ),
        ))
    }
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
    // P9 路径域（配置白名单时生效）
    if let Err(r) = ensure_allowed(&state, &dir) {
        return r;
    }
    let max_depth = body
        .as_ref()
        .and_then(|b| b.0.get("max_depth"))
        .and_then(|v| v.as_u64())
        .map(|v| v.min(512) as usize)
        .unwrap_or(64);
    let recursive = body
        .as_ref()
        .and_then(|b| b.0.get("recursive"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let options = ScanOptions {
        recursive,
        max_depth,
        ..ScanOptions::default()
    };
    // AUD-5（并发修复）：scan 为同步阻塞 IO（大库可达秒级）——移出 tokio worker
    // 线程（spawn_blocking），避免并发请求时卡死 reactor。
    let scan_result = tokio::task::spawn_blocking(move || scan_library(&dir, &options)).await;
    match scan_result {
        Ok(Ok(report)) => ok(scan_report_json(report)),
        Ok(Err(e)) => err_from(e),
        Err(e) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "MF-INTERNAL",
            format!("扫描任务执行失败: {e}"),
        ),
    }
}

/// `POST /api/library/refresh`：P8 LibraryRefresher——库级增量重扫
///（扫描 + D17 增量哈希缓存刷新/入库；db 落服务端数据目录，D16 本地铁律）。
///
/// 请求 `{ "dir"?, "recursive"?, "max_depth"? }`（dir 缺省同 `/api/scan`）。
/// 返回 `{ scannedFiles, scannedDirs, audio, cacheHits, hashed, skipped }`——
/// `cacheHits` 高 = 增量生效（零文件读取）。
pub async fn library_refresh(
    State(state): State<ServerState>,
    body: Option<JsonBody<Value>>,
) -> Response {
    let dir = body
        .as_ref()
        .and_then(|b| b.0.get("dir"))
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .or_else(|| state.library_dir.clone());
    let Some(dir) = dir else {
        return err(
            StatusCode::BAD_REQUEST,
            "MF-API-BAD-REQUEST",
            "缺 dir 且服务端未配置 MUSICFORGE_LIBRARY_DIR",
        );
    };
    // P9 路径域（配置白名单时生效）
    if let Err(r) = ensure_allowed(&state, &dir) {
        return r;
    }
    let recursive = body
        .as_ref()
        .and_then(|b| b.0.get("recursive"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let max_depth = body
        .as_ref()
        .and_then(|b| b.0.get("max_depth"))
        .and_then(|v| v.as_u64())
        .map(|v| v.min(512) as usize)
        .unwrap_or(64);
    let options = ScanOptions {
        recursive,
        max_depth,
        ..ScanOptions::default()
    };
    let db_path = state.data_dir.join("library.db");
    let result = tokio::task::spawn_blocking(move || {
        let db = musicforge_core::db::Db::open(&db_path)?;
        let r = musicforge_core::scan::refresh_library(&db, &dir, &options)?;
        Ok::<_, musicforge_core::NcmError>(r)
    })
    .await;
    match result {
        Ok(Ok(r)) => ok(json!({
            "scannedFiles": r.scanned_files,
            "scannedDirs": r.scanned_dirs,
            "audio": r.audio,
            "cacheHits": r.cache_hits,
            "hashed": r.hashed,
            "skipped": r.skipped,
        })),
        Ok(Err(e)) => err_from(e),
        Err(e) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "MF-INTERNAL",
            format!("库刷新任务执行失败: {e}"),
        ),
    }
}

/// `GET /api/version`：版本与形态（生命周期/前端握手用）。
pub async fn version() -> Response {
    ok(json!({
        "name": "musicforge-server",
        "version": env!("CARGO_PKG_VERSION"),
        // AUD-8：随域开放同步（原文案 "p8.2.1" 已过时）
        "api_surface": "p8.2.6",
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
        // 2026-09-12：鉴权总开关状态（前端据此显示"鉴权已关闭"提示、并隐藏 token 输入框）
        "auth_enabled": !state.auth_disabled,
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
pub async fn convert(
    State(state): State<ServerState>,
    JsonBody(body): JsonBody<Value>,
) -> Response {
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
    let ekey: Option<String> = body.get("ekey").and_then(|v| v.as_str()).map(String::from);
    // output_dir 缺省 = 源父目录（与 GUI IPC 同语义）
    let out_dir = if output_dir.trim().is_empty() {
        std::path::Path::new(source)
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    } else {
        output_dir.to_string()
    };
    // P9 路径域（配置白名单时生效）
    if let Err(r) = ensure_allowed(&state, std::path::Path::new(source)) {
        return r;
    }
    if let Err(r) = ensure_allowed(&state, std::path::Path::new(&out_dir)) {
        return r;
    }
    // AUD-5（并发修复）：插件迁移为同步阻塞（起子进程 + 全文件变换，可达秒级）
    // ——移出 tokio worker 线程。
    let plugin = plugin.to_string();
    let source = source.to_string();
    let convert_result = tokio::task::spawn_blocking(move || {
        musicforge_cli::format_migrate(&plugin, &source, &out_dir, None, ekey.as_deref())
    })
    .await;
    match convert_result {
        Ok(Ok(output_path)) => ok(json!({ "outputPath": output_path })),
        Ok(Err(e)) => err(StatusCode::BAD_REQUEST, e.mf_code(), format!("{e}")),
        Err(e) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "MF-INTERNAL",
            format!("迁移任务执行失败: {e}"),
        ),
    }
}

/// 批处理命名模板缺省（与前端 settings.template 兜底一致）。
const DEFAULT_BATCH_TEMPLATE: &str = "{title} - {artist}";

/// `POST /api/batch`：内置 NCM 批处理转换（P8.2.7——与桌面 `startBatch` 同源引擎）。
///
/// 请求 `{ inputs: [{path, root?}], out_dir?, template?, skip_existing?, recursive?, jobs?, dry_run? }`
/// （与前端 `BatchArgs` 同语义，snake_case）。同步执行（spawn_blocking）→
/// 返回与桌面事件流终态同形状的 summary（planned/ok/skipped/cancelled/failed/
/// durationMs/isCancelled/results[]）。
///
/// - 仅处理**内置转换域**（.ncm）——插件格式（kwm/qmc）走 `/api/convert`（前端分派）；
/// - `jobs` 硬约束 ≤10（Q7 姿态）；`cancel: None`——HTTP 同步形态无取消（连接断开
///   不中断任务，语义与 CLI 一次性执行一致）。
pub async fn batch(State(state): State<ServerState>, JsonBody(body): JsonBody<Value>) -> Response {
    let Some(inputs) = body.get("inputs").and_then(|v| v.as_array()) else {
        return err(
            StatusCode::BAD_REQUEST,
            "MF-API-BAD-REQUEST",
            "缺 inputs 数组",
        );
    };
    let expanded: Vec<(PathBuf, Option<PathBuf>)> = inputs
        .iter()
        .filter_map(|v| {
            let p = v.get("path")?.as_str()?;
            Some((
                PathBuf::from(p),
                v.get("root").and_then(|r| r.as_str()).map(PathBuf::from),
            ))
        })
        .collect();
    if expanded.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            "MF-API-BAD-REQUEST",
            "inputs 内无有效 path",
        );
    }
    // P9 路径域（配置白名单时生效）
    for (p, _) in &expanded {
        if let Err(r) = ensure_allowed(&state, p) {
            return r;
        }
    }
    let out_dir = body
        .get("out_dir")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let template = body
        .get("template")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_BATCH_TEMPLATE)
        .to_string();
    let skip_existing = body
        .get("skip_existing")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let recursive = body
        .get("recursive")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    // Q7：有界并发硬约束 10
    let jobs = body
        .get("jobs")
        .and_then(|v| v.as_u64())
        .map(|v| v.clamp(1, 10) as usize)
        .unwrap_or(4);
    let dry_run = body
        .get("dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let task_id = musicforge_cli::manifest::new_task_id();
    let manifest_path = musicforge_cli::manifest::default_manifest_path(
        // 无自定义输出目录时落服务端数据目录（fnOS：package 用户 home 之上的
        // 可控位置，而非隐式 local_config_dir）
        Some(out_dir.clone().unwrap_or_else(|| state.data_dir.clone())).as_deref(),
        &task_id,
    );
    let cfg = musicforge_cli::BatchConfig {
        // expanded 入口（G3）：inputs 留空，root 随行
        inputs: Vec::new(),
        out_dir,
        recursive,
        skip_existing,
        jobs,
        template,
        cancel: None,
        dry_run,
        manifest: Some(manifest_path),
    };
    // AUD-5 约束遵守：批处理为同步阻塞 IO——spawn_blocking
    let batch_result = tokio::task::spawn_blocking(move || {
        musicforge_cli::run_with_progress_expanded(expanded, cfg, |_| {})
    })
    .await;
    match batch_result {
        Ok(s) => ok(json!({
            "planned": s.planned,
            "ok": s.ok,
            "skipped": s.skipped,
            "cancelled": s.cancelled,
            "failed": s.failed,
            "durationMs": s.duration_ms,
            "isCancelled": s.is_cancelled(),
            "results": s.results.iter().map(|r| json!({
                "source": r.source.to_string_lossy(),
                "status": match r.status {
                    musicforge_cli::Status::Ok => "ok",
                    musicforge_cli::Status::Skipped => "skipped",
                    musicforge_cli::Status::Cancelled => "cancelled",
                    musicforge_cli::Status::Failed => "failed",
                },
                "output": r.output.as_ref().map(|p| p.to_string_lossy()),
                "reason": r.reason,
            })).collect::<Vec<_>>(),
        })),
        Err(e) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "MF-INTERNAL",
            format!("批处理任务执行失败: {e}"),
        ),
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

/// `POST /api/organize/plan`：整理计划预览（**只读**，绝不移动）。
///
/// 请求 `{ "dir", "template"?, "target_root"?, "strategy"? }`。
pub async fn organize_plan(
    State(state): State<ServerState>,
    JsonBody(body): JsonBody<Value>,
) -> Response {
    let r = match parse_organize_req(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    // P9 路径域（配置白名单时生效）
    if let Err(resp) = ensure_allowed(&state, &r.dir) {
        return resp;
    }
    if let Err(resp) = ensure_allowed(&state, &r.target_root) {
        return resp;
    }
    // AUD-5：plan 含全库扫描（同步阻塞）——spawn_blocking
    let plan_result = tokio::task::spawn_blocking(move || {
        let opts = OrganizeOptions {
            template: &r.template,
            target_root: &r.target_root,
            conflict: r.conflict,
        };
        plan_organize(&r.dir, &opts)
    })
    .await;
    match plan_result {
        Ok(Ok(plan)) => {
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
        Ok(Err(e)) => err_from(e),
        Err(e) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "MF-INTERNAL",
            format!("整理规划任务执行失败: {e}"),
        ),
    }
}

/// `POST /api/organize/apply`：执行整理计划（**破坏类**——safety 分级强制）。
///
/// - `confirm !== true` → `403 MF-OP-NEEDS-YES`（P2 安全分级语义的 API 版：
///   必须先 plan 预览、再显式 confirm 才落盘）；
/// - 绝不覆盖（apply 间隙目标被外部创建 → 该项失败）；
/// - 回滚清单落 target_root/.musicforge/。
pub async fn organize_apply(
    State(state): State<ServerState>,
    JsonBody(body): JsonBody<Value>,
) -> Response {
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
    // P9 路径域（配置白名单时生效）
    if let Err(resp) = ensure_allowed(&state, &r.dir) {
        return resp;
    }
    if let Err(resp) = ensure_allowed(&state, &r.target_root) {
        return resp;
    }
    // P1-2：r 稍后被 move 进 spawn_blocking——先留一份用于审计日志
    let dir_display = r.dir.display().to_string();
    let task_id = format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        std::process::id()
    );
    // AUD-5：plan + apply 全程同步阻塞——spawn_blocking
    let apply_result = tokio::task::spawn_blocking(move || {
        let opts = OrganizeOptions {
            template: &r.template,
            target_root: &r.target_root,
            conflict: r.conflict,
        };
        let plan = plan_organize(&r.dir, &opts)?;
        apply_organize_plan(&plan, &task_id)
    })
    .await;
    match apply_result {
        Ok(Ok(outcome)) => {
            // P1-2：破坏类操作审计留痕（谁/对哪个目录/结果/回滚清单位置）
            tracing::info!(
                op = "organize_apply",
                dir = %dir_display,
                moved = outcome.moved,
                skipped = outcome.skipped,
                failed = outcome.failed,
                rollback = ?outcome.rollback_manifest.as_ref().map(|p| p.display().to_string()),
                "destructive op applied"
            );
            ok(json!({
            "moved": outcome.moved,
            "skipped": outcome.skipped,
            "failed": outcome.failed,
            "rollback_manifest": outcome.rollback_manifest.map(|p| p.display().to_string()),
        }))
        }
        Ok(Err(e)) => err_from(e),
        Err(e) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "MF-INTERNAL",
            format!("整理执行任务失败: {e}"),
        ),
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
pub async fn clean_plan(
    State(state): State<ServerState>,
    JsonBody(body): JsonBody<Value>,
) -> Response {
    let Some(dir) = body.get("dir").and_then(|v| v.as_str()) else {
        return err(StatusCode::BAD_REQUEST, "MF-API-BAD-REQUEST", "缺 dir");
    };
    let dir = PathBuf::from(dir);
    // P9 路径域（配置白名单时生效）
    if let Err(resp) = ensure_allowed(&state, &dir) {
        return resp;
    }
    // AUD-5：全库扫描同步阻塞——spawn_blocking
    let plan_result = tokio::task::spawn_blocking(move || {
        let report = scan_library(&dir, &ScanOptions::default())?;
        let trash_root = dir.join(".musicforge").join("trash");
        let rules = enabled_rules(&body);
        Ok::<_, musicforge_core::NcmError>(build_clean_plan(&report, &rules, &trash_root, &dir))
    })
    .await;
    let plan = match plan_result {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return err_from(e),
        Err(e) => {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "MF-INTERNAL",
                format!("清洗规划任务执行失败: {e}"),
            )
        }
    };
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
pub async fn clean_apply(
    State(state): State<ServerState>,
    JsonBody(body): JsonBody<Value>,
) -> Response {
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
    // P9 路径域（配置白名单时生效）
    if let Err(resp) = ensure_allowed(&state, &dir) {
        return resp;
    }
    // P1-2：dir 稍后被 move 进 spawn_blocking——先留一份用于审计日志
    let dir_display = dir.display().to_string();
    let task_id = format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        std::process::id()
    );
    // AUD-5：scan + apply 全程同步阻塞——spawn_blocking
    let apply_result = tokio::task::spawn_blocking(move || {
        let report = scan_library(&dir, &ScanOptions::default())?;
        let trash_root = dir.join(".musicforge").join("trash");
        let rules = enabled_rules(&body);
        let plan = build_clean_plan(&report, &rules, &trash_root, &dir);
        apply_clean_plan(&plan, &task_id)
    })
    .await;
    match apply_result {
        Ok(Ok(outcome)) => {
            // P1-2：破坏类操作审计留痕（清洗：移入回收站数量 + 回滚清单）
            tracing::info!(
                op = "clean_apply",
                dir = %dir_display,
                moved = outcome.moved,
                dirs_removed = outcome.dirs_removed,
                rollback = ?outcome.rollback_manifest.as_ref().map(|p| p.display().to_string()),
                "destructive op applied"
            );
            ok(json!({
            "moved": outcome.moved,
            "dirs_removed": outcome.dirs_removed,
            "rollback_manifest": outcome.rollback_manifest.map(|p| p.display().to_string()),
        }))
        }
        Ok(Err(e)) => err_from(e),
        Err(e) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "MF-INTERNAL",
            format!("清洗执行任务失败: {e}"),
        ),
    }
}

/// `POST /api/trash/restore`：从回滚清单整体还原（**破坏类**——confirm 强制）。
///
/// 请求 `{ "manifest": "<rollback.jsonl 路径>", "confirm" }`。
pub async fn trash_restore(
    State(state): State<ServerState>,
    JsonBody(body): JsonBody<Value>,
) -> Response {
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
    // AUD-6（安全修复）：此前接受任意路径的 manifest——持 token 者可借 restore
    // 读取/「还原」任意 jsonl 文件描述的任意路径（越权面）。最小约束：manifest
    // 必须位于 `.musicforge` 回收站体系内（*.jsonl）——合法流（clean/organize
    // 的回滚清单）全部满足，其余一律拒绝。
    let manifest_path = std::path::Path::new(manifest);
    // P9 路径域（配置白名单时生效）
    if let Err(resp) = ensure_allowed(&state, manifest_path) {
        return resp;
    }
    let in_trash = manifest_path
        .components()
        .any(|c| c.as_os_str() == ".musicforge")
        && manifest_path.extension().and_then(|e| e.to_str()) == Some("jsonl");
    if !in_trash {
        return err(
            StatusCode::FORBIDDEN,
            "MF-TRASH-MANIFEST-INVALID",
            "manifest 必须是 .musicforge 回收站/回滚目录内的 *.jsonl 清单",
        );
    }
    let manifest = manifest.to_string(); // owned（借用不得跨 await）
                                         // AUD-5：还原含批量文件搬移——spawn_blocking
    let restore_result =
        tokio::task::spawn_blocking(move || restore_from_trash(std::path::Path::new(&manifest)))
            .await;
    match restore_result {
        Ok(Ok(n)) => ok(json!({ "restored": n })),
        Ok(Err(e)) => err_from(e),
        Err(e) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "MF-INTERNAL",
            format!("还原任务执行失败: {e}"),
        ),
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
            auth_guard: std::sync::Arc::new(crate::AuthGuard::new()),
            ui_dir: PathBuf::from("ui"),
            data_dir: std::env::temp_dir().join(format!("mf-api-test-{}", std::process::id())),
            library_dir: lib,
            allowed_roots: Vec::new(),
            // 端点业务测试不带签名 → legacy（M2 签名路径见 lib.rs 专项测试）
            auth_require_sign: false,
            // 鉴权保持开启（安全默认）
            auth_disabled: false,
            nonce_seen: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// P9 路径域：配置白名单的 state（用于越权拒绝测试）
    fn state_with_roots(roots: Vec<PathBuf>) -> ServerState {
        let mut s = state_with(None);
        s.allowed_roots = roots;
        s
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

    /// AUD-6 回归：restore 的 manifest 必须位于 .musicforge 体系内——
    /// 任意路径（如 /etc/passwd 旁伪造的 jsonl）必须 403 拒绝。
    #[tokio::test]
    async fn trash_restore_rejects_manifest_outside_musicforge() {
        let app = build_router(state_with(None));
        // 不在 .musicforge 下 → 403（即使扩展名是 .jsonl）
        let res = app
            .clone()
            .oneshot(req(
                "POST",
                "/api/trash/restore",
                Some(json!({"manifest": "/tmp/evil/rollback.jsonl", "confirm": true}).to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        let v = body_json(res).await;
        assert_eq!(v["code"], "MF-TRASH-MANIFEST-INVALID");
        // 在 .musicforge 下但非 .jsonl → 403
        let res = app
            .clone()
            .oneshot(req(
                "POST",
                "/api/trash/restore",
                Some(
                    json!({"manifest": "/data/.musicforge/trash/t1/rollback.txt", "confirm": true})
                        .to_string(),
                ),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    /// AUD-6 正路回归：合法 trash 清单路径通过校验（到达 core 层后因文件不存在
    /// 报 NcmError 稳定码——而非校验层 403）。
    #[tokio::test]
    async fn trash_restore_accepts_in_trash_manifest_path() {
        let app = build_router(state_with(None));
        let res = app
            .clone()
            .oneshot(req(
                "POST",
                "/api/trash/restore",
                Some(
                    json!({"manifest": "/data/lib/.musicforge/trash/task-1/rollback.jsonl", "confirm": true})
                        .to_string(),
                ),
            ))
            .await
            .unwrap();
        assert_ne!(
            res.status(),
            StatusCode::FORBIDDEN,
            "合法路径不得被校验层拦截"
        );
    }

    // ---------------------------------------------------------------- batch --

    /// P8.2.7 fixtures（真实 ncm 文件，与 CLI 测试同源）。
    fn ncm_fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../musicforge-core/tests/fixtures")
    }

    /// 递归收集文件（dry-run 断言辅助）。
    fn walk_files(dir: &std::path::Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.filter_map(|e| e.ok()) {
                let p = e.path();
                if p.is_dir() {
                    out.extend(walk_files(&p));
                } else {
                    out.push(p);
                }
            }
        }
        out
    }

    /// P8.2.7 回归：/api/batch 真实 ncm roundtrip——引擎桥接全链路。
    #[tokio::test]
    async fn batch_converts_real_ncm_fixture() {
        let src = ncm_fixtures();
        let ncm = std::fs::read_dir(&src)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().map(|e| e == "ncm").unwrap_or(false))
            .expect("fixtures 目录应含 .ncm 文件");
        let out = std::env::temp_dir().join(format!("mf-batch-{}", std::process::id()));
        let app = build_router(state_with(None));
        let res = app
            .clone()
            .oneshot(req(
                "POST",
                "/api/batch",
                Some(
                    json!({
                        "inputs": [{"path": ncm.display().to_string(), "root": null}],
                        "out_dir": out.display().to_string(),
                        "jobs": 2,
                    })
                    .to_string(),
                ),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let v = body_json(res).await;
        assert_eq!(v["ok"], true);
        assert_eq!(v["data"]["ok"], 1, "单文件转换应成功: {v}");
        assert_eq!(v["data"]["failed"], 0);
        // 产物落盘
        let output = v["data"]["results"][0]["output"]
            .as_str()
            .expect("成功项应有 output 路径");
        assert!(
            std::path::Path::new(output).exists(),
            "产物应存在: {output}"
        );
        std::fs::remove_dir_all(&out).ok();
    }

    /// P8.2.7 回归：dry_run=true 绝不写文件（AUD-1 语义的服务端承载）。
    #[tokio::test]
    async fn batch_dry_run_writes_nothing() {
        let src = ncm_fixtures();
        let ncm = std::fs::read_dir(&src)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().map(|e| e == "ncm").unwrap_or(false))
            .unwrap();
        let out = std::env::temp_dir().join(format!("mf-batch-dry-{}", std::process::id()));
        std::fs::create_dir_all(&out).unwrap();
        let app = build_router(state_with(None));
        let res = app
            .clone()
            .oneshot(req(
                "POST",
                "/api/batch",
                Some(
                    json!({
                        "inputs": [{"path": ncm.display().to_string(), "root": null}],
                        "out_dir": out.display().to_string(),
                        "dry_run": true,
                    })
                    .to_string(),
                ),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let v = body_json(res).await;
        assert_eq!(v["data"]["planned"], 1, "dry-run 报 planned 计数");
        // dry-run 允许落任务审计清单（.musicforge/manifests/*.jsonl，与桌面引擎
        // 同源行为）；断言核心：无任何**转换产物**落盘
        let produced: Vec<_> = walk_files(&out)
            .into_iter()
            .filter(|p| !p.components().any(|c| c.as_os_str() == ".musicforge"))
            .collect();
        assert!(
            produced.is_empty(),
            "dry-run 绝不落转换产物: {:?}",
            produced
        );
        std::fs::remove_dir_all(&out).ok();
    }

    /// P8 LibraryRefresher：/api/library/refresh 增量重扫（二次 = 全命中零重算）。
    #[tokio::test]
    async fn library_refresh_reports_incremental_stats() {
        let dir = std::env::temp_dir().join(format!("mf-refresh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.flac"), b"fLaC-stub-a").unwrap();
        let app = build_router(state_with(None));
        let body = json!({ "dir": dir.display().to_string() }).to_string();
        // 首轮：全部重算
        let first = app
            .clone()
            .oneshot(req("POST", "/api/library/refresh", Some(body.clone())))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        // 二次：命中缓存，零重算
        let res = app
            .clone()
            .oneshot(req("POST", "/api/library/refresh", Some(body)))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let v = body_json(res).await;
        assert_eq!(v["ok"], true);
        assert_eq!(v["data"]["audio"], 1, "1 个音频文件");
        assert_eq!(v["data"]["cacheHits"], 1, "二次刷新命中缓存");
        assert_eq!(v["data"]["hashed"], 0, "二次零重算");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// P9 路径域：配置 `allowed_roots` 后，白名单外路径一律 403 MF-PATH-NOT-ALLOWED。
    #[tokio::test]
    async fn path_domain_rejects_outside_roots_when_configured() {
        let allowed = std::env::temp_dir().join(format!("mf-allowed-{}", std::process::id()));
        std::fs::create_dir_all(&allowed).unwrap();
        let outside = std::env::temp_dir().join(format!("mf-outside-{}", std::process::id()));
        std::fs::create_dir_all(&outside).unwrap();
        let app = build_router(state_with_roots(vec![allowed.clone()]));
        for (method, uri, body) in [
            (
                "POST",
                "/api/scan",
                json!({"dir": outside.display().to_string()}),
            ),
            (
                "POST",
                "/api/organize/plan",
                json!({"dir": outside.display().to_string()}),
            ),
            (
                "POST",
                "/api/clean/plan",
                json!({"dir": outside.display().to_string()}),
            ),
        ] {
            let res = app
                .clone()
                .oneshot(req(method, uri, Some(body.to_string())))
                .await
                .unwrap();
            assert_eq!(
                res.status(),
                StatusCode::FORBIDDEN,
                "{uri} 白名单外必须 403"
            );
            let v = body_json(res).await;
            assert_eq!(v["code"], "MF-PATH-NOT-ALLOWED");
        }
        // 白名单内放行（空目录扫描成功）
        let res = app
            .clone()
            .oneshot(req(
                "POST",
                "/api/scan",
                Some(json!({"dir": allowed.display().to_string()}).to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        std::fs::remove_dir_all(&allowed).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    /// P9：未配置白名单 = 不约束（向后兼容——既有部署行为不变）。
    #[tokio::test]
    async fn path_domain_is_off_by_default() {
        let dir = std::env::temp_dir().join(format!("mf-noroots-{}", std::process::id()));
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
        assert_eq!(res.status(), StatusCode::OK, "未配置白名单时必须放行");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// P8.2.7：缺 inputs / 空 path 显式报错。
    #[tokio::test]
    async fn batch_rejects_missing_or_empty_inputs() {
        let app = build_router(state_with(None));
        let res = app
            .clone()
            .oneshot(req("POST", "/api/batch", Some(json!({}).to_string())))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let res = app
            .oneshot(req(
                "POST",
                "/api/batch",
                Some(json!({"inputs": [{"root": null}]}).to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let v = body_json(res).await;
        assert_eq!(v["code"], "MF-API-BAD-REQUEST");
    }
}
