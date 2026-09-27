#![no_main]
//! P3-29（扩展）：CUE 文本解析器模糊测试目标。
//!
//! 威胁模型（docs/threat-model.md §Parser memory corruption）：CUE 为文本格式，
//! 畸形/截断索引、非法时间码、乱序 FILE/TRACK 不得令解析器 panic（应返回 `Err` 或
//! `Some`）。本目标只验证「不崩溃」——无论解析成功或失败，对任意字节都必须安全返回，
//! 不得越界 / index panic。`parse_cue_text` 内部对索引越界有 `get` 守卫，畸形输入走
//! `Err` 路径而非崩溃；模糊测试用于回归守护该不变量。
//!
//! 运行（需 nightly + cargo-fuzz，本机 pinned 工具链为 stable，故不进主构建）：
//!
//! ```text
//! cargo +nightly fuzz run cue_parse
//! # 限定时长 / 用例数：
//! cargo +nightly fuzz run cue_parse -- -max_total_time=60
//! ```
use libfuzzer_sys::fuzz_target;
use musicforge_core::cue::{decode_bytes_to_utf8, parse_cue_text};

fuzz_target!(|data: &[u8]| {
    // P5a-6 扩展：此前非 UTF-8 输入被直接跳过，`decode_bytes_to_utf8` 的编码
    // 检测与解码路径（BOM、UTF-8 严格校验、chardetng 猜测 + GBK/BIG5/125x 候选
    // 回落）**完全未被模糊测试覆盖**。现喂任意字节走完整解码路径，再解析。
    // 无论解码/解析成功或失败都必须安全返回，不得 panic。
    let text = decode_bytes_to_utf8(data);
    let _ = parse_cue_text(&text);
});
