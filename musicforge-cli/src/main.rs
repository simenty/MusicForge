//! musicforge CLI 入口（薄壳：参数解析 + 汇总输出 + 退出码；批处理逻辑在 lib.rs）

use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use musicforge_cli::{run, run_resume, BatchConfig};

/// MusicForge — 默认离线、可靠、可观测的本地 ncm 转换器
///
/// 仅处理你已合法获得的文件的个人本地格式转换。零网络、不上传、不收集数据。
#[derive(Parser, Debug)]
#[command(
    name = "musicforge",
    version,
    about,
    after_help = "法律须知：本项目仅支持处理你已合法获得的文件的个人本地格式转换/备份。",
    subcommand_negates_reqs = true,
    args_conflicts_with_subcommands = true
)]
struct Args {
    /// .ncm 文件（可多个）
    files: Vec<String>,

    /// 处理目录（可与 -r 同用）
    #[arg(short = 'd', long)]
    directory: Option<String>,

    /// 递归处理目录（保留目录结构）
    #[arg(short = 'r', long, requires = "directory")]
    recursive: bool,

    /// 输出根目录（默认输出到源文件同目录）
    #[arg(short = 'o', long)]
    output: Option<String>,

    /// 跳过已存在且大小完整的输出文件
    #[arg(long)]
    skip_existing: bool,

    /// 并发数（默认 4）
    #[arg(short = 'j', long, default_value_t = 4)]
    jobs: usize,

    /// 导出失败清单 CSV 到指定路径
    #[arg(long)]
    export_failures: Option<String>,

    /// 命名模板（占位符 {title}/{artist}/{album}/{track}/{track:02d}/{format}；/ 产生子目录）
    #[arg(long, default_value = "{artist} - {title}")]
    template: String,

    /// 只规划不落盘：产出 manifest 计划条目，不写任何音频与侧车文件（v0.2.0）
    #[arg(long)]
    dry_run: bool,

    /// manifest 路径；不指定则不写留痕文件（dry-run 建议指定以便查看计划）
    #[arg(long)]
    manifest: Option<String>,

    /// 断点续跑：跳过该 manifest 中已成功完成的文件（配合 --manifest 使用）
    #[arg(long)]
    resume: Option<String>,

    /// 状态库路径（可再生缓存：文件索引/哈希缓存/任务历史）；必须是本地目录
    #[arg(long)]
    state_db: Option<String>,

    #[command(subcommand)]
    command: Option<Sub>,
}

#[derive(clap::Subcommand, Debug)]
enum Sub {
    /// 只读扫描：分类文件并报告垃圾/异常项（不改动任何文件）
    Scan {
        /// 要扫描的目录
        #[arg(value_name = "DIR")]
        dir: String,
        /// 递归子目录（默认开）
        #[arg(short = 'r', long, default_value_t = true)]
        recursive: bool,
        /// JSON 输出（机器可读，供 GUI/脚本消费）
        #[arg(long)]
        json: bool,
        /// 状态库：把音频文件索引写入 db（可再生缓存）
        #[arg(long)]
        state_db: Option<String>,
    },
    /// 清洗：把命中的垃圾/异常项移入回收站（默认 dry-run，--apply 才执行）
    Clean {
        /// 要清洗的目录（--restore 模式下不需要）
        #[arg(value_name = "DIR")]
        dir: Option<String>,
        /// 只启用指定规则（逗号分隔，如 MF-CLEAN-001,MF-CLEAN-003）；缺省=全部
        #[arg(long)]
        rules: Option<String>,
        /// 真正执行（缺省=只规划）
        #[arg(long)]
        apply: bool,
        /// 回收站根目录（默认 <DIR>/.musicforge/trash）
        #[arg(long)]
        trash: Option<String>,
        /// 状态库：任务历史与 ack 留痕
        #[arg(long)]
        state_db: Option<String>,
        /// 从回滚清单还原（值 = rollback.jsonl 路径）
        #[arg(long)]
        restore: Option<String>,
    },
    /// 去重：内容重复分组 + 可解释保留评分（默认 dry-run，--apply 移牺牲项入回收站）
    Dedupe {
        /// 要去重的目录
        #[arg(value_name = "DIR")]
        dir: String,
        /// 状态库（D17 哈希缓存：命中免重算；大库强烈建议提供）
        #[arg(long)]
        state_db: Option<String>,
        /// 真正执行（缺省=只规划；牺牲项全部进回收站，可 restore）
        #[arg(long)]
        apply: bool,
        /// 回收站根目录（默认 <DIR>/.musicforge/trash）
        #[arg(long)]
        trash: Option<String>,
        /// 关闭同名候选检测（默认开；同名组默认仅报告，不参与 apply）
        #[arg(long)]
        no_same_name: bool,
        /// 把同名候选组的非保留成员也纳入 apply 范围（同名≠同歌，慎用）
        #[arg(long, requires = "apply")]
        include_same_name: bool,
        /// AI 保留建议（v0.7.0 起提供；当前离线版显式报 MF-PLUGIN-NOT-FOUND）
        #[arg(long, conflicts_with = "apply")]
        suggest: bool,
        /// JSON 输出（机器可读）
        #[arg(long)]
        json: bool,
    },
    /// 整理：按命名模板把音频文件归位到规范目录结构（默认 dry-run，--apply 才移动）
    Organize {
        /// 要整理的曲库目录
        #[arg(value_name = "DIR")]
        dir: String,
        /// 目标根目录（缺省=原地整理）
        #[arg(long)]
        to: Option<String>,
        /// 命名模板（与 convert 同源渲染语义）
        #[arg(long, default_value = "{artist} - {title}")]
        template: String,
        /// 冲突策略（目标已存在时）：skip=报告跳过 / suffix=追加 (2) / overwrite-never=计失败
        #[arg(long, default_value = "skip")]
        conflict: String,
        /// 真正执行（缺省=只规划；移动可经回滚清单整体还原）
        #[arg(long)]
        apply: bool,
        /// JSON 输出（机器可读）
        #[arg(long)]
        json: bool,
    },
    /// 播放清单：M3U8 按分类导出 / 导入失效路径修复（只写清单文件，不动音乐）
    Playlist {
        #[command(subcommand)]
        cmd: PlaylistCmd,
    },
}

#[derive(clap::Subcommand, Debug)]
enum PlaylistCmd {
    /// 按分类导出曲库为 .m3u8 集合（每组一个文件，UTF-8 + EXTINF）
    Export {
        /// 曲库目录
        #[arg(value_name = "DIR")]
        dir: String,
        /// 输出目录（清单写到这里，必填——避免静默写进任何隐式位置）
        #[arg(long)]
        out: String,
        /// 分类键：artist（默认）/ album / none
        #[arg(long, default_value = "artist")]
        by: String,
    },
    /// 导入播放清单并修复失效路径（同名匹配 + 时长 ±1s 消歧）
    Import {
        /// 播放清单文件（.m3u / .m3u8）
        #[arg(value_name = "FILE")]
        file: String,
        /// 修复搜索根（在此目录内按文件名定位失效条目的新位置）
        #[arg(long)]
        search: String,
        /// 修复后清单写出路径（缺省 = <原名>.fixed.m3u8）
        #[arg(long)]
        out: Option<String>,
        /// JSON 输出（机器可读）
        #[arg(long)]
        json: bool,
    },
}

fn run_scan_sub(dir: &str, recursive: bool, as_json: bool, state_db: Option<&str>) -> i32 {
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

fn run_clean_sub(
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

/// 去重子命令：内容重复分组 + 可解释保留评分（默认 dry-run；牺牲项进回收站）。
/// dedupe 子命令参数包（clippy too_many_arguments：8 个独立参数收拢为结构体）。
struct DedupeArgs {
    state_db: Option<String>,
    apply: bool,
    trash: Option<String>,
    no_same_name: bool,
    include_same_name: bool,
    suggest: bool,
    json: bool,
}

fn run_dedupe_sub(dir: &str, a: &DedupeArgs) -> i32 {
    use musicforge_cli::safety::{resolve, ExecMode, OpClass, OpFlags};
    let DedupeArgs {
        ref state_db,
        apply,
        ref trash,
        no_same_name,
        include_same_name,
        suggest,
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

/// organize 子命令参数包（与 DedupeArgs 同理由：避免 too_many_arguments）。
struct OrganizeArgs {
    to: Option<String>,
    template: String,
    conflict: String,
    apply: bool,
    json: bool,
}

/// 整理子命令：按命名模板归位音频文件（默认 dry-run；移动可整体还原）。
fn run_organize_sub(dir: &str, a: &OrganizeArgs) -> i32 {
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
fn run_playlist_sub(cmd: PlaylistCmd) -> i32 {
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

/// 流式计算文件 sha256（大文件友好）；失败返回 None（调用方跳过缓存回写）。
fn sha256_file_stream(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h).ok()?;
    Some(format!("{:x}", h.finalize()))
}

/// 把本次任务写入状态库（缓存/历史；失败降级为告警）。
fn record_state(path: &Path, summary: &musicforge_cli::BatchSummary) {
    use musicforge_core::db::Db;
    let task_id = musicforge_cli::manifest::new_task_id();
    match Db::open(path) {
        Ok(db) => {
            if let Err(e) = db.start_task(&task_id, "convert", &chrono_like_now()) {
                eprintln!("⚠ 状态库写入失败（不影响转换结果）: {e}");
                return;
            }
            for r in &summary.results {
                if r.status != musicforge_cli::Status::Ok {
                    continue;
                }
                if let Some(target) = r.output.as_ref() {
                    let sha = musicforge_cli::sha256_of_sidecar(target);
                    let size = std::fs::metadata(target).map(|m| m.len()).unwrap_or(0);
                    let _ = db.upsert_file(
                        &target.to_string_lossy(),
                        size as i64,
                        None,
                        target.extension().and_then(|e| e.to_str()),
                        sha.as_deref(),
                    );
                }
                // D17：源文件行——sha256 命中缓存则跳过重算（L1+L2）；
                // 未命中才对流式读取源文件计算一次并回写缓存。
                if let Ok(md) = std::fs::metadata(&r.source) {
                    let size = md.len() as i64;
                    let mtime = md
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64);
                    if let Some(mtime) = mtime {
                        let key = r.source.to_string_lossy().into_owned();
                        let hit = db
                            .cached_hash(&key, size, mtime)
                            .map(|h| h.is_some())
                            .unwrap_or(false);
                        if !hit {
                            if let Some(sha) = sha256_file_stream(&r.source) {
                                let _ = db.upsert_file(
                                    &key,
                                    size,
                                    Some(mtime),
                                    Some("ncm"),
                                    Some(&sha),
                                );
                            }
                        }
                    }
                }
            }
            if let Err(e) = db.finish_task(
                &task_id,
                &chrono_like_now(),
                summary.ok as i64,
                summary.failed as i64,
            ) {
                eprintln!("⚠ 状态库收尾失败（不影响转换结果）: {e}");
            }
        }
        Err(e) => eprintln!("⚠ 状态库打开失败（不影响转换结果）: {e}"),
    }
}

/// 简易 UTC 时间戳（避免引入 chrono）。
fn chrono_like_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

fn main() {
    let args = Args::parse();

    // v0.3.0 子命令分派：scan / clean（legacy 顶层参数不受影响）
    if let Some(sub) = args.command {
        let code = match sub {
            Sub::Scan {
                dir,
                recursive,
                json,
                state_db,
            } => run_scan_sub(&dir, recursive, json, state_db.as_deref()),
            Sub::Clean {
                dir,
                rules,
                apply,
                trash,
                state_db,
                restore,
            } => run_clean_sub(
                dir.as_deref(),
                rules.as_deref(),
                apply,
                trash.as_deref(),
                state_db.as_deref(),
                restore.as_deref(),
            ),
            Sub::Dedupe {
                dir,
                state_db,
                apply,
                trash,
                no_same_name,
                include_same_name,
                suggest,
                json,
            } => run_dedupe_sub(
                &dir,
                &DedupeArgs {
                    state_db,
                    apply,
                    trash,
                    no_same_name,
                    include_same_name,
                    suggest,
                    json,
                },
            ),
            Sub::Organize {
                dir,
                to,
                template,
                conflict,
                apply,
                json,
            } => run_organize_sub(
                &dir,
                &OrganizeArgs {
                    to,
                    template,
                    conflict,
                    apply,
                    json,
                },
            ),
            Sub::Playlist { cmd } => run_playlist_sub(cmd),
        };
        std::process::exit(code);
    }

    let start = Instant::now();

    let mut inputs: Vec<PathBuf> = args.files.iter().map(PathBuf::from).collect();
    if let Some(d) = &args.directory {
        inputs.push(PathBuf::from(d));
    }
    if inputs.is_empty() {
        eprintln!("错误：未提供任何输入文件或目录（-h 查看用法）");
        // Windows 双击闪退修复：非管道模式下等待按键，让用户看清错误信息
        use std::io::IsTerminal;
        if std::io::stdin().is_terminal() {
            println!("\n按 Enter 键退出...");
            let mut buf = String::new();
            let _ = std::io::stdin().read_line(&mut buf);
        }
        std::process::exit(2);
    }

    // `collect_inputs` 对「不存在 / 不可读 / 非 .ncm」的输入是静默跳过的
    // （签名不含错误通道，改动会波及 GUI 的 collect_files 命令，故不在此处改契约）。
    // 后果：路径打错时表现为「汇总：成功 0」+ 退出码 0，用户以为跑完了。
    // 这里补一道可观测性兜底——只加 stderr 提示，不改退出码语义（QA 第二轮）。
    for p in &inputs {
        if !p.exists() {
            eprintln!("警告：输入路径不存在，已跳过：{}", p.display());
        }
    }
    let cfg = BatchConfig {
        inputs,
        out_dir: args.output.as_ref().map(PathBuf::from),
        recursive: args.recursive,
        skip_existing: args.skip_existing,
        jobs: args.jobs,
        template: args.template,
        cancel: None,
        dry_run: args.dry_run,
        // 默认不写 manifest：避免在任何工作目录产生 .musicforge 杂散目录；
        // 需要审计留痕时显式 --manifest <path>
        manifest: args.manifest.as_ref().map(PathBuf::from),
    };

    // v0.2.0 安全分级：convert 属「只产出新文件」的非破坏类，默认执行；
    // 破坏类命令（P3 的 clean / P4 的 dedupe）届时默认只规划，须 --apply。
    let mode = musicforge_cli::safety::resolve(
        musicforge_cli::safety::OpClass::NonDestructive,
        &musicforge_cli::safety::OpFlags {
            dry_run: args.dry_run,
            apply: false,
            yes: false,
        },
    );
    let mode = match mode {
        Ok(m) => m,
        Err(e) => {
            eprintln!("✗ {e}");
            std::process::exit(2);
        }
    };
    if mode == musicforge_cli::safety::ExecMode::DryRun {
        println!(
            "{}",
            musicforge_cli::safety::mode_note(
                musicforge_cli::safety::OpClass::NonDestructive,
                mode
            )
        );
    }

    let summary = match args.resume.as_ref() {
        Some(manifest) => run_resume(cfg, Path::new(manifest), |_| {}),
        None => run(cfg),
    };

    // 同上：零结果 + 非零输入 = 输入没匹配到任何 .ncm。
    // 不报错就会被当成「成功转换 0 个」，是典型的静默失败。
    if summary.results.is_empty() {
        eprintln!(
            "未发现任何 .ncm 文件：请检查输入路径是否正确、目录是否可读、文件扩展名是否为 .ncm"
        );
    }

    // 逐文件结果（repair receipt：失败带错误码 + 建议）
    // 注意：Ok 结果里仅「TagRead 降级」这一项会带 reason（见 lib.rs::execute_one），
    // 其余 Ok 的 reason 恒为 None。故此处打印 reason 等价于打印降级告警，互不干扰。
    let mut degraded = 0usize;
    for r in &summary.results {
        match r.status {
            musicforge_cli::Status::Ok => {
                println!(
                    "✓ {} → {}",
                    r.source.display(),
                    r.output
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                );
                if let Some(reason) = &r.reason {
                    degraded += 1;
                    eprintln!("⚠ {} —— {}", r.source.display(), reason);
                }
            }
            musicforge_cli::Status::Skipped => {
                println!("⏭ {} （已存在，跳过）", r.source.display());
            }
            musicforge_cli::Status::Cancelled => {
                println!("⏸ {} （已取消）", r.source.display());
            }
            musicforge_cli::Status::Failed => {
                if let Some(reason) = &r.reason {
                    eprintln!("✗ {} —— {}", r.source.display(), reason);
                }
            }
        }
    }

    let cancel_note = if summary.is_cancelled() {
        "（已取消）"
    } else {
        ""
    };
    let degraded_note = if degraded > 0 {
        format!("（其中 {degraded} 个音频已完整导出但元数据未写入，下次运行将自动重转）")
    } else {
        String::new()
    };
    println!(
        "\n汇总：成功 {} / 跳过 {} / 失败 {} · 耗时 {} ms{}{}",
        summary.ok,
        summary.skipped,
        summary.failed,
        summary.duration_ms,
        cancel_note,
        degraded_note
    );

    if let Some(csv) = &args.export_failures {
        let csv_path = PathBuf::from(csv);
        // 父目录不存在时自动创建，避免裸 os error 3 —— 用户拿不到有用信息
        if let Some(parent) = csv_path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    eprintln!("失败清单目录创建失败（{}）：{e}", parent.display());
                    std::process::exit(1);
                }
            }
        }
        match summary.export_failures_csv(csv_path.as_path()) {
            Ok(()) => println!("失败清单已导出：{csv}"),
            Err(e) => eprintln!("失败清单导出失败：{e}"),
        }
    }

    let _ = start; // duration 已在 summary 内
                   // 状态库留痕（可再生缓存：失败只告警，绝不因此让转换失败）
    if let Some(path) = args.state_db.as_ref() {
        record_state(Path::new(path), &summary);
    }

    std::process::exit(summary.exit_code());
}
