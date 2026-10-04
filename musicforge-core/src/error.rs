//! 错误体系（硬约束 1：零 panic；分层枚举对齐竞品 `NcmExceptions.cs` 范本 + 方案书五分类）

use thiserror::Error;

/// musicforge-core 统一错误。所有失败路径返回本枚举，绝不 panic。
#[derive(Debug, Error)]
pub enum NcmError {
    #[error("不是合法的 ncm 文件（magic 校验失败）")]
    BadMagic,

    #[error("文件在 {at} 处截断：需要 {need} 字节，实际 {got} 字节")]
    Truncated {
        at: &'static str,
        need: u64,
        got: u64,
    },

    #[error("{at} 的长度字段越界：{value}（上限 {max}）")]
    LengthOutOfRange {
        at: &'static str,
        value: u64,
        max: u64,
    },

    #[error("密钥明文前缀校验失败（期望 neteasecloudmusic 前缀）")]
    BadKeyPrefix,

    #[error("元数据外层前缀校验失败（期望 \"163 key(Don't modify):\"）")]
    BadMetaPrefix,

    #[error("元数据内层前缀校验失败（期望 \"music:\"）")]
    BadMusicPrefix,

    #[error("头部 CRC32 校验失败：文件已损坏。存储 {expected:#010x} ≠ 计算 {computed:#010x}（硬约束 9：拒绝产出损坏音频）")]
    CrcMismatch { expected: u32, computed: u32 },

    #[error("base64 解码元数据失败: {0}")]
    Base64(#[from] base64::DecodeError),

    #[error("元数据 JSON 解析失败: {0}")]
    MetadataJson(#[from] serde_json::Error),

    #[error("音频负载为空：无音频数据可解密")]
    EmptyAudio,

    #[error("密钥明文为空：前缀之后无 RC4 密钥数据，文件结构损坏")]
    EmptyKey,

    #[error(
        "无法判定音频格式（元数据无 format 且魔数不匹配）：拒绝产出可能损坏的文件（硬约束 9）"
    )]
    UnknownFormat,

    #[error("输出完整性校验失败：写入 {written} 字节，预期 {expected} 字节（硬约束 5）")]
    OutputIntegrity { written: u64, expected: u64 },

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("标签读取失败: {0}")]
    TagRead(String),

    #[error("标签写入失败: {0}")]
    TagWrite(String),

    #[error("状态库错误: {0}")]
    Db(String),

    #[error("无损转码失败: {0}")]
    Lossless(String),

    #[error("未找到 ffmpeg（已尝试 {searched} 个候选位置）")]
    FfmpegMissing { searched: usize },

    #[error("有损→无损升级转换被拦截（MP3→FLAC 等不会凭空恢复音质）")]
    UpgradeBlocked,

    #[error("目标文件已存在: {path}（转码/切分绝不覆盖既有文件）")]
    OutputExists { path: String },

    #[error("配置错误: {0}")]
    Config(String),

    #[error("需要用户确认: {0}")]
    PluginAckRequired(String),

    /// P6a-R：插件连续崩溃 3 次（§4.2）本会话禁用；或清单三禁位拒载（§4.10）
    #[error("插件被禁用: {0}")]
    PluginDisabled(String),

    #[error("插件不可用: {0}")]
    PluginNotFound(String),
}

impl NcmError {
    /// 稳定错误码（repair receipt：UI/日志可按码分类，不随文案变化）
    pub fn code(&self) -> &'static str {
        match self {
            NcmError::BadMagic => "NCM-BAD-MAGIC",
            NcmError::Truncated { .. } => "NCM-TRUNCATED",
            NcmError::LengthOutOfRange { .. } => "NCM-STRUCT-INVALID",
            NcmError::BadKeyPrefix
            | NcmError::BadMetaPrefix
            | NcmError::BadMusicPrefix
            | NcmError::EmptyKey => "NCM-STRUCT-INVALID",
            NcmError::CrcMismatch { .. } => "NCM-CRC-MISMATCH",
            NcmError::UnknownFormat => "NCM-FORMAT-UNKNOWN",
            NcmError::Base64(_) | NcmError::MetadataJson(_) => "NCM-METADATA-INVALID",
            NcmError::EmptyAudio => "NCM-EMPTY-AUDIO",
            NcmError::OutputIntegrity { .. } => "OUT-INTEGRITY",
            NcmError::Io(_) => "IO-ERROR",
            NcmError::TagRead(_) => "TAG-READ",
            NcmError::TagWrite(_) => "TAG-WRITE",
            NcmError::Db(_) => "MF-DB-FAILED",
            NcmError::Lossless(_) => "LOSSLESS-ERROR",
            NcmError::FfmpegMissing { .. } => "FFMPEG-MISSING",
            NcmError::UpgradeBlocked => "UPGRADE-BLOCKED",
            NcmError::OutputExists { .. } => "OUTPUT-EXISTS",
            NcmError::Config(_) => "MF-CONFIG-INVALID",
            NcmError::PluginAckRequired(_) => "MF-PLUGIN-ACK-REQUIRED",
            NcmError::PluginNotFound(_) => "MF-PLUGIN-NOT-FOUND",
            NcmError::PluginDisabled(_) => "MF-PLUGIN-DISABLED",
        }
    }

    /// 新命名空间稳定错误码（`MF-*`：跨格式/插件统一，P1e 起）。
    ///
    /// 与 [`NcmError::code`] 并存：旧 `NCM-*` 码**永久保留**以兼容既有脚本与
    /// 失败清单 CSV；新代码（GUI/报告/插件）一律用 `MF-*`。两者映射见
    /// `docs/result-codes.md`。
    pub fn mf_code(&self) -> &'static str {
        match self {
            NcmError::BadMagic => "MF-FORMAT-UNSUPPORTED",
            NcmError::Truncated { .. }
            | NcmError::LengthOutOfRange { .. }
            | NcmError::BadKeyPrefix
            | NcmError::BadMetaPrefix
            | NcmError::BadMusicPrefix
            | NcmError::EmptyKey
            | NcmError::CrcMismatch { .. } => "MF-FORMAT-CORRUPT",
            NcmError::Base64(_) | NcmError::MetadataJson(_) => "MF-METADATA-INVALID",
            NcmError::EmptyAudio => "MF-FORMAT-EMPTY-AUDIO",
            NcmError::UnknownFormat => "MF-FORMAT-UNKNOWN",
            NcmError::OutputIntegrity { .. } => "MF-OUTPUT-VERIFY-FAILED",
            NcmError::Io(_) => "MF-IO-FAILED",
            NcmError::TagRead(_) => "MF-TAG-READ-FAILED",
            NcmError::TagWrite(_) => "MF-TAG-WRITE-FAILED",
            NcmError::Db(_) => "MF-DB-FAILED",
            NcmError::Lossless(_) => "MF-LOSSLESS-FAILED",
            NcmError::FfmpegMissing { .. } => "MF-FFMPEG-MISSING",
            NcmError::UpgradeBlocked => "MF-LOSSY-TO-LOSSLESS",
            NcmError::OutputExists { .. } => "MF-OUTPUT-EXISTS",
            NcmError::Config(_) => "MF-CONFIG-INVALID",
            NcmError::PluginAckRequired(_) => "MF-PLUGIN-ACK-REQUIRED",
            NcmError::PluginNotFound(_) => "MF-PLUGIN-NOT-FOUND",
            NcmError::PluginDisabled(_) => "MF-PLUGIN-DISABLED",
        }
    }

    /// 用户可操作建议（repair receipt：发生了什么 → 你可以怎么做）
    pub fn suggestion(&self) -> &'static str {
        match self {
            NcmError::CrcMismatch { .. } => "Header verification failed: the file is corrupted or truncated. Please re-download it from NetEase Cloud Music and retry.",
            NcmError::BadMagic => "This file is not in ncm format (or is corrupted). Please verify its source and retry.",
            NcmError::Truncated { .. } | NcmError::LengthOutOfRange { .. } | NcmError::EmptyKey => {
                "File structure is abnormal and may be corrupted. Please obtain the source file again."
            }
            NcmError::Io(e) if e.kind() == std::io::ErrorKind::NotFound => "The source file was moved or deleted during processing. Please verify the path and retry.",
            NcmError::EmptyAudio => {
                "Audio payload is empty: no audio data to decrypt. The ncm file may be incomplete — please re-download it from NetEase Cloud Music and retry."
            }
            NcmError::UnknownFormat => {
                "Audio codec unrecognizable (metadata missing and magic bytes do not match). This version does not guess formats, to avoid producing corrupt files; please confirm the file source is complete."
            }
            NcmError::TagRead(_) => "Failed to read tags from the output file; it may be corrupted or incomplete. Please delete it and retry the conversion.",
            NcmError::TagWrite(_) => "Failed to write tags; check whether the output file is locked by another program.",
            NcmError::Db(_) => {
                "State DB error: it is only a regenerable cache and will be rebuilt after deletion; but do not place it on a network-mounted directory."
            }
            NcmError::Lossless(_) => {
                "Lossless transcoding failed: the source may be corrupted or contain an unsupported PCM form (e.g. float WAV). The source file was not modified — retry or choose a different target format."
            }
            NcmError::FfmpegMissing { .. } => {
                "ffmpeg not found. Install it from ffmpeg.org and add it to PATH, or specify the ffmpeg executable with --ffmpeg-path. Lossy export (MP3/AAC/Opus) depends on it."
            }
            NcmError::UpgradeBlocked => {
                "Converting a lossy source (MP3, etc.) to lossless (FLAC/WAV) cannot restore lost quality — it is a pseudo-upgrade and was blocked. If you truly need it (e.g. to unify library format), pass --i-know-lossy-to-lossless."
            }
            NcmError::OutputExists { .. } => {
                "The target file already exists; this tool never overwrites. Choose a different output directory, or handle the same-named artifact first."
            }
            NcmError::Config(_) => {
                "The config file is corrupted or its version is too new. Fix its contents, or delete config.json and let the program recreate it with defaults (config is regenerable)."
            }
            NcmError::PluginDisabled(_) => {
                "The plugin was disabled for this session due to repeated crashes, or its manifest contains forbidden permissions and was rejected. Restart the program or inspect the plugin manifest."
            }
            NcmError::PluginAckRequired(_) => {
                "This plugin is high-risk (e.g. local format migration) and requires explicit acknowledgement: musicforge plugins acknowledge <plugin-name> (or the GUI equivalent)."
            }
            NcmError::PluginNotFound(_) => {
                "The plugin runtime is unavailable. The offline build contains no plugins; install the corresponding plugin into the whitelisted directory and enable it, then retry."
            }
            _ => "Please check file and directory permissions and retry, or use the failure-manifest export to record this file.",
        }
    }
}
