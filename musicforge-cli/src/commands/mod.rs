//! CLI 子命令实现（P9 可维护性治理：巨型入口文件拆解）。
//!
//! 每个模块 = 原 `main.rs` 的一个**连续块逐字迁移**——逻辑零变更；
//! 通过 `use super::*` 继承 crate root 的 imports 与类型定义。
//!
//! 后续可按需把跨块的同域函数（如 OrganizeArgs 与 run_organize_sub）
//! 再归并——本轮刻意保持"逐字迁移"以确保零行为变更、可回退。

// （Rust：glob 导入不携带父模块的**私有** use 绑定）。

pub mod convert;
pub mod dedupe;
pub mod organize;
pub mod plugins;
pub mod scan_clean;
pub mod state;
// P2-3：服务端 token 管理（CLI 新增模块，非迁移块）
pub mod token;

pub use convert::*;
pub use dedupe::*;
pub use organize::*;
pub use plugins::*;
pub use scan_clean::*;
pub use state::*;
pub use token::*;
