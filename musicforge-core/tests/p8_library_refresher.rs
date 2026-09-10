//! P8 LibraryRefresher：库级增量重扫（扫描 + D17 增量哈希缓存刷新/入库）。
//!
//! 核心不变量：**二次刷新 = 缓存命中上升、重算归零**（size+mtime 未变的文件
//! 零文件读取）；扫描结论与全量扫描一致；唯一写入 = 状态库（音乐文件只读）。

use musicforge_core::db::Db;
use musicforge_core::scan::{refresh_library, scan_library, ScanOptions};

fn build_lib(root: &std::path::Path, n: usize) {
    for d in 0..3 {
        let dir = root.join(format!("album_{d:02}"));
        std::fs::create_dir_all(&dir).unwrap();
        for f in 0..n {
            std::fs::write(
                dir.join(format!("track_{f:02}.flac")),
                format!("fLaC-{d}-{f}"),
            )
            .unwrap();
        }
    }
}

#[test]
fn library_refresh_is_incremental_second_run() {
    let root = tempfile::tempdir().unwrap();
    let db_dir = tempfile::tempdir().unwrap();
    build_lib(root.path(), 4);
    let db_path = db_dir.path().join("library.db");
    let db = Db::open(&db_path).unwrap();

    let first = refresh_library(&db, root.path(), &ScanOptions::default()).unwrap();
    assert_eq!(first.audio, 12, "12 个音频文件");
    assert_eq!(first.hashed, 12, "首轮全部重算");
    assert_eq!(first.cache_hits, 0);

    // 二次刷新：文件未变 → 全命中，零重算
    let second = refresh_library(&db, root.path(), &ScanOptions::default()).unwrap();
    assert_eq!(second.audio, 12, "扫描结论不变");
    assert_eq!(second.cache_hits, 12, "二次全命中缓存");
    assert_eq!(second.hashed, 0, "二次零重算（零文件读取）");
}

#[test]
fn library_refresh_rehashes_only_changed_files() {
    let root = tempfile::tempdir().unwrap();
    let db_dir = tempfile::tempdir().unwrap();
    build_lib(root.path(), 2);
    let db_path = db_dir.path().join("library.db");
    let db = Db::open(&db_path).unwrap();
    let _ = refresh_library(&db, root.path(), &ScanOptions::default()).unwrap();

    // 改动 1 个文件（内容+大小变化 → mtime/size 失配）
    std::fs::write(
        root.path().join("album_00/track_00.flac"),
        b"fLaC-CHANGED-LONGER",
    )
    .unwrap();
    let third = refresh_library(&db, root.path(), &ScanOptions::default()).unwrap();
    assert_eq!(third.cache_hits, 5, "未变的 5 个命中");
    assert_eq!(third.hashed, 1, "仅变更的 1 个重算");
}

#[test]
fn library_refresh_matches_full_scan_conclusion() {
    let root = tempfile::tempdir().unwrap();
    let db_dir = tempfile::tempdir().unwrap();
    build_lib(root.path(), 3);
    // 混入垃圾与非音频（扫描结论需与 scan_library 一致）
    std::fs::write(root.path().join("Thumbs.db"), b"").unwrap();
    std::fs::write(root.path().join("cover.jpg"), b"").unwrap();
    let db_path = db_dir.path().join("library.db");
    let db = Db::open(&db_path).unwrap();

    let full = scan_library(root.path(), &ScanOptions::default()).unwrap();
    let r = refresh_library(&db, root.path(), &ScanOptions::default()).unwrap();
    assert_eq!(r.scanned_files, full.scanned_files, "文件数一致");
    assert_eq!(r.scanned_dirs, full.scanned_dirs, "目录数一致");
    assert_eq!(r.audio, full.audio, "音频数一致");
}
