//! P3 切片三：D17 增量哈希缓存接线（scan × db）。
//!
//! 验收口径（对齐 ROADMAP D17）：首扫建缓存（全量重算并回写），
//! 二扫全命中（零重算、零文件读取）；文件变化（size/mtime 任一变）
//! 后仅重算该文件；mtime 不可得退化为占位索引。

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use musicforge_core::db::Db;
use musicforge_core::scan::{refresh_hash_cache, scan_library, Category, ScanItem, ScanOptions};

fn uniq_root(tag: &str) -> std::path::PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("mf-d17-{tag}-{n}-{}-{}", seq, std::process::id()))
}

/// 2 音频 + 1 歌词 + 1 垃圾（均为自建临时内容，无版权材料）。
fn build_tree(root: &Path) {
    std::fs::create_dir_all(root.join("a")).unwrap();
    std::fs::write(root.join("a").join("one.flac"), b"fLaC-one-payload").unwrap();
    std::fs::write(root.join("a").join("two.mp3"), b"ID3-two-payload").unwrap();
    std::fs::write(root.join("a").join("one.lrc"), b"[00:01]x").unwrap();
    std::fs::write(root.join("Thumbs.db"), b"x").unwrap();
}

#[test]
fn scan_items_carry_mtime() {
    let root = uniq_root("mtime");
    build_tree(&root);
    let report = scan_library(&root, &ScanOptions::default()).unwrap();
    assert_eq!(report.audio, 2);
    let audio: Vec<_> = report
        .items
        .iter()
        .filter(|i| i.category == Category::Audio)
        .collect();
    assert!(
        audio.iter().all(|i| i.mtime.is_some()),
        "audio 行必须带 mtime（D17 缓存键的一半）"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn hash_cache_first_hash_then_all_hits() {
    let root = uniq_root("inc");
    build_tree(&root);
    let db = Db::open_in_memory().unwrap();
    let report = scan_library(&root, &ScanOptions::default()).unwrap();

    let st1 = refresh_hash_cache(&db, &report.items);
    assert_eq!(
        (st1.considered, st1.cache_hits, st1.hashed, st1.skipped),
        (2, 0, 2, 0),
        "首扫：全量重算并回写"
    );

    // 回写值必须与独立重算一致（不信任缓存写入方自证）
    let one = root.join("a").join("one.flac");
    let row = db.get_file(one.to_str().unwrap()).unwrap().unwrap();
    let expect = musicforge_core::scan::sha256_file_stream(&one).unwrap();
    assert_eq!(row.sha256.as_deref(), Some(expect.as_str()));
    assert!(row.mtime.is_some());
    assert_eq!(row.format.as_deref(), Some("flac"));

    let st2 = refresh_hash_cache(&db, &report.items);
    assert_eq!(
        (st2.cache_hits, st2.hashed),
        (2, 0),
        "二扫：全命中零重算（D17 验收口径）"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn hash_cache_rehashes_only_changed_file() {
    let root = uniq_root("chg");
    build_tree(&root);
    let db = Db::open_in_memory().unwrap();
    let r1 = scan_library(&root, &ScanOptions::default()).unwrap();
    refresh_hash_cache(&db, &r1.items);

    // 改写 one.flac（内容与大小都变 → 缓存必 miss；two.mp3 未动 → 必命中）
    std::fs::write(
        root.join("a").join("one.flac"),
        b"fLaC-one-payload-CHANGED-LONGER",
    )
    .unwrap();
    let r2 = scan_library(&root, &ScanOptions::default()).unwrap();
    let st = refresh_hash_cache(&db, &r2.items);
    assert_eq!((st.cache_hits, st.hashed), (1, 1), "仅变化的文件重算");

    let one = root.join("a").join("one.flac");
    let row = db.get_file(one.to_str().unwrap()).unwrap().unwrap();
    let expect = musicforge_core::scan::sha256_file_stream(&one).unwrap();
    assert_eq!(
        row.sha256.as_deref(),
        Some(expect.as_str()),
        "缓存与新内容一致"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn mtime_none_degrades_to_placeholder_row() {
    let db = Db::open_in_memory().unwrap();
    let item = ScanItem {
        path: std::path::PathBuf::from("Z:/nonexistent/ghost.flac"),
        category: Category::Audio,
        rule_id: None,
        size: 5,
        mtime: None,
    };
    let st = refresh_hash_cache(&db, &[item]);
    assert_eq!(
        (st.considered, st.cache_hits, st.hashed, st.skipped),
        (1, 0, 0, 1),
        "mtime 不可得：不重算不命中，计入 skipped"
    );
    let row = db.get_file("Z:/nonexistent/ghost.flac").unwrap().unwrap();
    assert!(row.sha256.is_none(), "占位行不带哈希");
    assert!(row.mtime.is_none());
}

#[test]
fn non_audio_items_are_ignored() {
    let root = uniq_root("nonaudio");
    build_tree(&root);
    let db = Db::open_in_memory().unwrap();
    let report = scan_library(&root, &ScanOptions::default()).unwrap();
    let st = refresh_hash_cache(&db, &report.items);
    assert_eq!(st.considered, 2, "只有音频参与判定（歌词/垃圾不计）");
    std::fs::remove_dir_all(&root).ok();
}
