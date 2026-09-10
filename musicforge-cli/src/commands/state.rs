// 拆分自 main.rs（P9 可维护性治理：巨型入口文件拆解）——**逐字迁移，行为零变更**。
use crate::*;

pub struct GenreArgs {
    pub map: Option<String>,
    pub replace_all: bool,
    pub apply: bool,
    pub json: bool,
}

/// genre 写入子命令：文件名风格码 → genre 标签（默认 dry-run；绝不覆盖已有值除非 --replace-all --yes）。
pub fn run_genre_sub(dir: &str, a: &GenreArgs) -> i32 {
    use musicforge_cli::safety::{resolve, ExecMode, OpClass, OpFlags};

    let map = match &a.map {
        Some(p) => match musicforge_core::stylecode::load_genre_map(Path::new(p)) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("✗ {e}");
                return 2;
            }
        },
        None => Default::default(),
    };

    // 写标签 = 原地修改用户文件 → Destructive；--replace-all 会覆盖已有
    // genre（不可经回收站还原）→ 升级为高危，需 --yes（复用安全分级表）
    let class = OpClass::Destructive {
        high_risk: a.replace_all,
    };
    let flags = OpFlags {
        dry_run: !a.apply,
        apply: a.apply,
        yes: !a.replace_all,
    };
    let mode = match resolve(class, &flags) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("✗ {e}");
            return 2;
        }
    };

    let plan =
        match musicforge_core::stylecode::plan_genre_writes(Path::new(dir), &map, a.replace_all) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("✗ genre 规划失败: {e}");
                return 1;
            }
        };
    let (will, has_genre, no_code, no_label) = plan.counts();

    // 执行先行于报告（与前两切片同一纪律）
    let mut outcome: Option<(usize, usize)> = None;
    if matches!(mode, ExecMode::Apply) {
        outcome = Some(musicforge_core::stylecode::apply_genre_writes(&plan));
    }

    if a.json {
        let items: Vec<serde_json::Value> = plan
            .items
            .iter()
            .map(|(p, d)| {
                serde_json::json!({
                    "path": p.display().to_string(),
                    "decision": match d {
                        musicforge_core::stylecode::GenreDecision::WillWrite { genre } => {
                            serde_json::json!({"status": "will-write", "genre": genre})
                        }
                        musicforge_core::stylecode::GenreDecision::HasGenre => {
                            serde_json::json!({"status": "has-genre"})
                        }
                        musicforge_core::stylecode::GenreDecision::NoCode => {
                            serde_json::json!({"status": "no-code"})
                        }
                        musicforge_core::stylecode::GenreDecision::NoLabel => {
                            serde_json::json!({"status": "no-label"})
                        }
                    },
                })
            })
            .collect();
        let out = serde_json::json!({
            "dir": dir,
            "mode": if matches!(mode, ExecMode::Apply) { "apply" } else { "dry-run" },
            "counts": { "will_write": will, "has_genre": has_genre, "no_code": no_code, "no_label": no_label },
            "outcome": outcome.map(|(w, f)| serde_json::json!({ "written": w, "failed": f })),
            "items": items,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        return 0;
    }

    println!(
        "将写入 {} · 已有 genre 跳过 {} · 无风格码 {} · 无可写标签 {}",
        will, has_genre, no_code, no_label
    );
    for (p, d) in &plan.items {
        if let musicforge_core::stylecode::GenreDecision::WillWrite { genre } = d {
            println!("  {} → genre=\"{}\"", p.display(), genre);
        }
    }
    match mode {
        ExecMode::DryRun => {
            println!("仅规划：未改动任何文件。加 --apply 执行。");
            0
        }
        ExecMode::Apply => {
            let (w, f) = outcome.expect("apply 必有 outcome");
            println!("已写入 {w} 个文件的 genre 标签 · 失败 {f}");
            0
        }
    }
}

/// 流式计算文件 sha256（大文件友好）；失败返回 None（调用方跳过缓存回写）。
pub fn sha256_file_stream(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h).ok()?;
    Some(format!("{:x}", h.finalize()))
}

/// 把本次任务写入状态库（缓存/历史；失败降级为告警）。
pub fn record_state(path: &Path, summary: &musicforge_cli::BatchSummary) {
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
