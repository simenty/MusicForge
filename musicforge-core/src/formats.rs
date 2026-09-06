//! 格式层（P1b/D9 绞杀者重构：按「格式族」归位）。
//!
//! - [`ncm`]：NCM 容器的密码学原语、头部解析与流式解码
//! - [`probe`]：音频格式判定（约束 8 三级融合：metadata.format 优先 → 魔数兜底 → 终检）
//!
//! 旧公开路径 `crate::crypto` / `crate::decoder` / `crate::header` / `crate::format`
//! 由 `lib.rs` 的模块重导出保持可编译（P1c 起新代码走 `formats::registry`）。

pub mod ncm;
pub mod probe;
