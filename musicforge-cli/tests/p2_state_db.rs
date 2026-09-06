//! v0.2.0 状态层（D16）CLI 集成：--state-db 写任务历史 + 源文件哈希缓存。

use std::path::PathBuf;
use std::process::Command;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../musicforge-core/tests/fixtures")
}

/// 断点续跑语义与缓存生效：同一库跑两遍，第二遍源哈希全部命中缓存
/// （以「第二次运行后行数不变、哈希列非空」验证）。
#[test]
fn state_db_records_task_and_source_hashes() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let db_path = tmp.path().join("library.db");

    // 第一次：转换全部 7 个 fixture（会为每个成功项写源行+产物行）
    let exe = env!("CARGO_BIN_EXE_musicforge");
    let st = Command::new(exe)
        .args([
            "-d",
            fixtures().to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--state-db",
            db_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(st.status.success(), "首次转换应成功");

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let tasks: i64 = conn
        .query_row("SELECT COUNT(1) FROM tasks", [], |r| r.get(0))
        .unwrap();
    assert!(tasks >= 1, "应有任务历史行");
    let rows_first: i64 = conn
        .query_row("SELECT COUNT(1) FROM files", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows_first, 14, "7 源行（带 mtime+sha）+ 7 产物行 = 14");
    let hashed: i64 = conn
        .query_row(
            "SELECT COUNT(1) FROM files WHERE sha256 IS NOT NULL AND mtime IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hashed, 7, "源行均应带 sha256+mtime（D17 缓存键）");

    // 第二次：--skip-existing 全部跳过 → 不应有任何新写入（缓存命中路径）
    let st2 = Command::new(exe)
        .args([
            "-d",
            fixtures().to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--skip-existing",
            "--state-db",
            db_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(st2.status.success(), "二次运行应成功（缓存命中）");

    let conn2 = rusqlite::Connection::open(&db_path).unwrap();
    let rows_after: i64 = conn2
        .query_row("SELECT COUNT(1) FROM files", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        rows_after, rows_first,
        "二次运行全部跳过，行数不应增长（覆盖更新而非新增）"
    );
}
