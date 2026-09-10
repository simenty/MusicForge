// 拆分自 main.rs（P9 可维护性治理：巨型入口文件拆解）——**逐字迁移，行为零变更**。
use crate::*;

pub struct OrganizeArgs {
    pub to: Option<String>,
    pub template: String,
    pub conflict: String,
    pub apply: bool,
    pub json: bool,
}

/// genre 子命令参数包。
/// 整理子命令：按命名模板归位音频文件（默认 dry-run；移动可整体还原）。
pub fn run_organize_sub(dir: &str, a: &OrganizeArgs) -> i32 {
    use musicforge_cli::safety::{resolve, ExecMode, OpClass, OpFlags};

    let Some(strategy) = musicforge_core::organize::ConflictStrategy::parse(&a.conflict) else {
        eprintln!(
            "✗ MF-OP-CONFLICT: 未知冲突策略 {}（可选 skip|suffix|overwrite-never）",
            a.conflict
        );
        return 2;
    };

    let class = OpClass::Destructive { high_risk: false };
    let flags = OpFlags {
        dry_run: !a.apply,
        apply: a.apply,
        yes: true, // 非高危：绝不覆盖目标 + 回滚清单可整体还原
    };
    let mode = match resolve(class, &flags) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("✗ {e}");
            return 2;
        }
    };

    let target_root = match a.to.as_deref() {
        Some(t) if !t.trim().is_empty() => std::path::PathBuf::from(t),
        _ => std::path::PathBuf::from(dir),
    };
    let options = musicforge_core::organize::OrganizeOptions {
        template: &a.template,
        target_root: &target_root,
        conflict: strategy,
    };
    let plan = match musicforge_core::organize::plan_organize(Path::new(dir), &options) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("✗ 整理规划失败: {e}");
            return 1;
        }
    };
    let counts = plan.counts();

    // 执行先行于报告（与 dedupe 同一教训：报告必须反映真实发生的事）
    let mut outcome: Option<musicforge_core::organize::OrganizeOutcome> = None;
    if matches!(mode, ExecMode::Apply) {
        let task_id = musicforge_cli::manifest::new_task_id();
        match musicforge_core::organize::apply_organize_plan(&plan, &task_id) {
            Ok(o) => outcome = Some(o),
            Err(e) => {
                eprintln!("✗ 整理执行失败: {e}");
                return 1;
            }
        }
    }

    if a.json {
        let items: Vec<serde_json::Value> = plan
            .items
            .iter()
            .map(|i| {
                serde_json::json!({
                    "source": i.source.display().to_string(),
                    "target": i.target.display().to_string(),
                    "status": match i.status {
                        musicforge_core::organize::OrganizeStatus::Planned => "planned",
                        musicforge_core::organize::OrganizeStatus::AlreadyInPlace => "in-place",
                        musicforge_core::organize::OrganizeStatus::SkippedConflict => "skipped-conflict",
                        musicforge_core::organize::OrganizeStatus::ConflictNever => "conflict-never",
                    },
                    "note": i.note,
                })
            })
            .collect();
        let out = serde_json::json!({
            "dir": dir,
            "target_root": plan.target_root.display().to_string(),
            "template": plan.template,
            "conflict": plan.strategy.as_str(),
            "mode": if matches!(mode, ExecMode::Apply) { "apply" } else { "dry-run" },
            "counts": {
                "planned": counts.planned,
                "in_place": counts.in_place,
                "skipped_conflict": counts.skipped_conflict,
                "conflict_never": counts.conflict_never,
            },
            "outcome": outcome.as_ref().map(|o| serde_json::json!({
                "moved": o.moved,
                "skipped": o.skipped,
                "failed": o.failed,
                "rollback_manifest": o.rollback_manifest.as_ref().map(|p| p.display().to_string()),
            })),
            "items": items,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        return 0;
    }

    println!(
        "整理: {} → {}（模板 {} · 冲突 {}）",
        dir,
        plan.target_root.display(),
        plan.template,
        plan.strategy.as_str()
    );
    println!(
        "将移动 {} · 已在位 {} · 冲突跳过 {} · 冲突失败 {}",
        counts.planned, counts.in_place, counts.skipped_conflict, counts.conflict_never
    );
    for i in plan
        .items
        .iter()
        .filter(|i| i.status == musicforge_core::organize::OrganizeStatus::Planned)
    {
        println!("  {} → {}", i.source.display(), i.target.display());
    }
    for i in &plan.items {
        if let Some(note) = &i.note {
            println!("  [{:?}] {} — {}", i.status, i.source.display(), note);
        }
    }

    match mode {
        ExecMode::DryRun => {
            println!(
                "仅规划：{} 项将移动（未改动任何文件）。加 --apply 执行。",
                counts.planned
            );
            0
        }
        ExecMode::Apply => {
            let o = outcome.as_ref().expect("apply 模式必有 outcome");
            println!(
                "已移动 {} 项 · 跳过 {} · 失败 {}（失败项绝不覆盖、绝不删源）",
                o.moved, o.skipped, o.failed
            );
            println!(
                "回滚清单: {}",
                o.rollback_manifest
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            );
            0
        }
    }
}

/// 播放清单子命令（只写清单文件，不动音乐文件）。
pub fn run_playlist_sub(cmd: PlaylistCmd) -> i32 {
    match cmd {
        PlaylistCmd::Export { dir, out, by } => {
            let Some(group_by) = musicforge_core::playlist::GroupBy::parse(&by) else {
                eprintln!("✗ 未知分类键 {by}（可选 artist|album|none）");
                return 2;
            };
            match musicforge_core::playlist::export_playlists(
                Path::new(&dir),
                Path::new(&out),
                group_by,
            ) {
                Ok(rep) => {
                    println!(
                        "导出 {} 个清单（曲库 {} 文件）→ {}",
                        rep.playlists.len(),
                        rep.files_seen,
                        out
                    );
                    for (p, n) in &rep.playlists {
                        println!("  {}（{} 条）", p.display(), n);
                    }
                    0
                }
                Err(e) => {
                    eprintln!("✗ 导出失败: {e}");
                    1
                }
            }
        }
        PlaylistCmd::Import {
            file,
            search,
            out,
            json,
        } => {
            match musicforge_core::playlist::import_and_repair(
                Path::new(&file),
                Path::new(&search),
                out.as_deref().map(Path::new),
            ) {
                Ok(rep) => {
                    if json {
                        let o = serde_json::json!({
                            "total": rep.total_entries,
                            "ok": rep.ok,
                            "repaired": rep.repaired.iter().map(|(a, b)| serde_json::json!({
                                "from": a.display().to_string(),
                                "to": b.display().to_string(),
                            })).collect::<Vec<_>>(),
                            "unresolved": rep.unresolved.iter().map(|(l, r)| serde_json::json!({
                                "line": l, "reason": r,
                            })).collect::<Vec<_>>(),
                            "written": rep.written.as_ref().map(|p| p.display().to_string()),
                        });
                        println!("{}", serde_json::to_string_pretty(&o).unwrap_or_default());
                    } else {
                        println!(
                            "共 {} 条 · 直接命中 {} · 修复 {} · 未修复 {}",
                            rep.total_entries,
                            rep.ok,
                            rep.repaired.len(),
                            rep.unresolved.len()
                        );
                        for (a, b) in &rep.repaired {
                            println!("  修复: {} → {}", a.display(), b.display());
                        }
                        for (l, r) in &rep.unresolved {
                            println!("  未修复: {l} — {r}");
                        }
                        println!(
                            "修复后清单: {}",
                            rep.written
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default()
                        );
                    }
                    0
                }
                Err(e) => {
                    eprintln!("✗ 导入失败: {e}");
                    1
                }
            }
        }
    }
}
