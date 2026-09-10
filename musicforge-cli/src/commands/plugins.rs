// 拆分自 main.rs（P9 可维护性治理：巨型入口文件拆解）——**逐字迁移，行为零变更**。
use crate::*;

/// 简易 UTC 时间戳（避免引入 chrono）。
pub fn chrono_like_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

// ---- P6b：插件管理 / 格式迁移子命令 ----

pub fn run_plugins_sub(cmd: PluginsCmd) -> i32 {
    let config_path = musicforge_core::config::AppConfig::default_path();
    match cmd {
        PluginsCmd::List => {
            let cfg = musicforge_core::config::AppConfig::load(&config_path).unwrap_or_default();
            let status = musicforge_cli::plugins::status(&cfg, &musicforge_cli::plugins::dirs());
            println!("{}", serde_json::to_string_pretty(&status).unwrap());
            0
        }
        PluginsCmd::Enable { names } => {
            if let Err(e) = musicforge_cli::plugins::set_enabled(&config_path, &names) {
                eprintln!("✗ {e}");
                return 1;
            }
            println!(
                "✓ 已启用: {}",
                if names.is_empty() {
                    "（空——全部禁用）".to_string()
                } else {
                    names.join(", ")
                }
            );
            0
        }
        PluginsCmd::DisableAll => {
            if let Err(e) = musicforge_cli::plugins::set_enabled(&config_path, &[]) {
                eprintln!("✗ {e}");
                return 1;
            }
            println!("✓ 已禁用全部插件（离线模式）");
            0
        }
        PluginsCmd::Acknowledge { name } => {
            if let Err(e) = musicforge_cli::plugins::acknowledge(&config_path, &name) {
                eprintln!("✗ {e}");
                return 1;
            }
            println!("✓ 已确认高风险插件 {name}（ACK 闸放行；写入 config.json plugins.acked）");
            0
        }
    }
}

pub fn run_format_migrate_sub(
    plugin: &str,
    source: &str,
    output_dir: &str,
    work_root: Option<&str>,
    ekey: Option<&str>,
) -> i32 {
    match musicforge_cli::format_migrate(plugin, source, output_dir, work_root, ekey) {
        Ok(output_path) => {
            println!("✓ 迁移完成: {output_path}");
            println!("  产物已通过 magic + 音频属性双验；源文件未被修改。");
            0
        }
        Err(e) => {
            eprintln!("✗ {}: {e} | 建议: {}", e.code(), e.suggestion());
            1
        }
    }
}
