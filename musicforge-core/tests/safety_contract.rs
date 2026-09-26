//! P3-23 契约 / 金标测试：core 共享 `safety` 模块（操作分级 + 路径域）。
//!
//! 钉死两条不变量，作为后续 server / GUI / plugin-host 从各自副本迁移到本模块时的
//! **等价性基线**：
//! 1. 操作分级「默认偏向不执行」（忘了加标志 → 什么都没做，而非误删曲库）；
//! 2. 路径域「`../` 不可越界、字符串前缀不可误判」。

use musicforge_core::safety::{ensure_allowed, mode_note, resolve, ExecMode, OpClass, OpFlags};
use std::path::Path;

// ---------------- 操作分级 ----------------

#[test]
fn nondestructive_applies_by_default() {
    assert_eq!(resolve(OpClass::NonDestructive, &OpFlags::default()), Ok(ExecMode::Apply));
}

#[test]
fn nondestructive_dry_run_stays_dry() {
    assert_eq!(
        resolve(
            OpClass::NonDestructive,
            &OpFlags {
                dry_run: true,
                ..Default::default()
            }
        ),
        Ok(ExecMode::DryRun)
    );
}

#[test]
fn destructive_defaults_to_dry_run() {
    assert_eq!(
        resolve(OpClass::Destructive { high_risk: false }, &OpFlags::default()),
        Ok(ExecMode::DryRun)
    );
}

#[test]
fn destructive_apply_without_yes_allowed_for_normal() {
    assert_eq!(
        resolve(
            OpClass::Destructive { high_risk: false },
            &OpFlags {
                apply: true,
                ..Default::default()
            }
        ),
        Ok(ExecMode::Apply)
    );
}

#[test]
fn high_risk_needs_yes() {
    // 高危破坏类：--apply 但无 --yes → 拒绝
    let e = resolve(
        OpClass::Destructive { high_risk: true },
        &OpFlags {
            apply: true,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(e.code(), "MF-OP-NEEDS-YES");

    // --apply 且 --yes → 放行
    assert_eq!(
        resolve(
            OpClass::Destructive { high_risk: true },
            &OpFlags {
                apply: true,
                yes: true,
                ..Default::default()
            }
        ),
        Ok(ExecMode::Apply)
    );
}

#[test]
fn dry_run_and_apply_conflict() {
    let e = resolve(
        OpClass::NonDestructive,
        &OpFlags {
            dry_run: true,
            apply: true,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(e.code(), "MF-OP-CONFLICT");
}

#[test]
fn mode_note_strings() {
    assert!(mode_note(OpClass::NonDestructive, ExecMode::DryRun).contains("规划"));
    assert!(mode_note(OpClass::Destructive { high_risk: false }, ExecMode::Apply).contains("修改/删除"));
}

// ---------------- 路径域 ----------------

fn roots(parts: &[&str]) -> Vec<std::path::PathBuf> {
    parts.iter().map(std::path::PathBuf::from).collect()
}

#[test]
fn empty_roots_means_no_constraint() {
    assert_eq!(ensure_allowed(&[], Path::new("/etc/passwd")), Ok(()));
}

#[test]
fn inside_root_allowed() {
    let r = roots(&["/data/music"]);
    assert_eq!(ensure_allowed(&r, Path::new("/data/music/song.mp3")), Ok(()));
    // 根自身也算放行
    assert_eq!(ensure_allowed(&r, Path::new("/data/music")), Ok(()));
}

#[test]
fn dotdot_escape_rejected() {
    // 经典绕过：纯词法前缀字符串比较会把 /data/music/../../etc 误判为在白名单内
    let r = roots(&["/data/music"]);
    let e = ensure_allowed(&r, Path::new("/data/music/../../etc/passwd")).unwrap_err();
    assert_eq!(e.code(), "MF-PATH-NOT-ALLOWED");
    assert!(e.message().contains("/data/music/../../etc/passwd"));
}

#[test]
fn string_prefix_trap_rejected() {
    // /data/music-evil 不是 /data/music 的子路径（组件级比较防字符串前缀陷阱）
    let r = roots(&["/data/music"]);
    let e = ensure_allowed(&r, Path::new("/data/music-evil/x")).unwrap_err();
    assert_eq!(e.code(), "MF-PATH-NOT-ALLOWED");
}

#[test]
fn outside_root_rejected() {
    let r = roots(&["/data/music"]);
    let e = ensure_allowed(&r, Path::new("/data/other/x")).unwrap_err();
    assert_eq!(e.code(), "MF-PATH-NOT-ALLOWED");
}

#[test]
fn any_root_matches() {
    let r = roots(&["/data/a", "/data/b"]);
    assert_eq!(ensure_allowed(&r, Path::new("/data/b/deep/file")), Ok(()));
}
