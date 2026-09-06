//! NCM 容器实现（P1b/D9：自 `src/{crypto,header,decoder}.rs` 原样迁入，零逻辑改动）。
//!
//! - [`crypto`]：AES-128-ECB（严格 PKCS7）、RC4 变体密钥盒与全局偏移密钥流
//! - [`header`]：头部解析（定长 read_exact + 长度上界 + CRC32 校验）
//! - [`decoder`]：流式 `Decoder` 与落盘（写后完整性校验 + 失败清理半成品）

pub mod crypto;
pub mod decoder;
pub mod header;
