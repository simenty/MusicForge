//! 执行安全分级 + 路径域放行（P3-23：从 cli / server 下沉到 core 共享模块）。
//!
//! 目的：把「默认偏向不执行」与「路径必须落在白名单根内」两类安全边界，从
//! cli 的 `safety.rs`、server 的 `api.rs::ensure_allowed` 各自为政，统一为
//! **一处定义、处处一致**的策略，供 server / GUI / plugin-host 统一调用
//! （消除「漏网的那一个」）。
//!
//! 设计原则：
//! - 操作分级：**默认偏向不执行**。用户忘了加标志时的结果应是「什么都没做」。
//! - 路径域：词法 `Component` 级规范化后前缀比较，杜绝 `../` 越界与字符串前缀陷阱；
//!   白名单为空 = 不约束（默认姿态，向后兼容 P0 之前无白名单）。server 若需解析
//!   符号链接可再 `canonicalize`（见 server/api.rs）。

use std::path::{Component, Path, PathBuf};

// ============================ 操作分级 ============================

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

/// 命令行 / 请求传入的标志。
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

/// 面向用户的模式说明（CLI / GUI 输出用）。
pub fn mode_note(class: OpClass, mode: ExecMode) -> &'static str {
    match (class, mode) {
        (_, ExecMode::DryRun) => "仅规划：未改动任何文件（加 --apply 才会执行）",
        (OpClass::NonDestructive, ExecMode::Apply) => "执行中（只产出新文件，不覆盖已有文件）",
        (OpClass::Destructive { .. }, ExecMode::Apply) => "执行中：将修改/删除已有文件",
    }
}

// ============================ 路径域 ============================

/// 路径域放行检查：path 必须落在某个 `allowed_roots` 之内（自身或子路径）。
///
/// - `allowed_roots` 为空 → 不约束（默认姿态，向后兼容 P0 之前无白名单）。
/// - 用 [`std::path::Component`] 做**词法规范化**后前缀比较，杜绝 `../` 越界绕过
///   （server 原实现曾用纯词法前缀**字符串**比较，被 `/data/music/../../etc` 绕过），
///   也避免字符串前缀陷阱（`/data/music-evil` 不会误判为 `/data/music` 的子路径）。
/// - 本函数只做词法层（跨平台可测）；server 若需解析符号链接应再 `canonicalize`。
pub fn ensure_allowed(allowed_roots: &[PathBuf], path: &Path) -> Result<(), SafetyError> {
    if allowed_roots.is_empty() {
        return Ok(());
    }
    let target = normalize_components(path).map_err(|_| {
        SafetyError::new(
            "MF-PATH-NOT-ALLOWED",
            format!("路径越出允许根：{}", path.display()),
        )
    })?;
    let allowed = allowed_roots.iter().any(|root| match normalize_components(root) {
        Ok(rn) => target == rn || is_prefix(&rn, &target),
        Err(_) => false,
    });
    if allowed {
        Ok(())
    } else {
        Err(SafetyError::new(
            "MF-PATH-NOT-ALLOWED",
            format!("路径不在允许根内：{}", path.display()),
        ))
    }
}

/// 词法规范化：展开 `.`、弹出 `..`；`..` 越过绝对根（或盘符根）即视为越界 → `Err`。
fn normalize_components<'a>(path: &'a Path) -> Result<Vec<Component<'a>>, ()> {
    let mut out: Vec<Component> = Vec::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => match out.last() {
                // 越过根：无法再向上（None / 已在根）
                None | Some(&Component::RootDir) | Some(&Component::Prefix(_)) => return Err(()),
                _ => {
                    out.pop();
                }
            },
            other => out.push(other),
        }
    }
    Ok(out)
}

/// `root` 是否为 `target` 的前缀（按 `Component` 精确相等比较，非字符串前缀）。
fn is_prefix<'a>(root: &[Component<'a>], target: &[Component<'a>]) -> bool {
    target.len() >= root.len() && target[..root.len()] == root[..]
}
