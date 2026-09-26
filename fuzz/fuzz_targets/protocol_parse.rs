#![no_main]
//! P3-29（扩展，X17）：插件协议 NDJSON 解析器模糊测试目标。
//!
//! 威胁模型（docs/plugin-protocol.md §fuzz）：协议文本来自不可信插件进程，畸形 / 截断
//! NDJSON 不得令 Host 解析器 panic（`from_str` 失败走 `Err`）。本目标只验证「不崩溃」——
//! 对任意字节，`Request` / `Response` / `PluginManifest` 反序列化必须安全返回，不得 panic。
//!
//! 运行（需 nightly + cargo-fuzz，本机 pinned 工具链为 stable，故不进主构建）：
//!
//! ```text
//! cargo +nightly fuzz run protocol_parse
//! cargo +nightly fuzz run protocol_parse -- -max_total_time=60
//! ```
use libfuzzer_sys::fuzz_target;
use musicforge_plugin_api::{PluginManifest, Request, Response};

fuzz_target!(|data: &[u8]| {
    if let Ok(line) = std::str::from_utf8(data) {
        let _ = serde_json::from_str::<Request>(line);
        let _ = serde_json::from_str::<Response>(line);
        let _ = serde_json::from_str::<PluginManifest>(line);
    }
});
