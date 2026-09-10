// 拆分自 main.rs（P9 可维护性治理：巨型入口文件拆解）——**逐字迁移，行为零变更**。
use crate::*;

/// 去重子命令：内容重复分组 + 可解释保留评分（默认 dry-run；牺牲项进回收站）。
/// dedupe 子命令参数包（clippy too_many_arguments：8 个独立参数收拢为结构体）。
pub struct DedupeArgs {
    pub state_db: Option<String>,
    pub apply: bool,
    pub trash: Option<String>,
    pub no_same_name: bool,
    pub include_same_name: bool,
    pub suggest: bool,
    pub covers: bool,
    pub json: bool,
}

pub fn run_dedupe_sub(dir: &str, a: &DedupeArgs) -> i32 {
    use musicforge_cli::safety::{resolve, ExecMode, OpClass, OpFlags};
    let DedupeArgs {
        ref state_db,
        apply,
        ref trash,
        no_same_name,
        include_same_name,
        suggest,
        covers,
        json: as_json,
    } = *a;
    let state_db = state_db.as_deref();
    let trash = trash.as_deref();

    // AI 保留建议：v0.7.0 起（review_duplicate_group 方法）。离线版显式报稳定码，
    // 绝不静默装作给过建议（K5/G5 教训：兜底伪装成功是最大敌）。
    if suggest {
        eprintln!(
            "✗ MF-PLUGIN-NOT-FOUND: AI 保留建议需要 review_duplicate_group 插件（v0.7.0 提供）。\
当前为完全离线版：请依据评分明细自行决策，或等待插件版。"
        );
        return 1;
    }

    let class = OpClass::Destructive { high_risk: false };
    let flags = OpFlags {
        dry_run: !apply,
        apply,
        yes: true, // 非高危：牺牲项全部进回收站，可整体还原
    };
    let mode = match resolve(class, &flags) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("✗ {e}");
            return 2;
        }
    };

    let db = match state_db {
        Some(p) => match musicforge_core::db::Db::open(Path::new(p)) {
            Ok(d) => Some(d),
            Err(e) => {
                eprintln!("⚠ 状态库打开失败（不使用缓存，直接计算）: {e}");
                None
            }
        },
        None => None,
    };

    let options = musicforge_core::dedupe::DedupeOptions {
        same_name: !no_same_name,
    };
    let report = match musicforge_core::dedupe::dedupe_scan(Path::new(dir), &options, db.as_ref()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("✗ 去重扫描失败: {e}");
            return 1;
        }
    };

    let trash_root = match trash {
        Some(t) => PathBuf::from(t),
        None => Path::new(dir).join(".musicforge/trash"),
    };
    let plan = musicforge_core::dedupe::build_dedupe_plan(
        &report,
        &trash_root,
        Path::new(dir),
        include_same_name,
    );

    // 信息性统计：可回收字节数（规划范围 = plan.actions 的口径）
    let mut saved_bytes: u64 = 0;
    for g in &report.groups {
        for f in g.sacrifices() {
            saved_bytes += f.size;
        }
    }
    if include_same_name {
        for g in &report.same_name {
            for (i, f) in g.files.iter().enumerate() {
                if i != g.keep_index {
                    saved_bytes += f.size;
                }
            }
        }
    }

    // 相似封面分组（--covers；仅报告，绝不进计划——换封面属 v0.7.0 能力）
    let cover_groups = if covers {
        match musicforge_core::dedupe::similar_cover_scan(Path::new(dir)) {
            Ok(g) => Some(g),
            Err(e) => {
                eprintln!("⚠ 相似封面扫描失败（跳过该报告）: {e}");
                None
            }
        }
    } else {
        None
    };

    // 执行先行于报告（测试抓出的 bug：原实现 JSON 分支在执行前提前 return，
    // 导致 --apply --json 只打印不执行）。报告反映真实发生的事。
    let mut outcome: Option<musicforge_core::scan::CleanOutcome> = None;
    if matches!(mode, ExecMode::Apply) {
        let task_id = musicforge_cli::manifest::new_task_id();
        match musicforge_core::scan::apply_clean_plan(&plan, &task_id) {
            Ok(o) => {
                if let Some(db) = db.as_ref() {
                    let _ = db.start_task(&task_id, "dedupe", &task_id);
                    let _ = db.finish_task(&task_id, &task_id, o.moved as i64, 0);
                }
                outcome = Some(o);
            }
            Err(e) => {
                eprintln!("✗ 去重执行失败: {e}");
                return 1;
            }
        }
    }

    if as_json {
        let groups: Vec<serde_json::Value> = report
            .groups
            .iter()
            .map(|g| {
                let sacrifices: Vec<serde_json::Value> = g
                    .sacrifices()
                    .iter()
                    .map(|f| {
                        serde_json::json!({
                            "path": f.path.display().to_string(),
                            "size": f.size,
                            "score": g.score_of(f),
                            "reason": g.sacrifice_reason(f),
                        })
                    })
                    .collect();
                let keep = g.keep();
                serde_json::json!({
                    "sha256": g.sha256,
                    "size": g.size,
                    "keep": {
                        "path": keep.path.display().to_string(),
                        "score": g.score_of(keep),
                        "detail": keep.score.detail(g.files.iter().map(|f| f.score.sample_rate).max().unwrap_or(0),
                                                    g.files.iter().map(|f| f.score.bit_depth).max().unwrap_or(0)),
                    },
                    "sacrifices": sacrifices,
                })
            })
            .collect();
        let same_name: Vec<serde_json::Value> = report
            .same_name
            .iter()
            .map(|g| {
                let candidates: Vec<serde_json::Value> = g
                    .files
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != g.keep_index)
                    .map(|(_, f)| {
                        serde_json::json!({
                            "path": f.path.display().to_string(),
                            "size": f.size,
                            "score": g.score_of(f),
                            "reason": g.candidate_reason(f),
                        })
                    })
                    .collect();
                let keep = g.keep();
                serde_json::json!({
                    "stem": g.stem,
                    "keep": {
                        "path": keep.path.display().to_string(),
                        "score": g.score_of(keep),
                    },
                    "candidates": candidates,
                })
            })
            .collect();
        let out = serde_json::json!({
            "dir": dir,
            "files_seen": report.files_seen,
            "cache_hits": report.cache_hits,
            "hashed_now": report.hashed_now,
            "skipped": report.skipped,
            "groups": groups,
            "same_name": same_name,
            "plan": {
                "actions": plan.actions.len(),
                "trash_root": plan.trash_root.display().to_string(),
                "include_same_name": include_same_name,
            },
            "mode": if matches!(mode, ExecMode::Apply) { "apply" } else { "dry-run" },
            "outcome": outcome.as_ref().map(|o| serde_json::json!({
                "moved": o.moved,
                "rollback_manifest": o.rollback_manifest.as_ref().map(|p| p.display().to_string()),
            })),
            "similar_covers": cover_groups.as_ref().map(|gs| gs.iter().map(|g| serde_json::json!({
                "rep": g.rep_path.display().to_string(),
                "rep_hash": format!("{:016x}", g.rep_hash),
                "members": g.members.iter().map(|(p, h, d)| serde_json::json!({
                    "path": p.display().to_string(),
                    "hash": format!("{:016x}", h),
                    "hamming": d,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>()),
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        return 0;
    }

    println!(
        "文件 {} · 缓存命中 {} · 重算 {} · 跳过 {}",
        report.files_seen, report.cache_hits, report.hashed_now, report.skipped
    );
    println!(
        "完全重复 {} 组 · 牺牲项 {} 项 · 可回收 {} 字节",
        report.groups.len(),
        plan.actions
            .iter()
            .filter(|a| a.rule_id == "MF-DUP-EXACT")
            .count(),
        saved_bytes
    );
    for (idx, g) in report.groups.iter().enumerate() {
        let keep = g.keep();
        println!(
            "组 {}/{} sha256 {}… 保留: {}（得分 {}）",
            idx + 1,
            report.groups.len(),
            &g.sha256[..8],
            keep.path.display(),
            g.score_of(keep)
        );
        for f in g.sacrifices() {
            println!("  牺牲: {} — {}", f.path.display(), g.sacrifice_reason(f));
        }
    }
    if !report.same_name.is_empty() {
        println!(
            "同名候选 {} 组（默认仅报告{}）",
            report.same_name.len(),
            if include_same_name {
                "；本次已纳入执行"
            } else {
                "；--include-same-name 可纳入执行"
            }
        );
        for g in &report.same_name {
            let keep = g.keep();
            println!(
                "同名组 \"{}\": 建议保留 {}（得分 {}）",
                g.stem,
                keep.path.display(),
                g.score_of(keep)
            );
            for (i, f) in g.files.iter().enumerate() {
                if i != g.keep_index {
                    println!("  候选: {} — {}", f.path.display(), g.candidate_reason(f));
                }
            }
        }
    }

    if let Some(gs) = &cover_groups {
        println!("相似封面 {} 组（仅报告；换封面 v0.7.0）", gs.len());
        for g in gs {
            println!(
                "封面组 代表 {}（hash {:016x}）",
                g.rep_path.display(),
                g.rep_hash
            );
            for (p, _h, d) in &g.members {
                println!("  [d={d}] {}", p.display());
            }
        }
    }

    match mode {
        ExecMode::DryRun => {
            println!(
                "仅规划：{} 项将移入回收站（未改动任何文件）。加 --apply 执行。",
                plan.actions.len()
            );
            0
        }
        ExecMode::Apply => {
            // 执行已在此前完成（见 outcome）；此处只做汇报
            let o = outcome.as_ref().expect("apply 模式必有 outcome");
            println!("已移入回收站 {} 项（保留项原位未动）", o.moved);
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
