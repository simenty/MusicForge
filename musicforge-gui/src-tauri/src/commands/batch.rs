// 批处理与导出 IPC 命令（start_batch/cancel_batch/失败清单导出）
// 拆分自 main.rs（P9 可维护性治理）——逐字迁移，行为零变更。
use crate::*;

/// 启动批处理：立即返回；进度经 `batch-file`（逐文件）与 `batch-done`（汇总）事件推送。
/// 已有任务运行时返回 Err。
#[tauri::command]
pub fn start_batch(
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

    // v0.2.0：manifest 落在 out_dir 的约定目录；无自定义输出目录时落本地配置目录
    let task_id = musicforge_cli::manifest::new_task_id();
    let manifest_path = musicforge_cli::manifest::default_manifest_path(
        match args.out_dir.as_deref() {
            Some(d) if !d.trim().is_empty() => Some(Path::new(d)),
            _ => None,
        },
        &task_id,
    );
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
        dry_run: args.dry_run,
        manifest: Some(manifest_path),
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
            "planned": summary.planned,
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
pub fn cancel_batch(state: tauri::State<'_, AppState>) -> bool {
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
pub struct FailureRow {
    pub source: String,
    pub status: String,
    pub reason: Option<String>,
}

/// 原生多选文件对话框 → 已选 .ncm 路径列表（用户取消 → 空列表）。
/// 对话框只做挑选，**过滤仍由 `collect_files` 统一负责**，保证拖拽与点选两条入口行为一致。
#[tauri::command]
pub async fn select_ncm_files(app: AppHandle, start_dir: Option<String>) -> Vec<String> {
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
pub async fn select_directory(
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
pub async fn save_failures(
    app: AppHandle,
    rows: Vec<FailureRow>,
) -> Result<Option<String>, String> {
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
