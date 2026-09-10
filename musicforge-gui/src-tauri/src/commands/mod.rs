//! Tauri IPC 命令实现（P9 可维护性治理：巨型入口文件拆解）。
//!
//! 每个模块 = 原 `main.rs` 的一个**连续块逐字迁移**——逻辑零变更；
//! 通过 `use crate::*` 继承 crate root 的 imports 与类型定义。
//!
//! `#[macro_use]` 是必需的：`#[tauri::command]` 会为每条命令生成
//! `__cmd__*` / `__tauri_command_name_*` 两个 `macro_rules` 宏，
//! `generate_handler!` 在 crate root 展开时需要它们——`macro_use`
//! 必须**逐层**声明（孙模块 → 子模块 → root）才能提升到 root 作用域。

#[macro_use]
pub mod batch;
#[macro_use]
pub mod library;
#[macro_use]
pub mod plugins;

pub use batch::*;
pub use library::*;
pub use plugins::*;
