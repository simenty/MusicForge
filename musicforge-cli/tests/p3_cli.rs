//! P3 CLI 集成：scan / clean 子命令（跑真二进制）。
//!
//! 覆盖：只读扫描（人读+JSON）、clean 默认 dry-run、--apply 移入回收站、
//! --restore 整体还原、--rules 规则过滤。全程不触碰真实音频 fixture
//! （使用临时目录自建的污染树）。

use std::path::{Path, PathBuf};
use std::process::Command;

fn exe() -> &'static str {
    env!("CARGO_BIN_EXE_musicforge")
}

/// 构造污染树（真实可写；Windows 下不含非法字符文件名）。
fn polluted() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "musicforge-p3cli-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let audio = root.join("audio");
    std::fs::create_dir_all(&audio).unwrap();
    std::fs::write(audio.join("song.flac"), b"fLaCdata").unwrap();
    std::fs::write(audio.join("orphan.lrc"), b"[00:00]x").unwrap();
    let covers = root.join("covers");
    std::fs::create_dir_all(&covers).unwrap();
    std::fs::write(covers.join("cover.jpg"), b"jpeg").unwrap();
    std::fs::write(root.join("Thumbs.db"), b"x").unwrap();
    std::fs::write(root.join("song.flac.part"), b"x").unwrap();
    std::fs::write(root.join("zero.txt"), b"").unwrap();
    let empty = root.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    root
}

fn run_cli(args: &[&str]) -> (i32, String) {
    let out = Command::new(exe()).args(args).output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

#[test]
fn scan_sub_reports_counts_and_rules() {
    let root = polluted();
    let (code, out) = run_cli(&["scan", root.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(out.contains("音频"), "应含音频计数: {out}");
    assert!(out.contains("MF-CLEAN-001"), "应含系统垃圾规则命中: {out}");
    assert!(out.contains("MF-CLEAN-005"), "应含孤立歌词规则命中: {out}");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn scan_json_is_machine_readable() {
    let root = polluted();
    let (code, out) = run_cli(&["scan", root.to_str().unwrap(), "--json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).expect("JSON 可解析");
    assert_eq!(v["dir"].as_str(), Some(root.to_str().unwrap()));
    assert!(v["scanned_files"].as_u64().unwrap() >= 6);
    assert!(v["rule_hits"]["MF-CLEAN-001"].is_u64());
    assert!(v["items"].as_array().unwrap().len() >= 6);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn clean_defaults_to_dry_run_and_never_moves() {
    let root = polluted();
    let audio_file = root.join("audio").join("song.flac");
    let before = std::fs::read(&audio_file).unwrap();

    let (code, out) = run_cli(&["clean", root.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(out.contains("仅规划"), "clean 缺省应为 dry-run: {out}");

    assert!(audio_file.exists(), "dry-run 不得触碰文件");
    assert_eq!(std::fs::read(&audio_file).unwrap(), before);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn clean_apply_moves_to_trash_and_restore_roundtrip() {
    let root = polluted();
    let audio_file = root.join("audio").join("song.flac");

    // apply：移入回收站
    let (code, out) = run_cli(&["clean", root.to_str().unwrap(), "--apply"]);
    assert_eq!(code, 0, "apply 应成功: {out}");
    assert!(out.contains("已移入回收站"));
    assert!(
        audio_file.exists(),
        "音频文件绝不应被清洗动作触碰（G5/铁律：只动认领的垃圾）"
    );

    // 解析回滚清单路径
    let rb = out
        .lines()
        .find(|l| l.starts_with("回滚清单: "))
        .and_then(|l| l.strip_prefix("回滚清单: "))
        .expect("应打印回滚清单路径")
        .to_string();
    assert!(Path::new(&rb).exists(), "回滚清单应存在: {rb}");

    // 还原：全部搬回
    let (code2, out2) = run_cli(&["clean", "--restore", &rb]);
    assert_eq!(code2, 0, "还原应成功: {out2}");
    assert!(out2.contains("已还原"));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn clean_rules_filter_limits_actions() {
    let root = polluted();
    // 只启用 001：Thumbs.db / .DS_Store / desktop.ini
    let (code, out) = run_cli(&[
        "clean",
        root.to_str().unwrap(),
        "--rules",
        "MF-CLEAN-001",
        "--apply",
    ]);
    assert_eq!(code, 0);
    assert!(
        out.contains("已移入回收站 1 项"),
        "p3_cli 污染树只有 1 个系统垃圾（Thumbs.db）: {out}"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// legacy 顶层参数不受子命令引入影响：-d/-r/-o 转换路径照常。
#[test]
fn legacy_convert_args_still_work() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let (code, out_text) = run_cli(&[
        "-d",
        fixtures().to_str().unwrap(),
        "-r",
        "-o",
        out.to_str().unwrap(),
        "--skip-existing",
    ]);
    assert_eq!(code, 0, "legacy 转换应成功: {out_text}");
    assert!(out_text.contains("成功"), "汇总行应存在");
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../musicforge-core/tests/fixtures")
}
