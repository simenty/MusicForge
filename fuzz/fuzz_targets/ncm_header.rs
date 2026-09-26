#![no_main]
//! P3-29：NCM 头解析器模糊测试目标。
//!
//! 威胁模型承诺：畸形 `.ncm` 不应令解析器 panic（应返回 `Err`）。本目标只验证
//! 「不崩溃」——无论解析成功或失败，对任意输入都必须安全返回，不得越界 / index panic。
//! `parse_blob` 内部的每次越界读取前都有 `need()` 边界检查（header.rs），故畸形输入
//! 走 `Err` 路径而非崩溃；模糊测试用于回归守护该不变量。
//!
//! 运行（需 nightly + cargo-fuzz，本机 pinned 工具链为 stable，故不进主构建）：
//!
//! ```text
//! cargo +nightly fuzz run ncm_header
//! # 限定时长 / 用例数：
//! cargo +nightly fuzz run ncm_header -- -max_total_time=60
//! ```
use libfuzzer_sys::fuzz_target;
use musicforge_core::formats::ncm::header::parse_blob;

fuzz_target!(|data: &[u8]| {
    // 忽略结果：只关心是否触发 panic / 越界（解析成功或 Truncated/BadMagic 等 Err 均合格）。
    let _ = parse_blob(data);
});
