//! MusicForge GUI（Tauri 2）：把 musicforge-cli 批处理桥接到前端。
//!
//! 命令面：`collect_files`（拖拽路径 → 过滤 .ncm）、`start_batch`（启动批处理，进度经
//! `batch-file`/`batch-done` 事件推送）、`cancel_batch`（协作式取消）、
//! `plan_batch`（dry-run 计划预览）、`scan_library`（只读曲库扫描）、
//! `select_ncm_files` / `select_output_dir`（原生选择对话框）、`save_failures`（失败清单导出）。
//! 架构：GUI 与 musicforge-core 同语言同进程（方案书 §6：零 FFI）。
//!
//! 安全面：`tauri-plugin-dialog` 只注册为 Rust 侧插件，**不向 JS 暴露 API**，
//! 前端只能经上述白名单命令打开对话框，ACL 攻击面最小。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

use musicforge_cli::CancelToken;

/// 运行状态：同一时刻至多一个批处理任务（running 标志 + 取消令牌）
#[derive(Default)]
pub struct AppState {
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
pub struct RunningGuard {
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
pub struct BatchArgs {
    inputs: Vec<InputPair>,
    out_dir: Option<String>,
    template: String,
    skip_existing: bool,
    recursive: bool,
    jobs: usize,
    /// v0.2.0：仅规划不落盘（dry-run）
    #[serde(default)]
    dry_run: bool,
}

/// P2 播放引擎（symphonia + cpal；引擎线程持有 !Send 的 cpal::Stream）。
/// 模块名用 `audio` 而非 `player`：后者与 `commands::player` 的 glob
/// 重导出（`pub use commands::*`）在 crate root 形成遮蔽冲突。
mod audio;

/// P2 媒体键 / 媒体面板（仅 Windows / macOS——souvlaki 的 Linux 后端
/// 依赖 libdbus，musl 交叉编译不可行）。
#[cfg(any(target_os = "windows", target_os = "macos"))]
mod media_controls;

#[macro_use]
mod commands;
pub use commands::*;

fn main() {
    // P5 崩溃日志：panic hook 先于 abort 执行，留下最后线索（数据目录 logs/crash.log）
    init_panic_logging();
    // P5 文件关联：冷启动参数（双击 .ncm / 命令行传入）——暂存，前端 mount 后取
    let startup_files = ncm_paths_from_args(std::env::args().skip(1));
    tauri::Builder::default()
        // P5 单实例：必须是**第一个**注册的插件（插件文档硬要求）。
        // 第二次启动 = 聚焦既有窗口 + 把本次 argv 里的 .ncm 转发给前端。
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            show_main_window(app);
            let files = ncm_paths_from_args(argv.into_iter().skip(1));
            if !files.is_empty() {
                let _ = app.emit("open-files", files);
            }
        }))
        // 仅 Rust 侧注册：插件的 JS API 不注入，前端无法自行唤起对话框
        .plugin(tauri_plugin_dialog::init())
        // P5 更新：唯一网络行为（dependency-policy.md 显式例外）——JS API 不授权，
        // 前端只能走白名单命令 check_update / install_update / restart_app
        .plugin(tauri_plugin_updater::Builder::new().build())
        // P6.6 全局快捷键：Ctrl+Alt+Space = 播放/暂停（任何界面生效）。
        // 注册在被占用时失败——setup 中的 register 静默忽略，不影响启动。
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    use tauri_plugin_global_shortcut::{Code, Modifiers, ShortcutState};
                    if event.state() == ShortcutState::Pressed
                        && shortcut.matches(Modifiers::CONTROL | Modifiers::ALT, Code::Space)
                    {
                        let _ = app.state::<audio::PlayerHandle>().toggle();
                    }
                })
                .build(),
        )
        .manage(AppState::default())
        .manage(StartupFiles(Mutex::new(startup_files)))
        // P2：播放引擎句柄（引擎线程随进程存活；音频设备懒打开——首播时才建立输出流）
        .manage(audio::PlayerHandle::spawn(
            musicforge_core::db::default_db_path(),
        ))
        .setup(|app| {
            // P2 系统托盘（关闭窗口 = 隐藏到托盘；托盘菜单控制播放与退出）
            build_tray(app.handle())?;
            // P2 媒体键（仅 Windows / macOS；初始化失败静默降级）
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            {
                // hwnd 仅 Windows 需要（SMTC 绑定窗口）；以 usize 跨线程传递
                #[cfg(target_os = "windows")]
                let hwnd = app
                    .get_webview_window("main")
                    .and_then(|w| w.hwnd().ok())
                    .map(|h| h.0 as usize);
                #[cfg(not(target_os = "windows"))]
                let hwnd: Option<usize> = None;
                media_controls::spawn(app.state::<audio::PlayerHandle>().inner().clone(), hwnd);
            }
            // P6.6：全局快捷键注册（Ctrl+Alt+Space；被占用时静默降级）
            {
                use tauri_plugin_global_shortcut::{
                    Code, GlobalShortcutExt, Modifiers, Shortcut,
                };
                let sc = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Space);
                let _ = app.global_shortcut().register(sc);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            collect_files,
            plan_batch,
            scan_library,
            refresh_library,
            dedupe_scan,
            dedupe_apply,
            start_batch,
            cancel_batch,
            preview_template,
            select_ncm_files,
            select_directory,
            save_failures,
            plugins_status,
            plugins_set_enabled,
            plugins_acknowledge,
            select_migration_files,
            format_migrate,
            // P1 曲库体验：维度层读写 + 媒体源管理 + 索引构建
            library_stats,
            list_tracks,
            count_tracks,
            list_artists,
            list_albums,
            search_tracks,
            sources_list,
            sources_add,
            sources_remove,
            index_source,
            sources_add_and_index,
            // P2 行为层：喜欢 / 播放历史
            track_toggle_like,
            liked_ids,
            unlike_tracks,
            play_history,
            history_clear,
            // P3 收藏与统计视图
            liked_tracks,
            stats_overview,
            recent_plays,
            // P4 工具箱：CUE 分轨
            cue_pick,
            cue_inspect,
            cue_split,
            // P5 文件关联：冷启动待打开文件
            take_startup_files,
            // P5 更新（网络边界：updater）
            check_update,
            install_update,
            restart_app,
            // P6 在线封面（网络边界：用户显式触发）
            cover_fetch,
            cover_pick_image,
            cover_set_local,
            track_cover,
            artist_cover,
            artist_cover_local,
            artist_covers_local,
            // P6 歌词（网络边界：打开歌词面板时）
            lyrics_fetch,
            // P6.4 歌单
            playlist_create,
            playlists_list,
            playlist_tracks,
            playlist_add,
            playlist_remove,
            playlist_rename,
            playlist_delete,
            playlist_move,
            playlist_export,
            playlists_covers,
            playlist_cleanup_preview,
            playlist_smart_cleanup,
            // P6.10 详情页
            artist_tracks,
            search_all,
            album_tracks,
            // P2 播放：队列/播放控制/状态
            player_play_queue,
            player_toggle,
            player_pause,
            player_stop,
            player_next,
            player_prev,
            player_jump,
            player_seek,
            player_set_volume,
            player_queue_move,
            player_queue_remove,
            player_queue_append,
            player_queue_clear,
            player_queue_insert_next,
            player_set_mode,
            player_status
        ])
        .on_window_event(|window, event| {
            // P2：关闭 = 隐藏到托盘（播放不中断）；真正的退出走托盘菜单「退出 MusicForge」。
            // 常驻托盘是音乐播放器的预期行为（关窗口≠停音乐）。
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// 构建系统托盘：菜单（播放控制 + 显示/退出）+ 左键点击显示主窗口。
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let play_pause = MenuItem::with_id(app, "play_pause", "播放 / 暂停", true, None::<&str>)?;
    let prev = MenuItem::with_id(app, "prev", "上一首", true, None::<&str>)?;
    let next = MenuItem::with_id(app, "next", "下一首", true, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 MusicForge", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&play_pause, &prev, &next, &show, &quit])?;

    let mut builder = TrayIconBuilder::with_id("mf-tray")
        .tooltip("MusicForge")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let handle = app.state::<audio::PlayerHandle>();
            match event.id.as_ref() {
                "play_pause" => {
                    let _ = handle.toggle();
                }
                "prev" => {
                    let _ = handle.prev();
                }
                "next" => {
                    let _ = handle.next();
                }
                "show" => show_main_window(app),
                "quit" => app.exit(0),
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

/// 显示并聚焦主窗口（托盘菜单 / 左键点击 / 单实例二启动共用）。
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

// ============ P5：崩溃日志 ============

/// 崩溃日志路径：数据目录 `logs/crash.log`（与状态库同目录）。
fn crash_log_path() -> PathBuf {
    musicforge_core::db::default_db_path()
        .parent()
        .map(|p| p.join("logs").join("crash.log"))
        .unwrap_or_else(|| PathBuf::from("musicforge-crash.log"))
}

/// panic 时把信息追加到崩溃日志。release 为 `panic = "abort"`：hook 先于 abort
/// 执行，因此这里仍能留痕。**日志写入失败绝不再引发 panic**（全部忽略错误）。
fn init_panic_logging() {
    let path = crash_log_path();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(f, "[unix {ts}] panic: {info}");
        }
    }));
}

// ============ P5：文件关联（.ncm）与冷启动参数 ============

/// 冷启动 argv 里的待打开 `.ncm`（前端 mount 后 take 走，只消费一次）。
#[derive(Default)]
pub struct StartupFiles(pub Mutex<Vec<String>>);

/// 从命令行参数提取 `.ncm` 路径（忽略 `-` 开头的开关；去重保序）。
fn ncm_paths_from_args<I: IntoIterator<Item = String>>(args: I) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    args.into_iter()
        .filter(|a| !a.starts_with('-'))
        .filter(|a| a.to_ascii_lowercase().ends_with(".ncm"))
        .filter(|a| seen.insert(a.clone()))
        .collect()
}

/// P5：取走冷启动待打开文件（take 语义——前端只处理一次）。
#[tauri::command]
fn take_startup_files(state: tauri::State<StartupFiles>) -> Vec<String> {
    state
        .0
        .lock()
        .map(|mut v| std::mem::take(&mut *v))
        .unwrap_or_default()
}

// ============ P5：更新（唯一网络行为——见 docs/dependency-policy.md）============

/// 检查更新：拉取 `latest.json` 并校验（签名校验发生在下载安装时）。
/// 无更新 → `{ available: false }`；有 → 版本号与说明。**不自动下载**。
#[tauri::command]
async fn check_update(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(u)) => Ok(serde_json::json!({
            "available": true,
            "version": u.version,
            "currentVersion": u.current_version,
            "notes": u.body,
        })),
        Ok(None) => Ok(serde_json::json!({ "available": false })),
        Err(e) => Err(e.to_string()),
    }
}

/// 下载并安装更新。**签名校验失败会在此报错**（绝不静默降级）。
/// 安装完成后需重启应用生效（`restart_app`）。
#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await.map_err(|e| e.to_string())? {
        Some(u) => u
            .download_and_install(|_, _| {}, || {})
            .await
            .map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

/// 重启应用（更新安装完成后由用户点击触发）。
#[tauri::command]
fn restart_app(app: tauri::AppHandle) {
    app.restart();
}

#[cfg(test)]
mod startup_files_tests {
    use super::ncm_paths_from_args;

    /// 只收 .ncm、忽略开关、去重保序、扩展名大小写不敏感。
    #[test]
    fn extracts_ncm_paths_only() {
        let got = ncm_paths_from_args(
            [
                "C:\\x\\a.ncm",
                "--flag",
                "b.NCM",
                "C:\\x\\a.ncm",
                "song.mp3",
                "-o",
                "C:\\y\\c.ncm",
            ]
            .into_iter()
            .map(String::from),
        );
        assert_eq!(
            got,
            vec![
                "C:\\x\\a.ncm".to_string(),
                "b.NCM".to_string(),
                "C:\\y\\c.ncm".to_string()
            ]
        );
        assert!(ncm_paths_from_args(Vec::<String>::new()).is_empty());
    }
}

// ============ P6a（X37/X36）：插件面板功能态 ============
//
// 命令面新增 `plugins_status` / `plugins_set_enabled`：
// - **零网络**：只做本地文件读取（白名单插件目录 + config.json），页面无任何远程请求；
// - **白名单**（PLUGIN_POLICY.md）：`%LOCALAPPDATA%\MusicForge\plugins\` 与
//   `~/.local/share/musicforge/plugins/`——白名单之外的路径一律不可见；
// - 清单读取是**纯 JSON 解析**（不 spawn 插件进程）——默认构建（无 plugin-host
//   feature）同样可用；进程级 spawn/host 属 CLI/GUI 后续 AI 流程，不在本面板；
// - 启用状态持久化走 X36 config（`plugins.enabled`），缺失段回退默认（空）。

/// 白名单插件目录（PLUGIN_POLICY.md；本机场景）。
#[cfg(test)]
mod tests {
    use super::*;

    /// B10：扫描/收集类命令已改为 `async`（阻塞段走 `spawn_blocking`，不再占
    /// UI 主线程）。同步 `#[test]` 用 `block_on` 驱动，**断言语义不变**。
    fn block<T>(f: impl std::future::Future<Output = T>) -> T {
        tauri::async_runtime::block_on(f)
    }

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
        let flat = block(collect_files(
            vec![base.to_string_lossy().into_owned()],
            false,
        ));
        assert_eq!(flat.len(), 1, "非递归不得进入 sub，且必须过滤非 .ncm");
        assert!(flat[0].path.ends_with("a.ncm"), "应收集 a.ncm");
        assert!(flat[0].root.is_some(), "目录输入必须带 root");

        // 递归：a.ncm + sub/c.ncm
        let rec = block(collect_files(
            vec![base.to_string_lossy().into_owned()],
            true,
        ));
        assert_eq!(rec.len(), 2, "递归应收集 sub 下的 .ncm");

        // 散文件输入：root 必须为 None（前端据此区分两种输入来源）
        let single = block(collect_files(
            vec![base.join("a.ncm").to_string_lossy().into_owned()],
            false,
        ));
        assert_eq!(single.len(), 1);
        assert!(single[0].root.is_none(), "散文件输入 root 必须为 None");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// scan_library：返回形状与 CLI `scan --json` 同源（summary/ruleHits/items）。
    /// 前端 ScanPanel 按这些键渲染——键名一旦漂移即面板空白（静默断裂）。
    #[test]
    fn scan_library_command_returns_shared_shape() {
        let base = std::env::temp_dir().join(format!("mf-gui-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("a.flac"), b"fLaC").unwrap();
        std::fs::write(base.join("Thumbs.db"), b"x").unwrap();

        let v = block(scan_library(base.to_string_lossy().into_owned(), true)).unwrap();
        assert_eq!(v["dir"], base.to_string_lossy().as_ref());
        assert_eq!(v["scannedFiles"], 2);
        assert_eq!(v["summary"]["audio"], 1);
        assert_eq!(v["summary"]["junk"], 1);
        assert_eq!(v["summary"]["emptyDirs"], 0);

        let hits = v["ruleHits"].as_array().unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["id"], "MF-CLEAN-001");
        assert_eq!(hits[0]["count"], 1);
        assert!(!hits[0]["description"].as_str().unwrap().is_empty());
        assert_eq!(hits[0]["risk"], "low");

        let items = v["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        let junk = items
            .iter()
            .find(|i| i["rule"] == "MF-CLEAN-001")
            .expect("垃圾项必须带规则 ID");
        assert_eq!(junk["category"], "junk");

        // 错误路径：目录不存在必须显式 Err（前端可见），绝不返回空报告伪装成功
        let err = block(scan_library("Z:/definitely/missing/dir".to_string(), true));
        assert!(err.is_err(), "不存在的目录必须报错");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// dedupe_scan / dedupe_apply 契约：组结构 + 改选执行 + 路径逃逸拒绝。
    #[test]
    fn dedupe_commands_contract() {
        let base = std::env::temp_dir().join(format!("mf-gui-dup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("a.flac"), b"dup-content").unwrap();
        std::fs::write(base.join("b.flac"), b"dup-content").unwrap();
        std::fs::write(base.join("unique.flac"), b"unique").unwrap();

        let v = block(dedupe_scan(base.to_string_lossy().into_owned())).unwrap();
        let groups = v["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 1, "应恰 1 个 exact 组: {v}");
        assert_eq!(groups[0]["all"].as_array().unwrap().len(), 2);
        assert_eq!(groups[0]["sacrifices"].as_array().unwrap().len(), 1);
        assert!(!groups[0]["keep"]["path"].as_str().unwrap().is_empty());

        // 改选执行：牺牲组内另一成员（用户改选语义 = 前端换 radio 后提交）
        let keep_path = groups[0]["keep"]["path"].as_str().unwrap();
        let other = if keep_path.ends_with("a.flac") {
            base.join("b.flac")
        } else {
            base.join("a.flac")
        };
        let r = dedupe_apply(
            base.to_string_lossy().into_owned(),
            vec![other.to_string_lossy().into_owned()],
        )
        .unwrap();
        assert_eq!(r["moved"].as_u64(), Some(1));
        assert!(!other.exists(), "牺牲项应已进回收站");
        assert!(Path::new(keep_path).exists(), "保留项原位");
        let rb = r["rollback"].as_str().unwrap();
        assert!(Path::new(rb).exists(), "回滚清单应存在");

        // 路径逃逸：曲库目录之外的文件必须被拒绝
        let outside = std::env::temp_dir().join(format!("mf-outside-{}.txt", std::process::id()));
        std::fs::write(&outside, b"x").unwrap();
        let err = dedupe_apply(
            base.to_string_lossy().into_owned(),
            vec![outside.to_string_lossy().into_owned()],
        )
        .unwrap_err();
        assert!(err.contains("安全拒绝"), "必须显式拒绝路径逃逸: {err}");
        assert!(outside.exists(), "外部文件不得被动");

        let _ = std::fs::remove_dir_all(&base);
        std::fs::remove_file(&outside).ok();
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

    // ---- P6a（X37/X36）：插件面板命令契约 ----

    /// P6b.4：format_migrate 核心在默认构建（无 plugin-host）下响亮降级
    /// （MF-PLUGIN-NOT-FOUND），绝不静默装作执行过。
    #[test]
    fn format_migrate_command_loud_without_runtime() {
        let err =
            format_migrate_core("kwm-migration", "C:/music/song.kwm", None, None).unwrap_err();
        assert!(err.contains("MF-PLUGIN-NOT-FOUND"), "{err}");
        assert!(err.contains("建议"), "必须带可操作建议: {err}");
        // output_dir 缺省 → 源父目录语义（不 panic）
        let err2 = format_migrate_core(
            "kwm-migration",
            "C:/music/song.kwm",
            Some("C:/music/out"),
            None,
        )
        .unwrap_err();
        assert!(err2.contains("MF-PLUGIN-NOT-FOUND"), "{err2}");
    }

    /// P6b.2 既有语义的命令层回归：acknowledge 幂等追加 + 空名拒绝。
    #[test]
    fn plugins_acknowledge_inner_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "mf-gui-ack-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let cfg_path = dir.join("config.json");
        plugins_acknowledge_inner(&cfg_path, "kwm-migration").unwrap();
        plugins_acknowledge_inner(&cfg_path, "kwm-migration").unwrap();
        let cfg = musicforge_core::config::AppConfig::load(&cfg_path).unwrap();
        assert_eq!(cfg.plugins.acked, vec!["kwm-migration".to_string()]);
        assert!(
            plugins_acknowledge_inner(&cfg_path, "  ").is_err(),
            "空名必须拒绝"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// plugins_status：形状契约 + 白名单目录扫描 + config 启用列表透传。
    #[test]
    fn plugins_status_contract() {
        let base = std::env::temp_dir().join(format!("mf-gui-plugins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pdir = base.join("plugins").join("mock-ai");
        std::fs::create_dir_all(&pdir).unwrap();
        std::fs::write(
            pdir.join("plugin.json"),
            r#"{"name":"mock-ai","api_version":"1.0.0","kind":"ai","network":false,"ack_required":false,"extensions":[]}"#,
        )
        .unwrap();
        // 白名单外的坏清单：缺 name → 必须被跳过，绝不让面板空白/崩溃
        let bad = base.join("plugins").join("broken");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("plugin.json"), r#"{"api_version":"1.0.0"}"#).unwrap();

        let cfg_path = base.join("config.json");
        let v = plugins_status_inner(&cfg_path, &[base.join("plugins")]);
        assert_eq!(v["runtimeAvailable"], cfg!(feature = "plugin-host"));
        assert_eq!(v["pluginDirs"].as_array().unwrap().len(), 1);
        let installed = v["installed"].as_array().unwrap();
        assert_eq!(installed.len(), 1, "坏清单必须跳过: {v}");
        assert_eq!(installed[0]["name"], "mock-ai");
        assert_eq!(installed[0]["apiVersion"], "1.0.0");
        assert_eq!(installed[0]["network"], false);
        assert_eq!(
            v["enabled"],
            serde_json::json!([]),
            "缺省 config → 空 enabled（X36 空段语义）"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// plugins_set_enabled：持久化 roundtrip + 空名/重复名显式拒绝。
    #[test]
    fn plugins_set_enabled_roundtrip_and_validation() {
        let base = std::env::temp_dir().join(format!("mf-gui-plugins-set-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let cfg_path = base.join("nested").join("config.json");

        let r = plugins_set_enabled_inner(
            &cfg_path,
            vec!["ai-openai-compatible".into(), "lyrics-online".into()],
        )
        .unwrap();
        assert_eq!(
            r["enabled"],
            serde_json::json!(["ai-openai-compatible", "lyrics-online"])
        );
        // 落盘验证：重新加载 config 必须读到同一列表（X36 持久化语义）
        let cfg = musicforge_core::config::AppConfig::load(&cfg_path).unwrap();
        assert_eq!(cfg.plugins.enabled.len(), 2);

        // 空名拒绝
        let err = plugins_set_enabled_inner(&cfg_path, vec!["  ".into()]).unwrap_err();
        assert!(err.contains("不得为空"), "{err}");
        // 重复名拒绝
        let err = plugins_set_enabled_inner(&cfg_path, vec!["a".into(), "a".into()]).unwrap_err();
        assert!(err.contains("重复"), "{err}");
        // 失败路径不得破坏既有配置
        let cfg2 = musicforge_core::config::AppConfig::load(&cfg_path).unwrap();
        assert_eq!(cfg2.plugins.enabled.len(), 2, "被拒绝的调用不得改写配置");

        let _ = std::fs::remove_dir_all(&base);
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
