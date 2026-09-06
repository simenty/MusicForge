//! P4 CLI 集成：dedupe 子命令（跑真二进制）。
//!
//! 覆盖：默认 dry-run 不动文件、--apply 牺牲项进回收站 + 保留项原位未动 +
//! clean --restore 往返、--suggest 显式报 MF-PLUGIN-NOT-FOUND、--include-same-name
//! 需 --apply（clap requires 闸）。

use std::path::PathBuf;
use std::process::Command;

fn exe() -> &'static str {
    env!("CARGO_BIN_EXE_musicforge")
}

/// 树：1 组 exact ×2（同内容）+ 1 组同名候选（同目录同 stem 不同内容）+ 1 unique。
fn tree() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "musicforge-p4cli-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let a = root.join("a");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::write(a.join("same1.flac"), b"dup-content-0001").unwrap();
    std::fs::write(a.join("same2.flac"), b"dup-content-0001").unwrap();
    std::fs::write(a.join("song.flac"), b"song-content-flac-v1").unwrap();
    std::fs::write(a.join("song.mp3"), b"song-content-mp3-v2").unwrap();
    std::fs::write(root.join("u.flac"), b"unique-content").unwrap();
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
fn dedupe_defaults_to_dry_run_and_never_moves() {
    let root = tree();
    let a = root.join("a");
    let before = std::fs::read(a.join("same1.flac")).unwrap();

    let (code, out) = run_cli(&["dedupe", root.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("仅规划"), "缺省应为 dry-run: {out}");
    assert!(out.contains("牺牲"), "应列出牺牲项: {out}");
    assert!(out.contains("同名候选"), "应报告同名候选组: {out}");

    // dry-run 零改动
    assert!(a.join("same1.flac").exists());
    assert_eq!(std::fs::read(a.join("same1.flac")).unwrap(), before);
    assert!(a.join("song.mp3").exists(), "同名候选默认绝不动");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn dedupe_suggest_reports_plugin_not_found() {
    let root = tree();
    let (code, out) = run_cli(&["dedupe", root.to_str().unwrap(), "--suggest"]);
    assert_ne!(code, 0, "--suggest 在离线版必须非零退出");
    assert!(
        out.contains("MF-PLUGIN-NOT-FOUND"),
        "必须显式报稳定码: {out}"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn dedupe_apply_moves_sacrifices_keeps_best_and_restores() {
    let root = tree();
    let a = root.join("a");
    let (code, out) = run_cli(&["dedupe", root.to_str().unwrap(), "--apply", "--json"]);
    assert_eq!(code, 0, "{out}");

    // JSON 口径核对：1 组牺牲（2→1），同名候选不纳入，apply 真实执行
    let v: serde_json::Value = serde_json::from_str(&out).expect("JSON 可解析");
    assert_eq!(v["groups"].as_array().unwrap().len(), 1);
    assert_eq!(v["groups"][0]["sacrifices"].as_array().unwrap().len(), 1);
    assert_eq!(v["plan"]["actions"].as_u64(), Some(1));
    assert_eq!(v["plan"]["include_same_name"].as_bool(), Some(false));
    assert_eq!(
        v["outcome"]["moved"].as_u64(),
        Some(1),
        "apply 必须真实执行"
    );

    // 磁盘口径：牺牲项离位、保留项与同名候选原位未动
    let sacrifice = v["groups"][0]["sacrifices"][0]["path"].as_str().unwrap();
    assert!(
        !std::path::Path::new(sacrifice).exists(),
        "牺牲项应已进回收站"
    );
    let keep = v["groups"][0]["keep"]["path"].as_str().unwrap();
    assert!(std::path::Path::new(keep).exists(), "保留项必须原位");
    assert!(a.join("song.flac").exists() && a.join("song.mp3").exists());

    // restore 往返（回滚清单路径取自 JSON outcome）
    let rb = PathBuf::from(
        v["outcome"]["rollback_manifest"]
            .as_str()
            .expect("apply 应产出回滚清单"),
    );
    assert!(rb.exists(), "rollback.jsonl 应存在: {}", rb.display());
    let (code2, out2) = run_cli(&["clean", "--restore", rb.to_str().unwrap()]);
    assert_eq!(code2, 0, "{out2}");
    assert!(
        std::path::Path::new(sacrifice).exists(),
        "还原后牺牲项回原位"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn organize_dry_run_never_moves() {
    let root = tree();
    let (code, out) = run_cli(&["organize", root.to_str().unwrap(), "--json"]);
    assert_eq!(code, 0, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("JSON 可解析");
    assert_eq!(v["mode"], "dry-run");
    assert!(v["counts"]["planned"].as_u64().unwrap() >= 3, "{out}");
    // dry-run 零改动
    assert!(root.join("a/same1.flac").exists());
    assert!(root.join("u.flac").exists());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn organize_apply_moves_and_rollback_restores() {
    let root = tree();
    let target = root.join("sorted");
    let (code, out) = run_cli(&[
        "organize",
        root.to_str().unwrap(),
        "--to",
        target.to_str().unwrap(),
        "--apply",
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("JSON 可解析");
    assert_eq!(v["mode"], "apply");
    let moved = v["outcome"]["moved"].as_u64().unwrap();
    assert!(moved >= 3, "至少 3 个音频应被移动: {out}");

    // 源位置已空、目标存在
    assert!(!root.join("a/same1.flac").exists());
    let rb = PathBuf::from(v["outcome"]["rollback_manifest"].as_str().unwrap());
    assert!(rb.exists(), "回滚清单应存在");

    // 还原（复用 clean --restore）
    let (code2, out2) = run_cli(&["clean", "--restore", rb.to_str().unwrap()]);
    assert_eq!(code2, 0, "{out2}");
    assert!(root.join("a/same1.flac").exists(), "还原后文件回原位");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn organize_conflict_skip_reported_and_never_overwrites() {
    let root = tree();
    let target = root.join("sorted");
    std::fs::create_dir_all(&target).unwrap();
    // 预创建冲突目标：模板 "{title}" 下 same1.flac → sorted/same1.flac
    std::fs::write(target.join("same1.flac"), b"PRE-EXISTING").unwrap();

    let (code, out) = run_cli(&[
        "organize",
        root.to_str().unwrap(),
        "--to",
        target.to_str().unwrap(),
        "--template",
        "{title}",
        "--apply",
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("JSON 可解析");
    assert_eq!(
        v["counts"]["skipped_conflict"].as_u64(),
        Some(1),
        "冲突应按 skip 报告: {out}"
    );
    // 冲突目标原封不动，源文件原位保留
    assert_eq!(
        std::fs::read(target.join("same1.flac")).unwrap(),
        b"PRE-EXISTING"
    );
    assert!(root.join("a/same1.flac").exists());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn organize_rejects_unknown_conflict_strategy() {
    let root = tree();
    let (code, out) = run_cli(&[
        "organize",
        root.to_str().unwrap(),
        "--conflict",
        "bogus",
        "--apply",
    ]);
    assert_eq!(code, 2, "未知策略必须退出码 2: {out}");
    assert!(out.contains("未知冲突策略"), "{out}");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn playlist_export_then_import_repair_cycle() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let root = std::env::temp_dir().join(format!(
        "musicforge-p4pl-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let lib = root.join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    // 真实可解析 WAV（44100B 数据 = 0.5s→0s；仅需存在与可被 lofty 解析）
    let wav = |len: usize| {
        let data = vec![0u8; len];
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&44100u32.to_le_bytes());
        v.extend_from_slice(&44100u32.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(&data);
        v
    };
    std::fs::write(lib.join("a.wav"), wav(88200)).unwrap();
    std::fs::write(lib.join("b.wav"), wav(88200)).unwrap();

    // 导出（不分组 → library.m3u8）
    let pls = root.join("pls");
    let (code, out) = run_cli(&[
        "playlist",
        "export",
        lib.to_str().unwrap(),
        "--out",
        pls.to_str().unwrap(),
        "--by",
        "none",
    ]);
    assert_eq!(code, 0, "{out}");
    let list = pls.join("library.m3u8");
    assert!(list.exists(), "{out}");

    // 搬走一个文件制造失效路径
    let moved_to = root.join("elsewhere");
    std::fs::create_dir_all(&moved_to).unwrap();
    std::fs::rename(lib.join("a.wav"), moved_to.join("a.wav")).unwrap();

    // 导入修复（搜索根 = 库根的父目录）
    let search = root.join("").to_str().unwrap().to_string();
    let (code2, out2) = run_cli(&[
        "playlist",
        "import",
        list.to_str().unwrap(),
        "--search",
        search.trim_end(),
        "--json",
    ]);
    assert_eq!(code2, 0, "{out2}");
    let v: serde_json::Value = serde_json::from_str(&out2).expect("JSON 可解析");
    assert_eq!(v["total"].as_u64(), Some(2));
    assert_eq!(
        v["repaired"].as_array().unwrap().len(),
        1,
        "a.wav 应被修复: {out2}"
    );
    assert_eq!(v["ok"].as_u64(), Some(1));
    let written = PathBuf::from(v["written"].as_str().unwrap());
    assert!(written.exists());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn playlist_export_rejects_unknown_group_key() {
    let root = tree();
    let (code, out) = run_cli(&[
        "playlist",
        "export",
        root.to_str().unwrap(),
        "--out",
        root.join("pls").to_str().unwrap(),
        "--by",
        "bogus",
    ]);
    assert_eq!(code, 2, "未知分类键必须退出码 2: {out}");
    assert!(out.contains("未知分类键"), "{out}");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn dedupe_include_same_name_requires_apply() {
    let root = tree();
    // clap 闸：--include-same-name 不带 --apply 直接被拒（exit 2）
    let (code, _out) = run_cli(&["dedupe", root.to_str().unwrap(), "--include-same-name"]);
    assert_ne!(code, 0, "缺 --apply 时 --include-same-name 必须被拒");
    std::fs::remove_dir_all(&root).ok();
}
