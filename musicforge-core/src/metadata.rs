//! 元数据层 facade（P1b/D9 绞杀者重构）。
//!
//! 物理布局（2018 版「文件 + 同名目录」）：
//! - [`model`]：元数据模型与 JSON 解析（原 `src/metadata.rs`）
//! - [`tagger`]：lofty 标签写入（原 `src/tagger.rs`）
//!
//! 本文件保证旧路径 `crate::metadata::Metadata`、`crate::metadata::parse`、
//! `crate::tagger`（经 `lib.rs` 重导出）全部保持可编译。

pub mod model;
pub mod tagger;

pub use model::parse;
pub use model::Metadata;
