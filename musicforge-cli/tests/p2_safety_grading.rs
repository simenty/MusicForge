//! v0.2.0 安全分级策略（safety::resolve）——把「默认偏向不执行」钉死。
//!
//! 这些断言保护的是**用户忘了加标志时的结果**：应该是「什么都没做」，
//! 而不是「误删了一整个曲库」。

use musicforge_cli::safety::{resolve, ExecMode, OpClass, OpFlags};

fn f(dry_run: bool, apply: bool, yes: bool) -> OpFlags {
    OpFlags {
        dry_run,
        apply,
        yes,
    }
}

/// 非破坏类（convert）：默认执行——否则每次转换都要多敲一个参数。
#[test]
fn non_destructive_defaults_to_apply() {
    let mode = resolve(OpClass::NonDestructive, &f(false, false, false)).unwrap();
    assert_eq!(mode, ExecMode::Apply);
}

/// 破坏类（clean / dedupe）：**默认只规划**。
#[test]
fn destructive_defaults_to_dry_run() {
    let mode = resolve(
        OpClass::Destructive { high_risk: false },
        &f(false, false, false),
    )
    .unwrap();
    assert_eq!(mode, ExecMode::DryRun, "破坏类命令默认必须只规划");
}

/// 破坏类显式 --apply 才执行。
#[test]
fn destructive_requires_apply() {
    let mode = resolve(
        OpClass::Destructive { high_risk: false },
        &f(false, true, false),
    )
    .unwrap();
    assert_eq!(mode, ExecMode::Apply);
}

/// 高危破坏类：--apply 之外还需 --yes，否则报错（不猜意图）。
#[test]
fn high_risk_requires_yes() {
    let err = resolve(
        OpClass::Destructive { high_risk: true },
        &f(false, true, false),
    )
    .unwrap_err();
    assert_eq!(err.code(), "MF-OP-NEEDS-YES");

    let ok = resolve(
        OpClass::Destructive { high_risk: true },
        &f(false, true, true),
    )
    .unwrap();
    assert_eq!(ok, ExecMode::Apply);
}

/// --dry-run 与 --apply 冲突：直接报错，不静默选一个。
#[test]
fn conflicting_flags_are_rejected() {
    let err = resolve(OpClass::NonDestructive, &f(true, true, false)).unwrap_err();
    assert_eq!(err.code(), "MF-OP-CONFLICT");
}

/// 任何类别下 --dry-run 都压倒一切（用户显式要求只看计划）。
#[test]
fn dry_run_always_wins() {
    for class in [
        OpClass::NonDestructive,
        OpClass::Destructive { high_risk: false },
        OpClass::Destructive { high_risk: true },
    ] {
        let mode = resolve(class, &f(true, false, true)).unwrap();
        assert_eq!(
            mode,
            ExecMode::DryRun,
            "{class:?}: --dry-run 必须压倒其他标志"
        );
    }
}
