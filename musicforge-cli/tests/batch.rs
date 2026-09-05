//! 集成测试：批处理语义（有界并发/跳过增量/失败隔离/结构保留/退出码语义）

use std::path::{Path, PathBuf};

use musicforge_cli::{run, BatchConfig, Status};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../musicforge-core/tests/fixtures")
}

fn cfg(inputs: Vec<PathBuf>, out: &Path) -> BatchConfig {
    BatchConfig {
        inputs,
        out_dir: Some(out.to_path_buf()),
        recursive: false,
        skip_existing: false,
        jobs: 2,
        template: "{title}".to_string(),
        cancel: None,
    }
}

/// 全部成功 + 输出逐文件存在 + 退出码 0
#[test]
fn batch_all_ok() {
    let tmp = tempfile::tempdir().unwrap();
    let summary = run(cfg(vec![fixtures()], tmp.path()));
    assert_eq!(summary.ok, 7, "7 个 fixture 全部成功");
    assert_eq!(summary.failed, 0);
    assert_eq!(summary.exit_code(), 0);
    // 每个成功项的输出文件确实存在
    for r in &summary.results {
        assert!(r.output.as_ref().unwrap().exists(), "输出应存在: {:?}", r.output);
    }
}

/// 增量跳过：第二次运行（skip_existing）全部 Skipped
#[test]
fn batch_skip_existing_incremental() {
    let tmp = tempfile::tempdir().unwrap();
    let inputs = vec![fixtures()];
    let first = run(cfg(inputs.clone(), tmp.path()));
    assert_eq!(first.ok, 7);

    let mut second_cfg = cfg(inputs, tmp.path());
    second_cfg.skip_existing = true;
    let second = run(second_cfg);
    assert_eq!(second.skipped, 7, "第二次全部跳过");
    assert_eq!(second.ok, 0);
}

/// 失败隔离：损坏文件只影响自己，其余成功；失败原因带错误码
#[test]
fn batch_failure_isolation() {
    let tmp = tempfile::tempdir().unwrap();
    let src_dir = tempfile::tempdir().unwrap();

    // 3 个好文件 + 1 个 CRC 篡改坏文件
    let good = ["flac_with_cover.ncm", "mp3_raw_no_id3.ncm", "no_cover.ncm"];
    for g in good {
        std::fs::copy(fixtures().join(g), src_dir.path().join(g)).unwrap();
    }
    let mut bad = std::fs::read(fixtures().join("flac_with_cover.ncm")).unwrap();
    bad[100] ^= 0xff;
    std::fs::write(src_dir.path().join("__bad.ncm"), &bad).unwrap();

    let summary = run(cfg(vec![src_dir.path().to_path_buf()], tmp.path()));
    assert_eq!(summary.ok, 3, "3 个好文件成功");
    assert_eq!(summary.failed, 1, "1 个坏文件失败");
    assert_eq!(summary.exit_code(), 1);

    let failed = summary.results.iter().find(|r| r.status == Status::Failed).unwrap();
    let reason = failed.reason.as_ref().unwrap();
    assert!(reason.contains("NCM-CRC-MISMATCH"), "失败原因应含错误码: {reason}");
    assert!(reason.contains("建议"), "失败原因应含可操作建议（repair receipt）");
    // 其余 3 个输出不受影响
    for r in &summary.results {
        if r.status == Status::Ok {
            assert!(r.output.as_ref().unwrap().exists());
        }
    }
}

/// 结构保留：递归导入嵌套目录 → 输出目录树映射（修上游 C-N7 平铺覆盖）
#[test]
fn batch_recursive_preserves_structure() {
    let out = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();

    let sub = src.path().join("2024/专辑A");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::copy(fixtures().join("flac_with_cover.ncm"), sub.join("song.ncm")).unwrap();
    std::fs::copy(fixtures().join("flac_with_cover.ncm"), src.path().join("dup_name.ncm")).unwrap();

    let cfg = BatchConfig {
        inputs: vec![src.path().to_path_buf()],
        out_dir: Some(out.path().to_path_buf()),
        recursive: true,
        skip_existing: false,
        jobs: 2,
        template: "{title}".to_string(),
        cancel: None,
    };
    let summary = run(cfg);
    assert_eq!(summary.ok, 2, "2 个文件（嵌套 + 根）都成功");
    // 结构保留：嵌套文件的目录映射到输出（文件名由模板渲染：title=测试曲目）
    assert!(
        out.path().join("2024/专辑A/测试曲目.flac").exists(),
        "子目录结构应保留（文件名由模板渲染）"
    );
    // 根目录文件渲染名与嵌套文件同前缀但不同目录 → 各自独立落盘（不覆盖）
    assert!(out.path().join("测试曲目.flac").exists(), "根文件按模板渲染落盘");
}

/// 有界并发：大量文件（100 个）全部成功且失败不传染（压力小样）
#[test]
fn batch_bulk_files() {
    let out = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    for i in 0..100 {
        std::fs::copy(
            fixtures().join("mp3_with_id3.ncm"),
            src.path().join(format!("t{i:03}.ncm")),
        )
        .unwrap();
    }
    let cfg = BatchConfig {
        inputs: vec![src.path().to_path_buf()],
        out_dir: Some(out.path().to_path_buf()),
        recursive: false,
        skip_existing: false,
        jobs: 4,
        template: "{title}".to_string(),
        cancel: None,
    };
    let summary = run(cfg);
    assert_eq!(summary.ok, 100);
    assert_eq!(summary.failed, 0);
}


/// 命名模板：目录段 + track 零填充 + 清洗（全生态空白功能）
#[test]
fn batch_template_dirs_and_padding() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = BatchConfig {
        inputs: vec![fixtures()],
        out_dir: Some(tmp.path().to_path_buf()),
        recursive: false,
        skip_existing: false,
        jobs: 2,
        template: "{artist}/{album}/{track:02d} {title}".to_string(),
        cancel: None,
    };
    let summary = run(cfg);
    assert_eq!(summary.ok, 7);
    // flac_with_cover：测试歌手/测试专辑/00 测试曲目.flac（fixture 无 track → 00）
    assert!(
        tmp.path().join("测试歌手/测试专辑/00 测试曲目.flac").exists(),
        "模板目录结构 + 零填充 track 应生效"
    );
}

/// 命名模板：非法字符清洗端到端（metadata 含 <>:?*"/ 的曲目名）
#[test]
fn batch_template_sanitizes_illegal_chars_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = BatchConfig {
        inputs: vec![fixtures()],
        out_dir: Some(tmp.path().to_path_buf()),
        recursive: false,
        skip_existing: false,
        jobs: 2,
        template: "{title}.flac".to_string(),
        cancel: None,
    };
    let summary = run(cfg);
    assert_eq!(summary.ok, 7);
    let has_illegal = summary
        .results
        .iter()
        .filter_map(|r| r.output.as_ref())
        .any(|o| {
            o.file_name()
                .and_then(|f| f.to_str())
                .map(|f| f.contains(['<', '>', ':', '"', '/', '|', '?', '*']))
                .unwrap_or(false)
        });
    assert!(!has_illegal, "输出文件名不得含非法字符");
}


/// 目标名去重（专测）：同目录同元数据两文件 → 渲染名碰撞 → " (2)" 后缀，不覆盖
#[test]
fn batch_dedup_same_dir_collision() {
    let out = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    std::fs::copy(fixtures().join("flac_with_cover.ncm"), src.path().join("a.ncm")).unwrap();
    std::fs::copy(fixtures().join("flac_with_cover.ncm"), src.path().join("b.ncm")).unwrap();

    let cfg = BatchConfig {
        inputs: vec![src.path().to_path_buf()],
        out_dir: Some(out.path().to_path_buf()),
        recursive: false,
        skip_existing: false,
        jobs: 2,
        template: "{title}".to_string(),
        cancel: None,
    };
    let summary = run(cfg);
    assert_eq!(summary.ok, 2, "两个文件都成功");
    assert!(out.path().join("测试曲目.flac").exists(), "第一个占用渲染名");
    assert!(out.path().join("测试曲目 (2).flac").exists(), "第二个自动去重");
    // 两个输出内容都完整（各自对应的源解密结果一致）
    let s1 = std::fs::read(out.path().join("测试曲目.flac")).unwrap().len();
    let s2 = std::fs::read(out.path().join("测试曲目 (2).flac")).unwrap().len();
    assert_eq!(s1, s2, "去重文件内容长度一致");
}

/// 取消语义：jobs=1 串行处理，回调里在第 3 个完成后触发取消 → 3 成功 + 97 取消
#[test]
fn batch_cancel_token() {
    use musicforge_cli::CancelToken;
    let out = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    for i in 0..100 {
        std::fs::copy(fixtures().join("mp3_with_id3.ncm"), src.path().join(format!("t{i:03}.ncm"))).unwrap();
    }
    let token = CancelToken::new();
    let cfg = BatchConfig {
        inputs: vec![src.path().to_path_buf()],
        out_dir: Some(out.path().to_path_buf()),
        recursive: false,
        skip_existing: false,
        jobs: 1,
        template: "{title}".to_string(),
        cancel: Some(token.clone()),
    };
    let seen = std::sync::atomic::AtomicUsize::new(0);
    let summary = musicforge_cli::run_with_progress(cfg, |_r| {
        let n = seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n >= 2 {
            token.cancel(); // 第 3 个结果回调时取消（其后 97 个不再开工）
        }
    });
    assert_eq!(summary.ok, 3, "取消前已开工的 3 个完成");
    assert_eq!(summary.cancelled, 97, "其余标记为取消");
    assert_eq!(summary.failed, 0);
}

/// #11 回归：规划阶段不再创建输出目录。
///
/// 取消令牌**预置为已取消** → 全部文件走 `execute_one` 顶部的 Cancelled 分支，
/// 执行阶段从不落盘。修复前 `plan_one` 在规划期 `create_dir_all`，会给
/// Skipped / Cancelled 文件在磁盘留下空的残留目录。本测试断言：`run` 结束后
/// 多级输出目录 `a/b/c` 依旧不存在（规划对文件系统无副作用）。
#[test]
fn plan_does_not_leave_residual_output_dir() {
    use musicforge_cli::CancelToken;
    let out = tempfile::tempdir().unwrap();
    let nested = out.path().join("a").join("b").join("c"); // 多级、尚不存在
    assert!(!nested.exists(), "前置：输出目录不应已存在");

    let token = CancelToken::new();
    token.cancel(); // 预置为已取消
    let cfg = BatchConfig {
        inputs: vec![fixtures()],
        out_dir: Some(nested.clone()),
        recursive: false,
        skip_existing: false,
        jobs: 2,
        template: "{title}".to_string(),
        cancel: Some(token),
    };
    let summary = run(cfg);
    assert!(summary.cancelled > 0, "全部文件应被取消（一个都未开工）");
    assert!(
        !nested.exists(),
        "#11 回归失败：规划阶段不应创建输出目录（残留空目录 {}）",
        nested.display()
    );
}

/// F2 回归：sidecar `.musicforge.json` 不得写入源文件绝对路径（隐私）。
/// 转换 fixture → 读出 sidecar → 断言无 `source` 键、且不含源目录绝对路径痕迹。
/// 关键依据：`integrity_marker_ok` 只读取 `size`+`sha256`，从不读取 `source`，
/// 故删除该字段对 `--skip-existing` 命中逻辑零副作用。
#[test]
fn sidecar_must_not_contain_absolute_source() {
    let out = tempfile::tempdir().unwrap();
    let summary = run(cfg(vec![fixtures()], out.path()));
    assert_eq!(summary.ok, 7, "fixture 全部成功");

    for r in &summary.results {
        if r.status != Status::Ok {
            continue;
        }
        let out_path = r.output.as_ref().unwrap();
        let sidecar = PathBuf::from(format!("{}.musicforge.json", out_path.display()));
        assert!(sidecar.exists(), "sidecar 应存在: {:?}", sidecar);
        let text = std::fs::read_to_string(&sidecar).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(
            v.get("source").is_none(),
            "F2 回归失败：sidecar 仍含 source 字段（隐私泄漏）: {text}"
        );
        // 双重保险：文本层面也不应出现源 fixture 目录的绝对路径痕迹
        assert!(
            !text.contains("fixtures") && !text.contains("C:\\") && !text.contains("/Users/"),
            "F2 回归失败：sidecar 含绝对路径痕迹: {text}"
        );
    }
}

/// F3 回归：collect_inputs 不跟随符号链接（防越界遍历 + 自引用 junction 重复计数）。
/// Windows 下创建符号链接需提权（开发者模式/管理员），故仅在 unix 上实跑；
/// Windows 编译期排除，由 CI（Linux）提供回归网。
#[cfg(unix)]
#[test]
fn collect_inputs_skips_symlinks() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(fixtures().join("no_cover.ncm"), dir.path().join("real.ncm")).unwrap();
    let link = dir.path().join("link.ncm");
    symlink(dir.path().join("real.ncm"), &link).unwrap();

    let collected = musicforge_cli::collect_inputs(&[dir.path().to_path_buf()], false);
    let names: Vec<String> = collected
        .iter()
        .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"real.ncm".to_string()), "应收集真实文件");
    assert!(
        !names.contains(&"link.ncm".to_string()),
        "F3 回归失败：不应跟随符号链接收集 link.ncm"
    );
}

// ============ P1a 保护网：退出码语义 + 失败清单 CSV 字段兼容 ============
//
// 依据：`Summary::exit_code()` 的实际语义是「failed > 0 → 1，否则 0」
//（musicforge-cli/src/lib.rs::BatchSummary::exit_code）。Skipped / Cancelled
// 都**不**触发非零退出码。以下三条把这一契约独立钉死，重构时改坏退出码
// 语义会立即在这里报红，而不是等 CI 集成才发现。

/// 全成功批处理 → exit_code == 0
#[test]
fn exit_code_all_success_is_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let summary = run(cfg(vec![fixtures()], tmp.path()));
    assert_eq!(summary.ok, 7);
    assert_eq!(summary.failed, 0);
    assert_eq!(summary.exit_code(), 0, "全部成功必须是 0");
}

/// 含失败批处理 → exit_code != 0（且失败清单一行不落盘地给出错误码）
#[test]
fn exit_code_any_failure_is_nonzero() {
    let tmp = tempfile::tempdir().unwrap();
    let src_dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        fixtures().join("flac_with_cover.ncm"),
        src_dir.path().join("good.ncm"),
    )
    .unwrap();
    let mut bad = std::fs::read(fixtures().join("flac_with_cover.ncm")).unwrap();
    bad[100] ^= 0xff;
    std::fs::write(src_dir.path().join("__bad.ncm"), &bad).unwrap();

    let summary = run(cfg(vec![src_dir.path().to_path_buf()], tmp.path()));
    assert_eq!(summary.failed, 1, "1 个坏文件失败");
    assert_ne!(
        summary.exit_code(),
        0,
        "只要有失败，退出码就必须非零（脚本/CI 依赖这一点判断成败）"
    );
}

/// 失败清单 CSV 字段兼容：表头必须是 `source,code,reason`，且 code 列承载错误码。
/// GUI 的 `save_failures` 复用同一实现，改这里会同时破坏两端。
#[test]
fn failures_csv_header_is_stable() {
    let tmp = tempfile::tempdir().unwrap();
    let src_dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        fixtures().join("flac_with_cover.ncm"),
        src_dir.path().join("good.ncm"),
    )
    .unwrap();
    let mut bad = std::fs::read(fixtures().join("flac_with_cover.ncm")).unwrap();
    bad[100] ^= 0xff;
    std::fs::write(src_dir.path().join("__bad.ncm"), &bad).unwrap();

    let summary = run(cfg(vec![src_dir.path().to_path_buf()], tmp.path()));
    assert_eq!(summary.failed, 1);

    let csv_path = tmp.path().join("failures.csv");
    summary
        .export_failures_csv(&csv_path)
        .expect("失败清单导出必须成功");

    let text = std::fs::read_to_string(&csv_path).unwrap();
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("source,code,reason"),
        "CSV 表头字段名或顺序发生变化 —— 这是脚本/前端共同依赖的稳定契约"
    );

    // 失败行的 code 列必须承载错误码（repair receipt 的机器可读部分）
    let row = lines.next().expect("必须有一行失败记录");
    assert!(
        row.contains("NCM-CRC-MISMATCH"),
        "code 列应含错误码，实际行: {row}"
    );

    // 只有失败项入列：成功项不得出现
    assert!(
        !text.contains("good.ncm"),
        "成功文件不应出现在失败清单里: {text}"
    );
}
