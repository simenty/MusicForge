// 扫描/库刷新/去重 IPC 命令（自 main.rs 逐字迁移）
// 拆分自 main.rs（P9 可维护性治理）——逐字迁移，行为零变更。
use crate::*;

/// 模板预览：给定模板 + 示例元数据 → 返回渲染后的文件名
/// 预览与真实执行共用同一渲染器（P4 语义）——预览所见即所得。
#[tauri::command]
pub fn preview_template(template: String) -> Vec<String> {
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
pub struct InputPair {
    pub path: String,
    pub root: Option<String>,
}

/// 拖拽/选择的路径 → 过滤出 .ncm 文件（大小写不敏感；目录按 recursive 递归）。
/// 返回 (path, root) 对——root 必须随行返回，前端 start_batch 原样回传。
#[tauri::command]
pub fn collect_files(inputs: Vec<String>, recursive: bool) -> Vec<InputPair> {
    let paths: Vec<PathBuf> = inputs.iter().map(PathBuf::from).collect();
    musicforge_cli::collect_inputs(&paths, recursive)
        .into_iter()
        .map(|(p, root)| InputPair {
            path: p.to_string_lossy().into_owned(),
            root: root.map(|r| r.to_string_lossy().into_owned()),
        })
        .collect()
}

/// 计划预览（dry-run 的前端形态）：只规划不执行，返回 [源 → 目标] 列表。
/// 与 run_inner 共用 plan_one——预览与执行不可能分叉。
#[tauri::command]
pub fn plan_batch(args: BatchArgs) -> Result<Vec<serde_json::Value>, String> {
    let inputs: Vec<PathBuf> = args.inputs.iter().map(|p| PathBuf::from(&p.path)).collect();
    Ok(musicforge_cli::plan_only(
        &inputs,
        args.recursive,
        &args.template,
        args.out_dir.as_deref().map(Path::new),
    )
    .into_iter()
    .map(|i| {
        serde_json::json!({
            "source": i.source,
            "target": i.target,
            "format": i.format,
            "error": i.error,
        })
    })
    .collect())
}

/// 曲库扫描（P3 切片四）：只读扫描目录并分类，返回垃圾/孤立/命名异常清单。
///
/// - 与 core `scan_library` 同一实现（GUI/CLI 不可能分叉）；
/// - **只读**：不改动任何文件，也不写状态库（清洗执行与哈希缓存走 CLI，
///   破坏性操作必须有显式分级闸门，不藏在查看器里）；
/// - 目录不存在/不可读 → Err（显式失败，前端可见）。
/// P8 LibraryRefresher：库级**增量**重扫——扫描 + D17 增量哈希缓存刷新/入库。
///
/// - db 默认落在本地配置目录（D16：状态库严禁网络挂载；`Db::open` 内建校验）；
/// - 二次刷新：size+mtime 命中的文件零读取（成本只剩元数据遍历）；
/// - 唯一写入 = 状态库（可再生缓存），音乐文件只读。
#[tauri::command]
pub fn refresh_library(dir: String, state_db: Option<String>) -> Result<serde_json::Value, String> {
    let db_path = match state_db.as_deref().map(str::trim) {
        Some(p) if !p.is_empty() => std::path::PathBuf::from(p),
        _ => musicforge_core::db::local_config_dir().join("library.db"),
    };
    let db = musicforge_core::db::Db::open(&db_path).map_err(|e| e.to_string())?;
    let r = musicforge_core::scan::refresh_library(
        &db,
        std::path::Path::new(&dir),
        &musicforge_core::scan::ScanOptions::default(),
    )
    .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "dir": dir,
        "dbPath": db_path.display().to_string(),
        "scannedFiles": r.scanned_files,
        "scannedDirs": r.scanned_dirs,
        "audio": r.audio,
        "cacheHits": r.cache_hits,
        "hashed": r.hashed,
        "skipped": r.skipped,
    }))
}

/// 模板预览：给定模板 + 示例元数据 → 返回渲染后的文件名
/// 预览与真实执行共用同一渲染器（P4 语义）——预览所见即所得。
#[tauri::command]
pub fn scan_library(dir: String, recursive: bool) -> Result<serde_json::Value, String> {
    let report = musicforge_core::scan::scan_library(
        Path::new(&dir),
        &musicforge_core::scan::ScanOptions {
            recursive,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "dir": dir,
        "scannedFiles": report.scanned_files,
        "scannedDirs": report.scanned_dirs,
        "summary": {
            "audio": report.audio,
            "lyrics": report.lyrics,
            "covers": report.covers,
            "junk": report.junk,
            "other": report.other,
            "emptyDirs": report.empty_dirs.len(),
        },
        "ruleHits": report.rule_hits.iter().map(|(id, n)| serde_json::json!({
            "id": id,
            "count": n,
            "description": musicforge_core::scan::rule_card(id)
                .map(|c| c.description)
                .unwrap_or(""),
            "risk": format!(
                "{:?}",
                musicforge_core::scan::rule_card(id).map(|c| c.risk).unwrap_or(
                    musicforge_core::scan::Risk::Medium
                )
            )
            .to_lowercase(),
        })).collect::<Vec<_>>(),
        "items": report.items.iter().map(|i| serde_json::json!({
            "path": i.path.display().to_string(),
            "category": format!("{:?}", i.category).to_lowercase(),
            "rule": i.rule_id,
            "size": i.size,
        })).collect::<Vec<_>>(),
    }))
}

/// 去重扫描（P4.5 重复组视图）：只读，返回 exact 组 + 同名候选。
///
/// 组内对比数据随行（保留建议 + 评分明细 + 牺牲理由）——「建议保留」
/// 由前端高亮，「人工改选」由前端 radio 完成（改选结果经 `dedupe_apply`
/// 提交，服务端强校验）。
#[tauri::command]
pub fn dedupe_scan(dir: String) -> Result<serde_json::Value, String> {
    use musicforge_core::dedupe::{dedupe_scan, DedupeOptions};
    let report = dedupe_scan(
        Path::new(&dir),
        &DedupeOptions::default(),
        None, // GUI 扫描不接状态库（哈希即时计算；CLI 大库场景才用缓存）
    )
    .map_err(|e| e.to_string())?;
    let groups: Vec<serde_json::Value> = report
        .groups
        .iter()
        .map(|g| {
            let (mr, md) = (
                g.files
                    .iter()
                    .map(|f| f.score.sample_rate)
                    .max()
                    .unwrap_or(0),
                g.files.iter().map(|f| f.score.bit_depth).max().unwrap_or(0),
            );
            let keep = g.keep();
            serde_json::json!({
                "sha256": g.sha256,
                "size": g.size,
                "keep": {
                    "path": keep.path.display().to_string(),
                    "score": g.score_of(keep),
                    "detail": keep.score.detail(mr, md),
                },
                "sacrifices": g.sacrifices().iter().map(|f| serde_json::json!({
                    "path": f.path.display().to_string(),
                    "score": g.score_of(f),
                    "reason": g.sacrifice_reason(f),
                })).collect::<Vec<_>>(),
                "all": g.files.iter().map(|f| serde_json::json!({
                    "path": f.path.display().to_string(),
                    "score": g.score_of(f),
                    "size": f.size,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let same_name: Vec<serde_json::Value> = report
        .same_name
        .iter()
        .map(|g| {
            let keep = g.keep();
            serde_json::json!({
                "stem": g.stem,
                "keep": { "path": keep.path.display().to_string(), "score": g.score_of(keep) },
                "candidates": g.files.iter().enumerate().filter(|(i, _)| *i != g.keep_index)
                    .map(|(_, f)| serde_json::json!({
                        "path": f.path.display().to_string(),
                        "score": g.score_of(f),
                        "reason": g.candidate_reason(f),
                    })).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(serde_json::json!({
        "dir": dir,
        "filesSeen": report.files_seen,
        "hashedNow": report.hashed_now,
        "skipped": report.skipped,
        "groups": groups,
        "sameName": same_name,
    }))
}

/// 执行用户改选后的去重（GUI 分级闸门）：
///
/// - 前端提交**最终牺牲清单**（改选后的非保留成员）；
/// - 服务端逐条强校验：路径存在、是文件、且 **canonical 化后必须位于
///   `dir` 之内**——防 `../` 逃逸与符号链接跳板（威胁模型 T2）；
/// - 走 P3 回收站机制（rollback.jsonl），可经 `clean --restore` 整体还原；
/// - 绝不直接删除。
#[tauri::command]
pub fn dedupe_apply(
    dir: String,
    sacrifice_paths: Vec<String>,
) -> Result<serde_json::Value, String> {
    let dir_canon =
        std::fs::canonicalize(Path::new(&dir)).map_err(|e| format!("曲库目录不可读: {e}"))?;
    let mut actions = Vec::new();
    for p in &sacrifice_paths {
        let path = PathBuf::from(p);
        let canon = std::fs::canonicalize(&path).map_err(|e| format!("路径不可读 {p}: {e}"))?;
        if !canon.is_file() {
            return Err(format!("牺牲项不是文件: {p}"));
        }
        if !canon.starts_with(&dir_canon) {
            return Err(format!(
                "安全拒绝：牺牲项 {p} 位于曲库目录之外（已记录并阻止）"
            ));
        }
        // action 用 canonical 路径：apply_clean_plan 按 strip_prefix(scan_root)
        // 计算回收站内相对结构——两侧必须同一规范化形态（Windows 大小写差异
        // 会让非 canonical 路径失配，导致 rename 退化为自移动）
        actions.push(musicforge_core::scan::CleanAction {
            path: canon,
            rule_id: "MF-DUP-EXACT",
        });
    }
    let plan = musicforge_core::scan::CleanPlan {
        actions,
        empty_dirs: Vec::new(),
        trash_root: dir_canon.join(".musicforge/trash"),
        scan_root: dir_canon.clone(),
    };
    let to_move = plan.actions.len();
    let task_id = musicforge_cli::manifest::new_task_id();
    let outcome =
        musicforge_core::scan::apply_clean_plan(&plan, &task_id).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "requested": to_move,
        "moved": outcome.moved,
        "rollback": outcome.rollback_manifest.as_ref().map(|p| p.display().to_string()),
    }))
}
