#![no_main]
//! P3-29（扩展）：metadata JSON 解析器模糊测试目标。
//!
//! 威胁模型（docs/threat-model.md §Parser memory corruption）：metadata 来自在线 / 插件，
//! 畸形 JSON 不得 panic。`parse` 内部 `from_utf8` + `serde_json::from_str` 失败均走 `Err`，
//! 不崩溃；本目标只验证「不崩溃」——对任意字节，解析成功或 `MetadataJson`/`Serde` 等 `Err`
//! 均合格。模糊测试用于回归守护该不变量。
//!
//! 运行（需 nightly + cargo-fuzz，本机 pinned 工具链为 stable，故不进主构建）：
//!
//! ```text
//! cargo +nightly fuzz run metadata_parse
//! cargo +nightly fuzz run metadata_parse -- -max_total_time=60
//! ```
use libfuzzer_sys::fuzz_target;
use musicforge_core::metadata::model::parse;

fuzz_target!(|data: &[u8]| {
    let _ = parse(data);
});
