//! 命名模板 facade（P1b/D9 绞杀者重构）。
//!
//! 物理布局（2018 版「文件 + 同名目录」）：[`engine`]（模板引擎，原 `src/template.rs`）。
//!
//! 本文件保证旧路径 `crate::template::render_filename`、`crate::template::Fallbacks`
//! 保持可编译——GUI `preview_template` 与 CLI `--template` 共同依赖的公开面。

pub mod engine;

pub use engine::render_filename;
pub use engine::Fallbacks;
