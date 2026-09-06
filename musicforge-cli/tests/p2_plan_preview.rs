//! v0.2.0 GUI 计划预览的 API 语义：plan_only 与 run_inner 共用 plan_one，
//! 预览与执行**不可能分叉**——本文件钉住预览的三条语义。

use std::path::PathBuf;

use musicforge_cli::plan_only;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../musicforge-core/tests/fixtures")
}

/// 正常：7 个 fixture 全部规划成功，目标扩展名与判定格式一致。
#[test]
fn plan_only_lists_all_fixtures_with_targets() {
    let items = plan_only(&[fixtures()], false, "{title}", None);
    assert_eq!(items.len(), 7);
    for it in &items {
        assert!(it.error.is_none(), "fixture 规划不应失败: {it:?}");
        let target = it.target.as_ref().expect("应有目标路径");
        assert!(
            target.ends_with(".flac") || target.ends_with(".mp3"),
            "目标应以判定格式结尾: {target:?}"
        );
    }
}

/// 失败的输入不中断整体规划：以带 error 的条目呈现（NCM-BAD-MAGIC 稳定码）。
#[test]
fn plan_only_surfaces_failures_without_aborting() {
    let tmp = tempfile::tempdir().unwrap();
    let junk = tmp.path().join("junk.ncm");
    std::fs::write(&junk, b"garbage data here").unwrap();

    let items = plan_only(&[PathBuf::from(&junk)], false, "{title}", None);
    assert_eq!(items.len(), 1);
    assert!(items[0].target.is_none(), "失败条目不应有目标路径");
    let err = items[0].error.as_deref().expect("应有错误说明");
    assert!(err.contains("NCM-BAD-MAGIC"), "应含稳定码，实际: {err}");
}

/// 同渲染名去重在预览与执行间一致：同一 fixture 输入两次，预览即出现 " (2)"。
#[test]
fn plan_only_dedup_matches_execution() {
    let f = fixtures().join("no_cover.ncm");
    let items = plan_only(&[f.clone(), f], false, "{title}", None);
    assert_eq!(items.len(), 2);
    let t0 = items[0].target.as_ref().unwrap().to_string();
    let t1 = items[1].target.as_ref().unwrap().to_string();
    assert_ne!(t0, t1, "同源两次输入必须去重");
    assert!(
        t1.contains(" (2)"),
        "第二个应追加 (2) 后缀，实际 t0={t0:?} t1={t1:?}"
    );
}
