//! v0.2.0 状态层（D16）：SQLite 可再生缓存 + 历史。
//!
//! 断言围绕三条铁律：
//! 1. 真相在文件系统与 manifest——db 只做缓存/历史；
//! 2. **降级版本拒绝打开**（不猜、不静默重建）；
//! 3. **只能放本地配置目录**，网络挂载直接报错（X16）。

use musicforge_core::db::{default_db_path, ensure_local_db_path, Db, SCHEMA_VERSION};
use musicforge_core::NcmError;
use std::path::{Path, PathBuf};

fn temp_db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("musicforge-db-test-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("library.db")
}

/// 打开即建表；文件索引可写入并读回（增量扫描的缓存依据）。
#[test]
fn open_creates_schema_and_roundtrips_file_row() {
    let path = temp_db("roundtrip");
    let db = Db::open(&path).unwrap();

    db.upsert_file(
        "C:/music/a.ncm",
        1234,
        Some(42),
        Some("flac"),
        Some("deadbeef"),
    )
    .unwrap();
    let row = db.get_file("C:/music/a.ncm").unwrap().expect("应读回该行");
    assert_eq!(row.size, 1234);
    assert_eq!(row.mtime, Some(42));
    assert_eq!(row.format.as_deref(), Some("flac"));
    assert_eq!(row.sha256.as_deref(), Some("deadbeef"));

    // 覆盖更新（同一路径）
    db.upsert_file("C:/music/a.ncm", 9999, Some(43), Some("mp3"), None)
        .unwrap();
    let row = db.get_file("C:/music/a.ncm").unwrap().unwrap();
    assert_eq!(row.size, 9999);
    assert_eq!(row.sha256, None);
    assert_eq!(db.stats().unwrap().0, 1, "同路径应覆盖而非新增");

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// 任务历史 + ack 留痕。
#[test]
fn task_and_ack_records() {
    let db = Db::open_in_memory().unwrap();
    db.start_task("t-1", "convert", "2026-09-06T00:00:00Z")
        .unwrap();
    db.finish_task("t-1", "2026-09-06T00:01:00Z", 7, 0).unwrap();
    assert_eq!(db.stats().unwrap().1, 1);

    assert!(!db.has_ack("plugin.qmc").unwrap());
    db.set_ack("plugin.qmc").unwrap();
    assert!(db.has_ack("plugin.qmc").unwrap());
}

/// 高于当前 schema 版本的库：**拒绝打开**（降级不猜）。
#[test]
fn newer_schema_is_refused() {
    let path = temp_db("newer");
    {
        let db = Db::open(&path).unwrap();
        db.upsert_file("x", 1, None, None, None).unwrap();
    }
    // 人为把 user_version 抬高
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
    }
    // 注意：Db 未实现 Debug（rusqlite::Connection 不支持），故用 match 而非 unwrap_err
    match Db::open(&path) {
        Err(NcmError::Db(msg)) => assert!(msg.contains("拒绝打开"), "错误信息应说明拒绝: {msg}"),
        other => panic!("应返回 Db 错误，实际: {:?}", other.is_ok()),
    }
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// X16 位置铁律：网络挂载（UNC）直接拒绝。
#[test]
fn network_db_path_is_rejected() {
    let unc = Path::new(r"\\nas\music\library.db");
    let err = ensure_local_db_path(unc).unwrap_err();
    match err {
        NcmError::Db(msg) => assert!(msg.contains("网络位置"), "应说明网络挂载风险: {msg}"),
        other => panic!("应为 Db 错误，实际: {other:?}"),
    }
    assert!(ensure_local_db_path(Path::new("C:/tmp/library.db")).is_ok());
}

/// 默认位置必须落在本地配置目录，而不是任何音乐目录。
#[test]
fn default_path_is_local_config_dir() {
    let p = default_db_path();
    let s = p.to_string_lossy().to_string();
    assert!(!s.starts_with(r"\\"), "默认位置不得是网络路径: {s}");
    if cfg!(windows) {
        assert!(
            s.contains("MusicForge"),
            "Windows 应在 MusicForge 配置目录: {s}"
        );
    } else {
        assert!(
            s.contains("musicforge"),
            "unix 应在 musicforge 配置目录: {s}"
        );
    }
    assert!(p.file_name().unwrap() == "library.db");
}
