//! P3 曲库扫描与清洗：扫描分类、规则命中、清洗执行与回滚。

use std::fs;
use std::path::{Path, PathBuf};

use musicforge_core::scan::{build_clean_plan, scan_library, CleanPlan, ScanOptions, ScanReport};

fn fixtures() -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "musicforge-p3-{}-{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        seq,
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// 真机实测发现的回归：`.musicforge/`（回收站等工具状态）必须对扫描不可见，
/// 否则去重把回收站副本当新组、organize 会搬走待还原文件。
#[test]
fn musicforge_convention_dir_is_invisible_to_scan() {
    let root = fixtures();
    let hidden = root.join(".musicforge").join("trash").join("t-1");
    fs::create_dir_all(&hidden).unwrap();
    fs::write(hidden.join("ghost.flac"), b"fLaC...").unwrap();
    fs::write(root.join("visible.flac"), b"fLaC...").unwrap();

    let report = scan_library(&root, &ScanOptions::default()).unwrap();
    assert_eq!(report.audio, 1, "只有 visible.flac 应被扫描到");
    assert!(
        !report
            .items
            .iter()
            .any(|i| i.path.to_string_lossy().contains(".musicforge")),
        "任何 .musicforge 内容不得出现在扫描结果"
    );
    fs::remove_dir_all(&root).ok();
}

/// 构造污染树：覆盖全部 9 条规则的触发条件。
fn polluted() -> PathBuf {
    let root = fixtures();
    let audio_dir = root.join("audio");
    let empty_dir = root.join("empty");
    fs::create_dir_all(&audio_dir).unwrap();
    fs::create_dir_all(&empty_dir).unwrap();

    // 正常音频
    fs::write(audio_dir.join("song.flac"), b"fLaC....").unwrap();
    // MF-CLEAN-001 系统垃圾
    fs::write(audio_dir.join("Thumbs.db"), b"x").unwrap();
    fs::write(root.join(".DS_Store"), b"x").unwrap();
    fs::write(root.join("desktop.ini"), b"x").unwrap();
    // MF-CLEAN-002 临时文件
    fs::write(root.join("song.flac.part"), b"x").unwrap();
    // MF-CLEAN-003 零字节
    fs::write(root.join("zero.txt"), b"").unwrap();
    // MF-CLEAN-005 孤立歌词（同目录无 song.lrc 对应音频？song.flac 存在——用孤儿名）
    fs::write(audio_dir.join("orphan.lrc"), b"[00:00]x").unwrap();
    // MF-CLEAN-006 孤立封面（empty 之外，cover 目录无音频）
    let cover_dir = root.join("coverdir");
    fs::create_dir_all(&cover_dir).unwrap();
    fs::write(cover_dir.join("cover.jpg"), b"jpeg").unwrap();
    // MF-CLEAN-007 非法字符：仅 Unix 可真实创建（Windows 文件系统本身拒绝 |，
    // 真实场景中此类文件来自其他系统/网络拷贝——规则保留用于扫描报告）
    #[cfg(unix)]
    fs::write(root.join("bad|name.txt"), b"x").unwrap();
    // MF-CLEAN-009 乱码替换符
    fs::write(root.join("\u{FFFD}garbled.mp3"), b"x").unwrap();
    // 正常其他
    fs::write(root.join("notes.txt"), b"hello").unwrap();
    root
}

/// 长路径（>260 字符）单独构造（Windows 下组件上限另算，这里测路径总长）。
fn long_path_file(root: &Path) -> PathBuf {
    let mut p = root.to_path_buf();
    let seg = "d".repeat(40);
    for _ in 0..8 {
        p = p.join(&seg);
    }
    p.set_extension("txt");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, b"x").unwrap();
    p
}

#[test]
fn scan_classifies_and_counts() {
    let root = polluted();
    let report: ScanReport = scan_library(&root, &ScanOptions::default()).unwrap();

    assert!(report.audio >= 2, "至少 2 个音频（song.flac + 乱码.mp3）");
    assert!(report.junk >= 5, "系统垃圾/临时/零字节/孤立歌词/孤立封面");
    assert_eq!(report.empty_dirs.len(), 1, "empty 目录应被记录");

    // 规则命中
    assert!(report.rule_hits.get("MF-CLEAN-001").copied().unwrap_or(0) >= 3);
    assert!(report.rule_hits.get("MF-CLEAN-002").copied().unwrap_or(0) >= 1);
    assert!(report.rule_hits.get("MF-CLEAN-003").copied().unwrap_or(0) >= 1);
    assert!(report.rule_hits.get("MF-CLEAN-004").copied().unwrap_or(0) >= 1);
    assert!(report.rule_hits.get("MF-CLEAN-005").copied().unwrap_or(0) >= 1);
    assert!(report.rule_hits.get("MF-CLEAN-006").copied().unwrap_or(0) >= 1);
    #[cfg(unix)]
    assert!(report.rule_hits.get("MF-CLEAN-007").copied().unwrap_or(0) >= 1);
    assert!(report.rule_hits.get("MF-CLEAN-009").copied().unwrap_or(0) >= 1);

    let _ = fs::remove_dir_all(&root);
}

/// 长路径命中 MF-CLEAN-008。
#[test]
fn scan_flags_long_paths() {
    let root = fixtures();
    let long = long_path_file(&root);
    let report = scan_library(&root, &ScanOptions::default()).unwrap();
    assert!(
        report.rule_hits.get("MF-CLEAN-008").copied().unwrap_or(0) >= 1,
        "长路径应命中 MF-CLEAN-008"
    );
    let _ = fs::remove_dir_all(&root);
    let _ = long;
}

/// 清洗计划与执行：移入回收站 + 回滚清单 + 还原往返。
#[test]
fn clean_plan_apply_and_restore_roundtrip() {
    let root = polluted();
    let report = scan_library(&root, &ScanOptions::default()).unwrap();
    let trash = root.join(".musicforge").join("trash");
    let plan: CleanPlan = build_clean_plan(
        &report,
        &["MF-CLEAN-001", "MF-CLEAN-002", "MF-CLEAN-003"]
            .iter()
            .copied()
            .collect(),
        &trash,
        &root,
    );

    // 只包含启用的规则，且全部是 Junk 类
    assert_eq!(
        plan.actions.len(),
        report.items_by_rule("MF-CLEAN-001").len()
            + report.items_by_rule("MF-CLEAN-002").len()
            + report.items_by_rule("MF-CLEAN-003").len()
    );
    assert!(!plan.actions.is_empty());

    let outcome = musicforge_core::scan::apply_clean_plan(&plan, "t-1").unwrap();
    assert_eq!(outcome.moved, plan.actions.len(), "全部移入回收站");
    for a in &plan.actions {
        assert!(!a.path.exists(), "原位置应清空: {:?}", a.path);
    }
    assert!(outcome.rollback_manifest.as_ref().unwrap().exists());

    // 还原：全部搬回
    let n = musicforge_core::scan::restore_from_trash(outcome.rollback_manifest.as_ref().unwrap())
        .unwrap();
    assert_eq!(n, plan.actions.len());
    for a in &plan.actions {
        assert!(a.path.exists(), "还原后应回到原位: {:?}", a.path);
    }
    let _ = fs::remove_dir_all(&root);
}

/// 清洗不碰正常音频与其他类别（只动启用的垃圾规则）。
#[test]
fn clean_never_touches_audio_or_unenabled_rules() {
    let root = polluted();
    let report = scan_library(&root, &ScanOptions::default()).unwrap();
    let trash = root.join(".musicforge").join("trash");
    // 只启用 001（系统垃圾）——002/003 等不启用
    let plan = build_clean_plan(
        &report,
        &["MF-CLEAN-001"].iter().copied().collect(),
        &trash,
        &root,
    );
    for a in &plan.actions {
        assert_eq!(a.rule_id, "MF-CLEAN-001");
    }
    // 孤立歌词与孤立封面不在动作里
    let audio_count = report.audio;
    let outcome = musicforge_core::scan::apply_clean_plan(&plan, "t-1").unwrap();
    assert_eq!(outcome.moved, plan.actions.len());
    // 音频文件全部还在
    let still = scan_library(&root, &ScanOptions::default()).unwrap();
    assert!(still.audio >= audio_count, "音频不应被清洗动作触碰");
    let _ = fs::remove_dir_all(&root);
}
