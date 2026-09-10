// 拆分自 main.rs（P9 可维护性治理：巨型入口文件拆解）——**逐字迁移，行为零变更**。
use crate::*;

pub fn run_scan_sub(dir: &str, recursive: bool, as_json: bool, state_db: Option<&str>) -> i32 {
    use musicforge_core::scan::{scan_library, ScanOptions};
    let report = match scan_library(
        Path::new(dir),
        &ScanOptions {
            recursive,
            ..Default::default()
        },
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("✗ 扫描失败: {e}");
            return 1;
        }
    };

    // D17：音频文件索引 + 哈希缓存写入状态库（可再生缓存；失败降级为告警，
    // 绝不因缓存问题让扫描失败——G5 教训：失败必须显式可见而非静默）
    let mut hash_stats: Option<musicforge_core::scan::HashRefreshStats> = None;
    if let Some(dbp) = state_db {
        match musicforge_core::db::Db::open(Path::new(dbp)) {
            Ok(db) => {
                hash_stats = Some(musicforge_core::scan::refresh_hash_cache(
                    &db,
                    &report.items,
                ));
            }
            Err(e) => eprintln!("⚠ 状态库打开失败（不影响扫描结果）: {e}"),
        }
    }

    if as_json {
        let items: Vec<serde_json::Value> = report
            .items
            .iter()
            .map(|i| {
                serde_json::json!({
                    "path": i.path.display().to_string(),
                    "category": format!("{:?}", i.category).to_lowercase(),
                    "rule": i.rule_id,
                    "size": i.size,
                })
            })
            .collect();
        let out = serde_json::json!({
            "dir": dir,
            "scanned_files": report.scanned_files,
            "summary": report.summary(),
            "rule_hits": report.rule_hits,
            "hash_cache": hash_stats.as_ref().map(|s| serde_json::json!({
                "considered": s.considered,
                "cache_hits": s.cache_hits,
                "hashed": s.hashed,
                "skipped": s.skipped,
            })),
            "items": items,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    } else {
        println!("扫描目录: {dir}");
        println!(
            "文件 {} · 目录 {} · 音频 {} · 歌词 {} · 封面 {} · 垃圾 {} · 其他 {} · 空目录 {}",
            report.scanned_files,
            report.scanned_dirs,
            report.audio,
            report.lyrics,
            report.covers,
            report.junk,
            report.other,
            report.empty_dirs.len()
        );
        for (id, n) in &report.rule_hits {
            let desc = musicforge_core::scan::rule_card(id)
                .map(|c| c.description)
                .unwrap_or("");
            println!("  {id} ×{n}: {desc}");
        }
        if let Some(st) = &hash_stats {
            println!(
                "哈希缓存: 命中 {} · 重算 {} · 跳过 {}",
                st.cache_hits, st.hashed, st.skipped
            );
        }
    }
    0
}

/// D13 watcher：监听目录（T0/T1/T2 分派见 core watcher 模块）。
pub fn run_watch_sub(
    dir: &str,
    level: &str,
    target: Option<&str>,
    template: &str,
    debounce_ms: u64,
    whitelist: &[String],
) -> i32 {
    use musicforge_core::watcher::{WatchLevel, WatcherConfig};
    let Some(lv) = WatchLevel::parse(level) else {
        eprintln!("✗ MF-OP-CONFLICT: 未知 level {level}（可选 t0/t1/t2）");
        return 2;
    };
    let cfg = WatcherConfig {
        level: lv,
        target_root: target.map(PathBuf::from),
        template: template.to_string(),
        debounce_ms,
        whitelist: whitelist.iter().map(PathBuf::from).collect(),
    };
    if cfg.level != WatchLevel::T0Register && cfg.target_root.is_none() {
        eprintln!("✗ MF-OP-CONFLICT: t1/t2 需要 --target（整理目标根目录）");
        return 2;
    }
    match musicforge_core::watcher::run_watch(Path::new(dir), &cfg, &|line| println!("{line}")) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("✗ watch: {e}");
            1
        }
    }
}

pub fn run_clean_sub(
    dir: Option<&str>,
    rules: Option<&str>,
    apply: bool,
    trash: Option<&str>,
    state_db: Option<&str>,
    restore: Option<&str>,
) -> i32 {
    use musicforge_cli::safety::{resolve, ExecMode, OpClass, OpFlags};

    // 回滚还原：优先处理（与 dry-run/apply 互斥；不需要 DIR）
    if let Some(rb) = restore {
        match musicforge_core::scan::restore_from_trash(Path::new(rb)) {
            Ok(n) => println!("已还原 {n} 项到原位置"),
            Err(e) => {
                eprintln!("✗ 还原失败: {e}");
                return 1;
            }
        }
        return 0;
    }

    let class = OpClass::Destructive { high_risk: false };
    let flags = OpFlags {
        dry_run: !apply,
        apply,
        yes: true, // 非高危：清洗动作全部进回收站，可整体还原
    };
    let mode = match resolve(class, &flags) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("✗ {e}");
            return 2;
        }
    };

    let Some(dir) = dir else {
        eprintln!("✗ clean 需要 <DIR>（--restore 模式除外）");
        return 2;
    };
    let report = match musicforge_core::scan::scan_library(Path::new(dir), &Default::default()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("✗ 扫描失败: {e}");
            return 1;
        }
    };

    // 规则启用集（默认全部）
    let mut enabled: std::collections::HashSet<&'static str> = musicforge_core::scan::RULE_CARDS
        .iter()
        .map(|c| c.id)
        .collect();
    if let Some(list) = rules {
        let wanted: std::collections::HashSet<&str> = list.split(',').map(|s| s.trim()).collect();
        enabled.retain(|id| wanted.contains(id));
    }

    let trash_root = match trash {
        Some(t) => PathBuf::from(t),
        None => Path::new(dir).join(".musicforge/trash"),
    };
    let task_id = musicforge_cli::manifest::new_task_id();
    let plan =
        musicforge_core::scan::build_clean_plan(&report, &enabled, &trash_root, Path::new(dir));

    match mode {
        ExecMode::DryRun => {
            println!(
                "仅规划：{} 项将移入回收站（未改动任何文件）。加 --apply 执行。",
                plan.actions.len()
            );
            for a in &plan.actions {
                println!("  [{}] {}", a.rule_id, a.path.display());
            }
            0
        }
        ExecMode::Apply => {
            let outcome = match musicforge_core::scan::apply_clean_plan(&plan, &task_id) {
                Ok(o) => o,
                Err(e) => {
                    eprintln!("✗ 清洗执行失败: {e}");
                    return 1;
                }
            };
            println!(
                "已移入回收站 {} 项 · 空目录移除 {} 个",
                outcome.moved, outcome.dirs_removed
            );
            println!(
                "回滚清单: {}",
                outcome
                    .rollback_manifest
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            );
            if let Some(dbp) = state_db {
                if let Ok(db) = musicforge_core::db::Db::open(Path::new(dbp)) {
                    let _ =
                        db.start_task(&task_id, "clean", &musicforge_cli::manifest::new_task_id());
                    let _ = db.finish_task(
                        &task_id,
                        &musicforge_cli::manifest::new_task_id(),
                        outcome.moved as i64,
                        0,
                    );
                }
            }
            0
        }
    }
}
