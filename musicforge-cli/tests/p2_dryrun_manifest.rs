//! v0.2.0 安全任务层（切片一）：dry-run 与 manifest 留痕。
//!
//! 断言的是外部可观察行为：
//! - `--dry-run`（dry_run=true）：**零写入**（输出目录不出现任何音频与侧车），
//!   仅产出 manifest 计划条目；
//! - 正常执行：manifest 逐条记录结果，成功条目带产物 sha256 与适配器 id。

use std::path::{Path, PathBuf};

use musicforge_cli::{run, BatchConfig};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../musicforge-core/tests/fixtures")
}

fn cfg(inputs: Vec<PathBuf>, out: &Path, dry_run: bool, manifest: Option<PathBuf>) -> BatchConfig {
    BatchConfig {
        inputs,
        out_dir: Some(out.to_path_buf()),
        recursive: false,
        skip_existing: false,
        jobs: 2,
        template: "{title}".to_string(),
        cancel: None,
        dry_run,
        manifest,
    }
}

fn read_jsonl(path: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .expect("manifest 可读")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("每行是合法 JSON"))
        .collect()
}

/// dry-run：不落盘，只留痕计划（planned）。
#[test]
fn dry_run_writes_nothing_but_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let manifest = tmp.path().join("dry.jsonl");

    let summary = run(cfg(vec![fixtures()], &out, true, Some(manifest.clone())));

    assert_eq!(summary.ok, 0, "dry-run 不应产生成功转换");
    assert_eq!(summary.planned, 7, "7 个 fixture 应被规划");
    assert_eq!(summary.exit_code(), 0, "dry-run 成功退出码为 0");
    assert!(
        !out.exists()
            || std::fs::read_dir(&out)
                .map(|mut d| d.next().is_none())
                .unwrap_or(true),
        "dry-run 不得写出任何文件，实际输出目录: {out:?}"
    );

    let lines = read_jsonl(&manifest);
    assert_eq!(lines.len(), 8, "1 行任务头 + 7 条计划");
    assert_eq!(lines[0]["schema_version"], 1, "manifest schema_version=1");
    assert!(lines[0]["task_id"].as_str().is_some(), "任务头需带 task_id");
    for item in &lines[1..] {
        assert_eq!(item["result"], "planned");
        assert!(item["target"].as_str().is_some(), "计划条目需带目标路径");
    }
}

/// 正常执行：manifest 逐条记录成功结果与产物 sha256、适配器 id。
#[test]
fn apply_writes_manifest_with_success_items() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let manifest = tmp.path().join("apply.jsonl");

    let summary = run(cfg(vec![fixtures()], &out, false, Some(manifest.clone())));
    assert_eq!(summary.ok, 7, "7 个 fixture 全部成功");
    assert_eq!(summary.failed, 0);

    let lines = read_jsonl(&manifest);
    assert_eq!(lines.len(), 8, "1 行任务头 + 7 条结果");
    let items: Vec<&serde_json::Value> =
        lines.iter().filter(|v| v.get("result").is_some()).collect();
    assert_eq!(items.len(), 7);
    for item in items {
        assert_eq!(item["result"], "success");
        assert!(
            item["target_sha256"].as_str().is_some(),
            "成功条目需记录产物 sha256（审计留痕）"
        );
        assert_eq!(item["adapter"], "ncm");
        assert_eq!(
            item["rollback_available"], false,
            "undo 尚未实现，不得谎报可回滚"
        );
    }
}
