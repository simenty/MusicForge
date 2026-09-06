//! 执行安全分级（v0.2.0 安全任务层）。
//!
//! 目的：把「哪些命令默认只规划、哪些可以直接执行」从各命令的临时判断里
//! 抽出来，变成**一处定义、处处一致**的策略，并用测试钉住：
//!
//! | 命令类别 | 默认 | 需要执行时 |
//! |---|---|---|
//! | `NonDestructive`（如 convert：只产出新文件、不覆盖） | **Apply** | 无需额外标志 |
//! | `Destructive`（如 clean / dedupe / organize：会删改已有文件） | **DryRun** | 必须 `--apply` |
//! | `Destructive { high_risk: true }`（如批量删除/移动源文件） | **DryRun** | `--apply` **且** `--yes` |
//!
//! 设计原则：**默认偏向不执行**。用户忘了加标志时的结果应该是「什么都没做」，
//! 而不是「误删了一整个曲库」。

/// 命令类别：决定默认是否需要显式 `--apply`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpClass {
    /// 只产出新文件、不删除不覆盖已有文件（convert）
    NonDestructive,
    /// 会修改/删除已有文件（clean / dedupe / organize）
    Destructive {
        /// 高危：批量删除或移动源文件，除 `--apply` 外还需 `--yes`
        high_risk: bool,
    },
}

/// 解析后的执行模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecMode {
    /// 只规划并留痕，不改动任何文件
    DryRun,
    /// 真正执行
    Apply,
}

/// 命令行传入的标志。
#[derive(Debug, Default, Clone, Copy)]
pub struct OpFlags {
    pub dry_run: bool,
    pub apply: bool,
    pub yes: bool,
}

/// 安全策略错误（稳定码见 `docs/result-codes.md`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyError {
    code: &'static str,
    message: String,
}

impl SafetyError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// 稳定错误码。
    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for SafetyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SafetyError {}

/// 依据命令类别与标志解析执行模式。
///
/// - `--dry-run` 与 `--apply` 同时给出 → 冲突（不猜用户意图）；
/// - 破坏类默认 DryRun，必须 `--apply`；
/// - 高危破坏类还需 `--yes`。
pub fn resolve(class: OpClass, flags: &OpFlags) -> Result<ExecMode, SafetyError> {
    if flags.dry_run && flags.apply {
        return Err(SafetyError::new(
            "MF-OP-CONFLICT",
            "同时指定了 --dry-run 与 --apply：请只保留一个",
        ));
    }
    if flags.dry_run {
        return Ok(ExecMode::DryRun);
    }

    match class {
        OpClass::NonDestructive => Ok(ExecMode::Apply),
        OpClass::Destructive { high_risk } => {
            if !flags.apply {
                return Ok(ExecMode::DryRun);
            }
            if high_risk && !flags.yes {
                return Err(SafetyError::new(
                    "MF-OP-NEEDS-YES",
                    "高危操作：--apply 之外还需 --yes 才会真正执行",
                ));
            }
            Ok(ExecMode::Apply)
        }
    }
}

/// 面向用户的模式说明（CLI 输出用）。
pub fn mode_note(class: OpClass, mode: ExecMode) -> &'static str {
    match (class, mode) {
        (_, ExecMode::DryRun) => "仅规划：未改动任何文件（加 --apply 才会执行）",
        (OpClass::NonDestructive, ExecMode::Apply) => "执行中（只产出新文件，不覆盖已有文件）",
        (OpClass::Destructive { .. }, ExecMode::Apply) => "执行中：将修改/删除已有文件",
    }
}
