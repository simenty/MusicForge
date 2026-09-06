//! MusicForge GUI（Tauri 2）：把 musicforge-cli 批处理桥接到前端。
//!
//! 命令面：`collect_files`（拖拽路径 → 过滤 .ncm）、`start_batch`（启动批处理，进度经
//! `batch-file`/`batch-done` 事件推送）、`cancel_batch`（协作式取消）、
//! `select_ncm_files` / `select_output_dir`（原生选择对话框）、`save_failures`（失败清单导出）。
//! 架构：GUI 与 musicforge-core 同语言同进程（方案书 §6：零 FFI）。
//!
//! 安全面：`tauri-plugin-dialog` 只注册为 Rust 侧插件，**不向 JS 暴露 API**，
//! 前端只能经上述白名单命令打开对话框，ACL 攻击面最小。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

use musicforge_cli::CancelToken;

/// 运行状态：同一时刻至多一个批处理任务（running 标志 + 取消令牌）
#[derive(Default)]
struct AppState {
    running: AtomicBool,
    cancel: Mutex<Option<CancelToken>>,
}

/// RAII：批处理线程无论**正常结束**还是**提前终止**（panic unwind / 提前 return），
/// 离开作用域时都必须把 `running` 复位。
///
/// 修复前（QA 第二轮 G2）复位逻辑写在线程体最后一行：只要中间任何一步 panic
/// （worker 经 `thread::scope` 传播、序列化 panic…），那行就永远不会执行 ——
/// `running` 永久为 true，`batch-done` 也永不发射，前端永远停在「转换中」且
/// 「开始转换」按钮永久失效，只能重启应用。
///
/// ⚠ 已知边界：release 为 `panic = "abort"`，进程会直接终止，`Drop` 同样不会执行。
/// 因此本 guard 是纵深防御，真正的根因防线仍是「零 panic」（硬约束 1）。
struct RunningGuard {
    app: AppHandle,
}

impl Drop for RunningGuard {
    fn drop(&mut self) {
        if let Some(state) = self.app.try_state::<AppState>() {
            state.running.store(false, Ordering::SeqCst);
        }
    }
}

/// 从被污染的 mutex 中恢复数据。
///
/// `AppState::cancel` 的临界区只做一次 `Option` 赋值/读取，锁内不可能 panic，
/// 因此中毒只意味着「别的线程 panic 过」，数据本身依然完好。
/// 统一走 `into_inner()` 恢复，`start_batch`/`cancel_batch` 便不会因中毒而
/// 静默失效（尤其不会让 `running` 卡在 true 上）。
fn lock_cancel(m: &Mutex<Option<CancelToken>>) -> std::sync::MutexGuard<'_, Option<CancelToken>> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchArgs {
    inputs: Vec<InputPair>,
    out_dir: Option<String>,
    template: String,
    skip_existing: bool,
    recursive: bool,
    jobs: usize,
}

/// 模板预览：给定模板 + 示例元数据 → 返回渲染后的文件名
#[tauri::command]
fn preview_template(template: String) -> Vec<String> {
    let meta = musicforge_core::Metadata {
        name: Some("贝贝".to_string()),
        artist: Some("李荣浩".to_string()),
        album: Some("耳朵".to_string()),
        format: Some("flac".to_string()),
        track: Some(1),
        bitrate: Some(311454),
        duration: Some(4000),
        album_pic_url: None,
    };
    // 渲染两行：完整元数据 / 无元数据回退（QA 第二轮 N1：注释曾误写「三行」）
    vec![
        musicforge_core::template::render_filename(&template, Some(&meta), "song.ncm"),
        musicforge_core::template::render_filename(&template, None, "unknown.ncm"),
    ]
}

/// 展开后的输入项（G3 修复）：root = 目录输入的根（散文件为 null），
/// 随路径一起穿透 IPC，保证自定义输出目录下源目录树镜像与 CLI 一致。
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct InputPair {
    path: String,
    root: Option<String>,
}

/// 拖拽/选择的路径 → 过滤出 .ncm 文件（大小写不敏感；目录按 recursive 递归）。
/// 返回 (path, root) 对——root 必须随行返回，前端 start_batch 原样回传。
#[tauri::command]
fn collect_files(inputs: Vec<String>, recursive: bool) -> Vec<InputPair> {
    let paths: Vec<PathBuf> = inputs.iter().map(PathBuf::from).collect();
    musicforge_cli::collect_inputs(&paths, recursive)
        .into_iter()
        .map(|(p, root)| InputPair {
            path: p.to_string_lossy().into_owned(),
            root: root.map(|r| r.to_string_lossy().into_owned()),
        })
        .collect()
}

/// 启动批处理：立即返回；进度经 `batch-file`（逐文件）与 `batch-done`（汇总）事件推送。
/// 已有任务运行时返回 Err。
#[tauri::command]
fn start_batch(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    args: BatchArgs,
) -> Result<(), String> {
    // QA 第二轮 G1：**先**完成所有可失败步骤，再置 running。
    // 修复前 `running.swap(true)` 在最前面，一旦随后的 `cancel.lock()` 返回 Err
    // 就会带着 `running == true` 提前返回 —— 此后「开始转换」永远返回
    // 「已有转换任务正在运行」，只能重启应用。顺序本身即是修复。
    let token = CancelToken::new();
    {
        let mut cancel = lock_cancel(&state.cancel);
        if state.running.swap(true, Ordering::SeqCst) {
            return Err("已有转换任务正在运行".to_string());
        }
        *cancel = Some(token.clone());
    }

    let cfg = musicforge_cli::BatchConfig {
        // G3：输入已在导入阶段展开且保留 root——走 expanded 入口，root 随行，
        // 自定义输出目录时源目录树镜像与 CLI 完全一致
        inputs: Vec::new(),
        out_dir: args.out_dir.map(PathBuf::from),
        recursive: args.recursive,
        skip_existing: args.skip_existing,
        jobs: args.jobs,
        template: args.template,
        cancel: Some(token),
        dry_run: false,
        manifest: None,
    };
    let expanded: Vec<(PathBuf, Option<PathBuf>)> = args
        .inputs
        .into_iter()
        .map(|p| (PathBuf::from(&p.path), p.root.map(PathBuf::from)))
        .collect();

    let app_for_thread = app.clone();
    std::thread::spawn(move || {
        // 复位交给 Drop（见 RunningGuard 注释）；线程体内不再手动置 false
        let _guard = RunningGuard {
            app: app_for_thread.clone(),
        };
        let summary = musicforge_cli::run_with_progress_expanded(expanded, cfg, |r| {
            let payload = serde_json::json!({
                "source": r.source.to_string_lossy(),
                "status": match r.status {
                    musicforge_cli::Status::Ok => "ok",
                    musicforge_cli::Status::Skipped => "skipped",
                    musicforge_cli::Status::Cancelled => "cancelled",
                    musicforge_cli::Status::Failed => "failed",
                },
                "output": r.output.as_ref().map(|p| p.to_string_lossy()),
                "reason": r.reason,
                "tagsWritten": r.tags_written,
            });
            let _ = app_for_thread.emit("batch-file", payload);
        });

        let payload = serde_json::json!({
            "ok": summary.ok,
            "skipped": summary.skipped,
            "cancelled": summary.cancelled,
            "failed": summary.failed,
            "durationMs": summary.duration_ms,
            "isCancelled": summary.is_cancelled(),
            "results": summary
                .results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "source": r.source.to_string_lossy(),
                        "status": match r.status {
                            musicforge_cli::Status::Ok => "ok",
                            musicforge_cli::Status::Skipped => "skipped",
                            musicforge_cli::Status::Cancelled => "cancelled",
                            musicforge_cli::Status::Failed => "failed",
                        },
                        "output": r.output.as_ref().map(|p| p.to_string_lossy()),
                        "reason": r.reason,
                    })
                })
                .collect::<Vec<_>>(),
        });
        let _ = app_for_thread.emit("batch-done", payload);
    });
    Ok(())
}

/// 协作式取消：未开工的文件标记为 Cancelled（结果计数完整），已开工的跑完
#[tauri::command]
fn cancel_batch(state: tauri::State<'_, AppState>) -> bool {
    match lock_cancel(&state.cancel).as_ref() {
        Some(t) => {
            t.cancel();
            true
        }
        None => false,
    }
}

/// 前端传来的失败行（与 `FileResult` 的 JSON 形状一致）
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FailureRow {
    source: String,
    status: String,
    reason: Option<String>,
}

/// 原生多选文件对话框 → 已选 .ncm 路径列表（用户取消 → 空列表）。
/// 对话框只做挑选，**过滤仍由 `collect_files` 统一负责**，保证拖拽与点选两条入口行为一致。
#[tauri::command]
async fn select_ncm_files(app: AppHandle, start_dir: Option<String>) -> Vec<String> {
    let mut d = app
        .dialog()
        .file()
        .add_filter("NCM 音频文件", &["ncm"])
        .set_title("选择 .ncm 文件（可多选）");
    if let Some(dir) = start_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        d = d.set_directory(dir);
    }
    match d.blocking_pick_files() {
        Some(paths) => paths
            .into_iter()
            .filter_map(|p| p.into_path().ok())
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        None => Vec::new(),
    }
}

/// 原生目录选择对话框（输入目录与输出目录共用，靠 `title` 区分）。用户取消 → null
#[tauri::command]
async fn select_directory(
    app: AppHandle,
    start_dir: Option<String>,
    title: Option<String>,
) -> Option<String> {
    let title = title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("选择目录");
    let mut d = app.dialog().file().set_title(title);
    if let Some(dir) = start_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        d = d.set_directory(dir);
    }
    d.blocking_pick_folder()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
}

/// 导出失败清单：原生保存对话框 → 写 CSV。
/// 复用 `musicforge_cli::BatchSummary::export_failures_csv`，保证与 CLI `--export-failures` 格式完全一致。
/// 返回写入路径；无失败项或用户取消 → null。
#[tauri::command]
async fn save_failures(app: AppHandle, rows: Vec<FailureRow>) -> Result<Option<String>, String> {
    let failed: Vec<FailureRow> = rows.into_iter().filter(|r| r.status == "failed").collect();
    if failed.is_empty() {
        return Ok(None);
    }

    let chosen = app
        .dialog()
        .file()
        .add_filter("CSV 表格", &["csv"])
        .set_file_name("failures.csv")
        .set_title("导出失败清单")
        .blocking_save_file();

    let path = match chosen {
        Some(p) => p.into_path().map_err(|e| format!("保存路径无效：{e}"))?,
        None => return Ok(None), // 用户取消
    };

    // 与 CLI 一致：父目录不存在则自动创建（此前报裸 os error 3）
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("无法创建目录 {}：{e}", parent.display()))?;
        }
    }

    let results: Vec<musicforge_cli::FileResult> = failed
        .iter()
        .map(|r| musicforge_cli::FileResult {
            source: PathBuf::from(&r.source),
            status: musicforge_cli::Status::Failed,
            output: None,
            reason: r.reason.clone(),
            tags_written: 0,
        })
        .collect();

    let summary = musicforge_cli::BatchSummary {
        planned: 0,
        failed: results.len(),
        results,
        ok: 0,
        skipped: 0,
        cancelled: 0,
        duration_ms: 0,
    };

    summary
        .export_failures_csv(&path)
        .map_err(|e| format!("写入失败清单出错：{e}"))?;

    Ok(Some(path.to_string_lossy().into_owned()))
}

fn main() {
    tauri::Builder::default()
        // 仅 Rust 侧注册：插件的 JS API 不注入，前端无法自行唤起对话框
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            collect_files,
            start_batch,
            cancel_batch,
            preview_template,
            select_ncm_files,
            select_directory,
            save_failures
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ============ P1a 保护网：GUI ↔ 前端 IPC 契约测试 ============
//
// 两类覆盖：
// 1. **可纯调用的命令**（无 AppHandle/State 依赖）：真实行为断言。
// 2. **需要 AppHandle/State 的命令**：`cargo test` 下无法构造 Tauri 运行时，
//    用「包装函数 + 显式返回类型标注」做**编译期类型钉子** —— 返回类型一变，
//    本模块立即编译失败。字段级 schema 另由 InputPair/BatchArgs/FailureRow
//    的 serde 断言覆盖（前端真正依赖的是字段名与形状）。
#[cfg(test)]
mod tests {
    use super::*;

    /// preview_template：两行 = 完整元数据 / 无元数据回退。
    /// 前端靠它实时预览输出文件名，行数或内容变化即破坏契约。
    #[test]
    fn preview_template_returns_two_rows_with_rendered_names() {
        let rows = preview_template("{artist} - {title}".to_string());
        assert_eq!(rows.len(), 2, "必须返回两行：完整元数据 / 无元数据回退");
        assert_eq!(rows[0], "李荣浩 - 贝贝", "完整元数据行");
        assert_eq!(
            rows[1], "未知艺术家 - unknown.ncm",
            "无元数据回退行（title 回退到 fallback_stem）"
        );

        // 目录模板：GUI 预览与 CLI 渲染语义必须一致（含 track 零填充）
        let rows = preview_template("{artist}/{album}/{track:02d} {title}".to_string());
        assert_eq!(
            rows[0], "李荣浩/耳朵/01 贝贝",
            "GUI 预览必须与 CLI 渲染同语义"
        );
    }

    /// collect_files：目录输入按 recursive 过滤 .ncm，且 root 必须随行返回
    /// （G3 契约：root 丢失会让自定义输出目录下的源目录树不被镜像）。
    #[test]
    fn collect_files_filters_ncm_and_keeps_root() {
        let base = std::env::temp_dir().join(format!("mf-gui-contract-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let sub = base.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(base.join("a.ncm"), b"x").unwrap();
        std::fs::write(base.join("b.txt"), b"x").unwrap();
        std::fs::write(sub.join("c.ncm"), b"x").unwrap();

        // 非递归：只收 base 层，且不收 .txt
        let flat = collect_files(vec![base.to_string_lossy().into_owned()], false);
        assert_eq!(flat.len(), 1, "非递归不得进入 sub，且必须过滤非 .ncm");
        assert!(flat[0].path.ends_with("a.ncm"), "应收集 a.ncm");
        assert!(flat[0].root.is_some(), "目录输入必须带 root");

        // 递归：a.ncm + sub/c.ncm
        let rec = collect_files(vec![base.to_string_lossy().into_owned()], true);
        assert_eq!(rec.len(), 2, "递归应收集 sub 下的 .ncm");

        // 散文件输入：root 必须为 None（前端据此区分两种输入来源）
        let single = collect_files(
            vec![base.join("a.ncm").to_string_lossy().into_owned()],
            false,
        );
        assert_eq!(single.len(), 1);
        assert!(single[0].root.is_none(), "散文件输入 root 必须为 None");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// InputPair 序列化字段名集合：前端按 `{"path","root"}` 解构，
    /// 字段名一旦增删改即 IPC 断裂（且是静默断裂——前端拿到 undefined）。
    #[test]
    fn input_pair_serializes_with_stable_field_names() {
        let with_root = InputPair {
            path: "C:/a.ncm".into(),
            root: Some("C:/".into()),
        };
        let v: serde_json::Value = serde_json::to_value(&with_root).unwrap();
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|s| s.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["path", "root"], "InputPair 字段名集合发生变化");
        assert_eq!(v["path"], "C:/a.ncm");
        assert_eq!(v["root"], "C:/");

        let without_root = InputPair {
            path: "a.ncm".into(),
            root: None,
        };
        let v2: serde_json::Value = serde_json::to_value(&without_root).unwrap();
        assert!(
            v2["root"].is_null(),
            "root=None 必须序列化为 null（前端据此区分散文件/目录输入）"
        );
    }

    /// BatchArgs 反序列化：前端 `start_batch` 发送的 camelCase 载荷必须可解析。
    /// 同时钉住必填字段 —— 缺省会导致并发/跳过等语义静默漂移。
    #[test]
    fn batch_args_deserializes_camel_case_ipc_payload() {
        let ok = r#"{
            "inputs": [{"path": "C:/a.ncm", "root": null}],
            "outDir": "C:/out",
            "template": "{title}",
            "skipExisting": true,
            "recursive": false,
            "jobs": 4
        }"#;
        let args: BatchArgs = serde_json::from_str(ok).expect("camelCase 载荷必须可解析");
        assert_eq!(args.inputs.len(), 1);
        assert!(args.inputs[0].root.is_none());
        assert_eq!(args.out_dir.as_deref(), Some("C:/out"));
        assert_eq!(args.template, "{title}");
        assert!(args.skip_existing);
        assert!(!args.recursive);
        assert_eq!(args.jobs, 4);

        // 反例 1：把 skipExisting 写成 snake_case → 必填布尔字段缺席 → 必须失败。
        //（serde 默认忽略未知字段，故此断言证明的是「skipExisting 是必填 camelCase 键」）
        let snake = r#"{ "inputs": [], "outDir": null, "template": "t", "skip_existing": false, "recursive": false, "jobs": 1 }"#;
        assert!(
            serde_json::from_str::<BatchArgs>(snake).is_err(),
            "缺少 skipExisting 的载荷不应被接受"
        );

        // 反例 2：jobs 缺失 → 必须失败（并发语义不可静默取默认）
        let no_jobs = r#"{ "inputs": [], "outDir": null, "template": "t", "skipExisting": false, "recursive": false }"#;
        assert!(
            serde_json::from_str::<BatchArgs>(no_jobs).is_err(),
            "jobs 缺失的载荷不应被接受"
        );
    }

    /// FailureRow 反序列化：`save_failures` 入参契约（前端只传失败行）。
    #[test]
    fn failure_row_deserializes_camel_case() {
        let rows: Vec<FailureRow> = serde_json::from_str(
            r#"[
                {"source":"C:/a.ncm","status":"failed","reason":"NCM-X: bad"},
                {"source":"b.ncm","status":"ok","reason":null}
            ]"#,
        )
        .expect("FailureRow 载荷必须可解析");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].source, "C:/a.ncm");
        assert_eq!(rows[0].status, "failed");
        assert_eq!(rows[0].reason.as_deref(), Some("NCM-X: bad"));
        assert!(rows[1].reason.is_none(), "reason 为 null 必须可解析");
    }

    // ---- 编译期返回类型钉子（无 AppHandle/State，无法运行期断言） ----
    // 每个包装函数的返回类型标注即断言：底层命令返回类型一旦变化，本文件无法编译。

    #[allow(dead_code)]
    fn pin_cancel_batch(state: tauri::State<'_, AppState>) -> bool {
        cancel_batch(state)
    }

    #[allow(dead_code)]
    fn pin_start_batch(
        app: AppHandle,
        state: tauri::State<'_, AppState>,
        args: BatchArgs,
    ) -> Result<(), String> {
        // start_batch 本身是**同步**命令（立即返回，进度走事件），故此处不加 async/await
        start_batch(app, state, args)
    }

    #[allow(dead_code)]
    async fn pin_select_ncm_files(app: AppHandle, start_dir: Option<String>) -> Vec<String> {
        select_ncm_files(app, start_dir).await
    }

    #[allow(dead_code)]
    async fn pin_select_directory(
        app: AppHandle,
        start_dir: Option<String>,
        title: Option<String>,
    ) -> Option<String> {
        select_directory(app, start_dir, title).await
    }

    #[allow(dead_code)]
    async fn pin_save_failures(
        app: AppHandle,
        rows: Vec<FailureRow>,
    ) -> Result<Option<String>, String> {
        save_failures(app, rows).await
    }
}
