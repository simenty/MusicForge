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
        }
    }

    /// 用户可操作建议（repair receipt：发生了什么 → 你可以怎么做）
    pub fn suggestion(&self) -> &'static str {
        match self {
            NcmError::CrcMismatch { .. } => "文件头校验失败，文件已损坏或被截断。建议在网易云音乐中重新下载后重试。",
            NcmError::BadMagic => "该文件不是 ncm 格式（或已损坏）。请确认来源后重试。",
            NcmError::Truncated { .. } | NcmError::LengthOutOfRange { .. } | NcmError::EmptyKey => {
                "文件结构异常，可能已损坏。建议重新获取源文件。"
            }
            NcmError::Io(e) if e.kind() == std::io::ErrorKind::NotFound => "源文件在处理期间被移动或删除。请确认路径后重试。",
            NcmError::EmptyAudio => {
                "音频负载为空：无音频数据可解密。该 ncm 文件可能不完整，建议在网易云音乐中重新下载后重试。"
            }
            NcmError::UnknownFormat => {
                "无法识别音频编码（元数据缺失且魔数不匹配）。本版本不猜测格式以免产出损坏文件；请确认文件来源是否完整。"
            }
            NcmError::TagRead(_) => "输出文件标签读取失败，文件可能已损坏或不完整。建议删除后重试转换。",
            NcmError::TagWrite(_) => "标签写入失败，请检查输出文件是否被其他程序占用。",
            NcmError::Db(_) => {
                "状态库异常：它只是可再生缓存，删除后会自动重建；但请勿将其放在网络挂载目录上。"
            }
            NcmError::Lossless(_) => {
                "无损转码失败：源文件可能损坏或包含不支持的 PCM 形态（如浮点 WAV）。源文件未被修改，可放心重试或更换目标格式。"
            }
            NcmError::FfmpegMissing { .. } => {
                "未找到 ffmpeg。安装方式：ffmpeg.org 下载后加入 PATH，或用 --ffmpeg-path 指定 ffmpeg 可执行文件位置。有损导出（MP3/AAC/Opus）依赖它。"
            }
            NcmError::UpgradeBlocked => {
                "有损源（MP3 等）转无损（FLAC/WAV）不会恢复已丢失的音质，属于伪升级，已拦截。如确有需要（如统一入库格式），加 --i-know-lossy-to-lossless。"
            }
            NcmError::OutputExists { .. } => {
                "目标文件已存在，本工具绝不覆盖。请更换输出目录，或先处理同名产物。"
            }
            NcmError::Config(_) => {
                "配置文件损坏或版本过高。修正内容，或删除 config.json 后由程序以默认配置重建（配置是可再生的）。"
            }
            NcmError::PluginAckRequired(_) => {
                "该插件属高风险类（如本地格式迁移），需先显式确认：musicforge plugins acknowledge <插件名>（或 GUI 等价操作）。"
            }
            NcmError::PluginNotFound(_) => {
                "插件运行时不可用。离线版不包含插件；请安装对应插件到白名单目录并启用后重试。"
            }
            _ => "请检查文件与目录权限后重试，或使用失败清单导出功能记录该文件。",
        }
    }
}
