//! # musicforge-core
//!
//! MusicForge 核心解密库：离线、流式、CRC 校验的网易云音乐 `.ncm` 解密。
//!
//! 设计约束（方案书 §7，11 条硬约束）在本 crate 的实现映射：
//! 1. 零 panic：所有失败路径返回 [`NcmError`]，release 构建启用 `panic = "abort"`
//! 2. RC4 使用**全局偏移**（修 B-5 块内偏移定时炸弹）
//! 3. 定长读取一律 `read_exact`（修短读静默截断）
//! 4. 所有长度字段做上界校验（防 OOM/越界）
//! 5. 所有 Read/Seek 返回值检查；`dump` 校验落盘字节数（修谎报成功）
//! 6. 本 crate **零网络依赖**（CI 双层扫描：use 语句 + 产物符号）
//! 7. FLAC/元数据解析走 lofty（应用层），本库不做标签写入
//! 8. 格式三级融合：metadata.format 优先 → 魔数兜底 → 解析终检（`format.rs`）
//! 9. **CRC32 头校验**：头损坏明确报错，绝不静默产出损坏音频
//! 10. 并发策略在应用层（有界 worker pool + 文件 UUID 事件）
//! 11. Metadata 缺失 → `None`，调用方跳过打标签，绝不崩

pub mod crypto;
pub mod decoder;
pub mod error;
pub mod format;
pub mod header;
pub mod metadata;
pub mod tagger;
pub mod template;

pub use decoder::Decoder;
pub use error::NcmError;
pub use format::Format;
pub use metadata::Metadata;

/// NCM 文件魔数（已实证：小端比较 `0x4E455443`("CTEN") + `0x4D414446`("FDAM")）
pub const MAGIC: [u8; 8] = *b"CTENFDAM";
/// 密钥明文前缀（17 字节，已实证）
pub const NETEASE_PREFIX: [u8; 17] = *b"neteasecloudmusic";
/// 元数据外层前缀（22 字节，已实证）
pub const META_PREFIX: [u8; 22] = *b"163 key(Don't modify):";
/// 元数据内层前缀（6 字节，已实证）
pub const MUSIC_PREFIX: [u8; 6] = *b"music:";

/// 硬编码核心密钥（公开逆向共识，非机密）
pub const CORE_KEY: [u8; 16] = [
    0x68, 0x7A, 0x48, 0x52, 0x41, 0x6D, 0x73, 0x6F, 0x35, 0x6B, 0x49, 0x6E, 0x62, 0x61, 0x78, 0x57,
];
/// 硬编码元数据密钥（公开逆向共识，非机密）
pub const META_KEY: [u8; 16] = [
    0x23, 0x31, 0x34, 0x6C, 0x6A, 0x6B, 0x5F, 0x21, 0x5C, 0x5D, 0x26, 0x30, 0x55, 0x3C, 0x27, 0x28,
];
