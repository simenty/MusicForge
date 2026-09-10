// 插件状态/启用/ACK 与格式迁移 IPC 命令
// 拆分自 main.rs（P9 可维护性治理）——逐字迁移，行为零变更。
use crate::*;

pub fn plugin_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(la) = std::env::var_os("LOCALAPPDATA") {
        v.push(PathBuf::from(la).join("MusicForge").join("plugins"));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    if let Some(h) = home {
        v.push(h.join(".local/share/musicforge/plugins"));
    }
    v
}

/// 读取单个插件目录的 `plugin.json`（解析失败/字段缺失 → None，跳过不猜）。
pub fn read_plugin_manifest(dir: &Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(dir.join("plugin.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let name = v.get("name")?.as_str()?.to_string();
    let api_version = v.get("api_version")?.as_str()?.to_string();
    let kind = v.get("kind")?.as_str()?.to_string();
    let network = v.get("network").and_then(|n| n.as_bool()).unwrap_or(false);
    // P6b.2：ACK 闸声明 + 能力声明（extensions 透传给前端展示）
    let ack_required = v
        .get("ack_required")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let extensions = v
        .get("extensions")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(serde_json::json!({
        "name": name,
        "apiVersion": api_version,
        "kind": kind,
        "network": network,
        "ackRequired": ack_required,
        "extensions": extensions,
        "dir": dir.display().to_string(),
    }))
}

/// 插件状态装配（可注入路径，测试友好）。
pub fn plugins_status_inner(config_path: &Path, dirs: &[PathBuf]) -> serde_json::Value {
    let cfg = musicforge_core::config::AppConfig::load(config_path).unwrap_or_default();
    let mut installed: Vec<serde_json::Value> = Vec::new();
    for d in dirs {
        if !d.is_dir() {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                if let Some(m) = read_plugin_manifest(&p) {
                    installed.push(m);
                }
            }
        }
    }
    serde_json::json!({
        "runtimeAvailable": cfg!(feature = "plugin-host"),
        "configPath": config_path.display().to_string(),
        "pluginDirs": dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>(),
        "enabled": cfg.plugins.enabled,
        "installed": installed,
    })
}

/// 插件面板状态（X37）：运行时可用性 + 白名单目录已装清单 + config 启用列表。
#[tauri::command]
pub fn plugins_status() -> serde_json::Value {
    plugins_status_inner(
        &musicforge_core::config::AppConfig::default_path(),
        &plugin_dirs(),
    )
}

/// 更新启用列表（X36 config 持久化；返回生效后的列表）。
///
/// 校验：空名/重复名拒绝（显式失败，绝不静默去重——配置错了要让用户看见）。
pub fn plugins_set_enabled_inner(
    config_path: &Path,
    enabled: Vec<String>,
) -> Result<serde_json::Value, String> {
    let mut seen = std::collections::BTreeSet::new();
    for name in &enabled {
        let name = name.trim();
        if name.is_empty() {
            return Err("插件名不得为空".to_string());
        }
        if !seen.insert(name.to_string()) {
            return Err(format!("重复的插件名: {name}"));
        }
    }
    let mut cfg =
        musicforge_core::config::AppConfig::load(config_path).map_err(|e| e.to_string())?;
    cfg.plugins.enabled = seen.into_iter().collect();
    cfg.save(config_path).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "enabled": cfg.plugins.enabled }))
}

/// 启用列表更新命令（写入 config.json 的 `plugins.enabled`）。
#[tauri::command]
pub fn plugins_set_enabled(enabled: Vec<String>) -> Result<serde_json::Value, String> {
    plugins_set_enabled_inner(&musicforge_core::config::AppConfig::default_path(), enabled)
}

/// P6b.2：高风险插件 ACK 确认（GUI 等价 `musicforge plugins acknowledge`）。
///
/// 幂等；写入 config.json `plugins.acked`。前端必须展示风险提示后再调用
/// （格式迁移类：在授权工作根内读写文件）。
pub fn plugins_acknowledge_inner(config_path: &Path, name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("插件名不得为空".to_string());
    }
    let mut cfg =
        musicforge_core::config::AppConfig::load(config_path).map_err(|e| e.to_string())?;
    if !cfg.plugins.acked.iter().any(|x| x == name) {
        cfg.plugins.acked.push(name.trim().to_string());
        cfg.save(config_path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn plugins_acknowledge(name: String) -> Result<(), String> {
    plugins_acknowledge_inner(&musicforge_core::config::AppConfig::default_path(), &name)
}

/// P6b.4：按插件能力声明选择迁移源文件（对话框仅 Rust 侧可调）。
#[tauri::command]
pub async fn select_migration_files(
    app: AppHandle,
    extensions: Vec<String>,
    start_dir: Option<String>,
) -> Vec<String> {
    let exts: Vec<&str> = extensions.iter().map(|s| s.as_str()).collect();
    let mut d = app
        .dialog()
        .file()
        .add_filter("音频容器（待迁移）", &exts)
        .set_title("选择待迁移文件（可多选）");
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

/// P6b.4：格式迁移执行核心（同步；命令层薄封装）。
/// X49：ekey 透传（QMCv2 尾标变体——用户自备，本地传递，零网络）。
pub fn format_migrate_core(
    plugin: &str,
    source: &str,
    output_dir: Option<&str>,
    ekey: Option<&str>,
) -> Result<serde_json::Value, String> {
    let out_dir = match output_dir.map(str::trim) {
        Some(o) if !o.is_empty() => o.to_string(),
        _ => Path::new(source)
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
    };
    let output_path = musicforge_cli::format_migrate(plugin, source, &out_dir, None, ekey)
        .map_err(|e| format!("{}: {e} | 建议: {}", e.code(), e.suggestion()))?;
    Ok(serde_json::json!({ "outputPath": output_path }))
}

/// P6b.4：格式迁移执行（经 musicforge-cli 桥接；output_dir 缺省 = 源父目录；
/// 默认构建响亮报 MF-PLUGIN-NOT-FOUND）。
/// X49：ekey = 用户自备密钥（QMC STag 变体；插件业务码 QMC-EKEY-REQUIRED/
/// INVALID 以 source_code 前缀透传——前端识别后展示引导）。
#[tauri::command]
pub async fn format_migrate(
    plugin: String,
    source: String,
    output_dir: Option<String>,
    ekey: Option<String>,
) -> Result<serde_json::Value, String> {
    format_migrate_core(&plugin, &source, output_dir.as_deref(), ekey.as_deref())
}

// ============ P1a 保护网：GUI ↔ 前端 IPC 契约测试 ============
//
// 两类覆盖：
// 1. **可纯调用的命令**（无 AppHandle/State 依赖）：真实行为断言。
// 2. **需要 AppHandle/State 的命令**：`cargo test` 下无法构造 Tauri 运行时，
//    用「包装函数 + 显式返回类型标注」做**编译期类型钉子** —— 返回类型一变，
//    本模块立即编译失败。字段级 schema 另由 InputPair/BatchArgs/FailureRow
//    的 serde 断言覆盖（前端真正依赖的是字段名与形状）。
