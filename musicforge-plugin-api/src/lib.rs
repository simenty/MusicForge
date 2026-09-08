//! MusicForge 插件协议 crate（P6a 预研，D18/D20/X8）。
//!
//! 设计冻结来源：`docs/p6a-ai-interface.md` + 方案 v2.7.2。
//!
//! - **协议正典（X8）**：stdio 上的强类型 NDJSON——
//!   请求 `{"id","method","params"}`，响应 `{"id","ok","result"|"error":{code,message}}`；
//! - **D20**：`api_version` semver 区间协商（本预研实现 `>=M,<N` 与精确 `M` 子集）；
//! - **最小请求模型**：类型层面不存在 `absolute_path`/`audio_bytes`/`cover_bytes`
//!   字段——违反即编译不过，而非运行时审查；
//! - **三条红线**（P6a 硬验收）：AI 只建议不执行 / 最小请求 / 降级完整性。
//!
//! 本 crate **零网络、零进程管理**——那是 [`musicforge-plugin-host`] 的职责
//! （D12：能力分层，跨层下沉 = 架构缺陷）。

use serde::{Deserialize, Serialize};

/// 稳定错误码（插件域，P6a 起生效；详见 docs/result-codes.md）。
pub mod codes {
    pub const API_INCOMPATIBLE: &str = "MF-PLUGIN-API-INCOMPATIBLE";
    pub const TIMEOUT: &str = "MF-PLUGIN-TIMEOUT";
    pub const FAILED: &str = "MF-PLUGIN-FAILED";
    pub const METHOD_UNKNOWN: &str = "MF-PLUGIN-METHOD-UNKNOWN";
    pub const MANIFEST_INVALID: &str = "MF-PLUGIN-MANIFEST-INVALID";
    /// Host 侧支持的最大协议主版本（D20 区间的上界来源）
    pub const HOST_API_MAJOR: u64 = 1;
}

/// P6a-R（协议 v0.1，2026-09-09）：协议级常量与新增方法集。
///
/// 规格来源：`docs/plugin-protocol.md`（v0.1-draft 审查修订版）；
/// 全部字段可选向后兼容（R26 对策：不 bump api_version major）。
pub mod v1 {
    /// 当前协议版本（plugin.init 握手的 protocol_version）
    pub const PROTOCOL_VERSION: u64 = 1;
    /// 单条消息上限（16MB；Host 解析超限记违规并丢弃该行——P9 沙箱阶段再做流式强化）
    pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
    /// migrate 超时默认（10min；可配置）
    pub const MIGRATE_TIMEOUT_MS: u64 = 600_000;
    /// shutdown 优雅退出等待（超时 kill）
    pub const SHUTDOWN_GRACE_MS: u64 = 5_000;
    /// init 握手超时
    pub const INIT_TIMEOUT_MS: u64 = 10_000;
    /// 空闲回收阈值（Host 策略参考值）
    pub const IDLE_RECYCLE_MS: u64 = 5 * 60_000;
    /// 连续崩溃禁用阈值
    pub const CRASH_DISABLE_THRESHOLD: u32 = 3;
}

/// 事件方法名（X39：无 id、不要求响应；仅在存在进行中请求时可发）。
pub mod events {
    pub const PROGRESS: &str = "event.progress";
    pub const LOG: &str = "event.log";
}

/// 冻结方法集（`docs/p6a-ai-interface.md` §2；P6a 施工清单第 2 项）。
///
/// 命名空间约定：`plugin.*` = 生命周期管理；`ai.*` / `lyrics.*` / `cover.*` = 能力域。
pub mod methods {
    pub const PLUGIN_MANIFEST: &str = "plugin.manifest";
    pub const PLUGIN_HEALTH: &str = "plugin.health";
    pub const PLUGIN_SHUTDOWN: &str = "plugin.shutdown";
    /// P6a-R（X39）：首个且必须首个调用——协议协商 + work_dir 授权（legacy 插件
    /// 回 METHOD-UNKNOWN → Host 降级基础信封模式）
    pub const PLUGIN_INIT: &str = "plugin.init";
    pub const AI_IDENTIFY_TRACK: &str = "ai.identify_track";
    /// P1-4：批量变体（capabilities.batch 声明；≤100/次；Host 对未声明插件退化循环）
    pub const AI_IDENTIFY_TRACKS: &str = "ai.identify_tracks";
    /// 规则文本交回 core 执行（AI 只建议，执行权在 core）
    pub const AI_GENERATE_FILENAME_REGEX: &str = "ai.generate_filename_regex";
    /// D24：重复组保留建议（质量画像之外的语义判断）
    pub const AI_REVIEW_DUPLICATE_GROUP: &str = "ai.review_duplicate_group";
    /// 红线：绝不改歌手/歌名
    pub const LYRICS_VERIFY: &str = "lyrics.verify";
    /// P6a-R：刮削候选（lrc 内嵌 ≤1MB；禁止返回 URL 让 Host 下载）
    pub const LYRICS_SEARCH: &str = "lyrics.search";
    /// D22：封面来源优先级最末
    pub const COVER_SEARCH: &str = "cover.search";
    pub const COVER_GENERATE: &str = "cover.generate";
    /// P6b：本地格式迁移（L3；路径边界 + 双验 + 隔离 + 审计，见 RFC
    /// docs/rfc/p6b-format-plugin-framework.md；ack_required 清单申报 + ACK 闸）
    pub const FORMAT_MIGRATE: &str = "format.migrate";
    /// P6a-R（X40）：首 4KB + 尾 4KB 双采样变体识别（STag/QTag 尾标）
    pub const FORMAT_PROBE: &str = "format.probe";
    /// P6a-R：完整性校验（损坏显式报错）
    pub const FORMAT_VALIDATE: &str = "format.validate";
    /// P6a-R（P1-1）：清理 work_dir 临时文件后响应；5s 无响应 kill + Host 兜底
    pub const FORMAT_CANCEL: &str = "format.cancel";
}

// ---------------------------------------------------------------- 信封 --

/// 请求信封（X8 强类型 NDJSON）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Request {
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// 错误体（X42 双层模型：`code` = 传输/协议层 `MF-PLUGIN-*`；
/// `source_code` = 插件业务码透传，如 `QMC-EKEY-INVALID`——UI 展示原码与引导）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginError {
    pub code: String,
    pub message: String,
    /// P6a-R：业务层原码（缺省不序列化——向后兼容）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_code: Option<String>,
}

impl PluginError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            source_code: None,
        }
    }

    /// 带业务原码构造（X42）。
    pub fn with_source(
        code: &str,
        message: impl Into<String>,
        source_code: impl Into<String>,
    ) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            source_code: Some(source_code.into()),
        }
    }
}

/// 响应信封：`ok=true` 携带 `result`；`ok=false` 携带 `error`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Response {
    pub id: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<PluginError>,
}

impl Response {
    pub fn ok(id: &str, result: serde_json::Value) -> Self {
        Self {
            id: id.to_string(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: &str, code: &str, message: impl Into<String>) -> Self {
        Self {
            id: id.to_string(),
            ok: false,
            result: None,
            error: Some(PluginError::new(code, message)),
        }
    }
}

// ---------------------------------------------------------------- 清单 --

/// 插件种类（P6a-R 对齐协议 v0.1 §7 值域；`alias` 兼容 v0.7.0–v0.8.0 旧清单值）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PluginKind {
    #[serde(rename = "ai-provider", alias = "ai")]
    Ai,
    #[serde(rename = "lyrics-provider")]
    LyricsProvider,
    #[serde(rename = "cover-provider")]
    CoverProvider,
    #[serde(rename = "format-adapter", alias = "format")]
    FormatAdapter,
    #[serde(rename = "nas-adapter")]
    NasAdapter,
    #[serde(rename = "notification")]
    Notification,
}

/// plugin.json 权限清单（v2 契约）。
///
/// 准入规则（方案 §4.10）：`network == false` 对 format-adapter 强制；
/// `data_not_sent` 必须显式列出（最小请求模型的对偶声明）。
/// P6b 增补 `ack_required`（X35 规则：缺键默认 false，旧清单向后兼容）——
/// 为 true 的高风险插件（如格式迁移）必须经主程序确认闸
/// （`musicforge plugins acknowledge <id>` / GUI 等价）后方可调用，
/// 否则返回 `MF-PLUGIN-ACK-REQUIRED`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginManifest {
    pub name: String,
    /// 插件实现的协议版本，如 `"1.0.0"`（D20 区间协商的插件侧输入）
    pub api_version: String,
    pub kind: PluginKind,
    pub network: bool,
    #[serde(default)]
    pub data_sent: Vec<String>,
    #[serde(default)]
    pub data_not_sent: Vec<String>,
    #[serde(default)]
    pub ack_required: bool,
    /// P6b.2：格式插件能力声明——可迁移的容器扩展名（小写、不含点，如 "kwm"）。
    /// 加密容器无明文魔数，Host 按扩展名探测（PLUGIN_POLICY §4 逐格式兼容性申报）。
    #[serde(default)]
    pub extensions: Vec<String>,
    /// P6a-R（协议 v0.1 §7 权限清单）：`delete_source_file`/`move_source_file`/
    /// `upload_audio` 任一为 true → 宿主**拒载**（§4.10 准入；对抗断言钉死）。
    #[serde(default)]
    pub permissions: PluginPermissions,
}

/// 权限清单（§7）。铁律：`delete_source_file`/`move_source_file`/`upload_audio`
/// 三个位**永不合法**——声明即拒载（Host 加载时校验）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginPermissions {
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub read_audio_metadata: bool,
    /// true 仅 format-adapter 可申请
    #[serde(default)]
    pub read_audio_file: bool,
    #[serde(default)]
    pub write_tags: bool,
    #[serde(default)]
    pub delete_source_file: bool,
    #[serde(default)]
    pub move_source_file: bool,
    #[serde(default)]
    pub upload_audio: bool,
}

impl PluginPermissions {
    /// §4.10 准入：三禁位任一为 true → 拒载。
    pub fn has_forbidden(&self) -> bool {
        self.delete_source_file || self.move_source_file || self.upload_audio
    }
}

/// `format.migrate` 请求参数（P6a-R v0.1 形状，规格 §5.4；L3 域——路径为必要输入，
/// AI 域禁发 `absolute_path` 的规则不适用于本方法，但仅限 format 插件域）。
///
/// **v0.8.1 协同 breaking**（R26）：由 v0.7.0 平铺形状 `{work_root, source_path,
/// output_dir, ekey}` 升级为本形状——Host 与 4 个既有插件同批改造。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FormatMigrateParams {
    /// 任务标识（event.progress 的 job_id 关联，X39）
    pub job_id: String,
    /// 源文件绝对路径（Host 校验在授权目录内后转发）
    pub input_path: String,
    /// 产物预期路径（Host 校验在授权目录内）
    pub output_path: String,
    /// 插件唯一可写工作目录（= init 授权的 work_dir 或其子目录）
    pub work_dir: String,
    /// 选项（含 X38 ekey）
    #[serde(default)]
    pub options: MigrateOptions,
}

/// 产物双验结果（magic + 音频属性）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MigrateVerification {
    pub magic: String,
    #[serde(default)]
    pub sample_rate: Option<u32>,
    #[serde(default)]
    pub channels: Option<u16>,
    #[serde(default)]
    pub duration_s: Option<f64>,
}

/// 审计行（源/产物 sha256 + 隔离标记）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MigrateAudit {
    pub source_sha256: String,
    pub output_sha256: String,
    pub quarantined: bool,
}

/// `format.migrate` 结果（v0.1：artifacts 相对 work_dir 出站 X41 + 现框架双验/审计保留）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FormatMigrateResult {
    /// "success"（失败走 error 信封；本字段为终态语义标记）
    pub status: String,
    /// 产物格式（如 "flac"）
    pub output_format: String,
    /// X41：产物相对 work_dir 路径数组（Host resolve + 逃逸校验后取用）
    #[serde(default)]
    pub artifacts: Artifacts,
    /// 主产物绝对路径（现框架兼容；Host 优先信任 artifacts 校验结果）
    pub output_path: String,
    pub verification: MigrateVerification,
    pub audit: MigrateAudit,
    #[serde(default)]
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------- AI 域类型 --

/// 逐字段置信度（v2.7.2 细化：阈值判定可精确到字段级）。
pub type FieldConfidence = std::collections::BTreeMap<String, f32>;

/// `ai.identify_track` 的建议产物（X13：只建议，执行权在 core 的 Plan 层）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IdentifySuggestion {
    pub title: String,
    #[serde(default)]
    pub artists: Vec<String>,
    #[serde(default)]
    pub album: Option<String>,
    pub confidence: f32,
    #[serde(default)]
    pub field_confidence: FieldConfidence,
    pub reason: String,
}

// ---------------------------------------------------------------- 方法参数/结果 --

/// `ai.identify_track` 请求参数（协议形状冻结于 p6a-ai-interface.md §3）。
///
/// 最小请求模型：类型层面不存在 `absolute_path`/`audio_bytes`/`cover_bytes`。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct IdentifyTrackParams {
    pub normalized_filename: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub artists: Vec<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub language_hint: Option<String>,
}

/// `ai.generate_filename_regex` 请求参数：文件名样例集。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FilenameRegexParams {
    pub samples: Vec<String>,
}

/// `ai.generate_filename_regex` 结果：规则文本（执行权在 core，插件不碰文件系统）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FilenameRegexResult {
    pub rule: String,
    pub confidence: f32,
    pub reason: String,
}

/// 重复组成员画像（D24 语义判断的最小上下文；不含路径，仅元数据）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DuplicateMember {
    /// 文件名（含扩展名；禁发绝对路径）
    pub filename: String,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub bitrate_kbps: Option<u64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub size_bytes: Option<u64>,
}

/// `ai.review_duplicate_group` 请求参数。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DuplicateGroupParams {
    pub members: Vec<DuplicateMember>,
}

/// `ai.review_duplicate_group` 结果：建议保留项（下标）+ 理由。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DuplicateReviewResult {
    pub keep_index: usize,
    pub reason: String,
}

/// `lyrics.verify` 请求参数（红线：绝不改歌手/歌名——本方法只核验，不产出替换标题）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LyricsVerifyParams {
    pub title: String,
    pub artists: Vec<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// 歌词文本抽样（前 N 行），供语义比对
    pub lyrics_excerpt: String,
}

/// 歌词核验结论。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LyricsVerdict {
    Match,
    Mismatch,
    Uncertain,
}

/// `lyrics.verify` 结果：核验结论 + 候选（候选仅为歌词来源描述，不含标题/艺人替换）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LyricsVerifyResult {
    pub verdict: LyricsVerdict,
    pub confidence: f32,
    #[serde(default)]
    pub candidates: Vec<String>,
}

/// 封面候选（`cover.search` / `cover.generate` 共用）。
///
/// `image_ref` 为远端引用或本地缓存描述——禁发 `cover_bytes` 本体。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CoverCandidate {
    pub source: String,
    pub image_ref: String,
    #[serde(default)]
    pub width_px: Option<u32>,
    #[serde(default)]
    pub height_px: Option<u32>,
    pub confidence: f32,
}

/// 封面域查询参数（D22：来源优先级最末，仅在无达标内嵌时由 core 发起）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CoverQueryParams {
    pub title: String,
    #[serde(default)]
    pub artists: Vec<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub style_hint: Option<String>,
}

/// `cover.search` / `cover.generate` 结果。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CoverResult {
    pub candidates: Vec<CoverCandidate>,
}

/// `plugin.health` 结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthResult {
    pub status: String,
    #[serde(default)]
    pub uptime_ms: Option<u64>,
}

/// `plugin.shutdown` 结果（插件应答后自行退出）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShutdownResult {
    pub accepted: bool,
}

// ---------------------------------------------------------------- v0.1 新增（P6a-R） --

/// `plugin.init` 请求参数（X39 握手；规格 §4.1）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InitParams {
    /// 协议版本（本 Host 恒 [`v1::PROTOCOL_VERSION`](v1::PROTOCOL_VERSION)）
    pub protocol_version: u64,
    /// 插件唯一可写目录（§9 出站资源边界；Host 保证存在）
    pub work_dir: String,
    /// 用户区域（如 "zh-CN"）
    #[serde(default)]
    pub locale: String,
    /// Host 能力声明（插件按位裁剪行为）
    #[serde(default)]
    pub host_capabilities: HostCapabilities,
}

/// Host 能力位（§4.1；全部缺省 false——新增能力向后兼容）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostCapabilities {
    #[serde(default)]
    pub batch: bool,
    #[serde(default)]
    pub events: bool,
    #[serde(default)]
    pub artifacts: bool,
}

/// `plugin.init` 结果：api_version 区间 + 与 plugin.json 同构的清单。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InitResult {
    /// 插件支持的协议区间（D20；Host 复核，双源一致性）
    pub api_version: String,
    #[serde(default)]
    pub manifest: Option<PluginManifest>,
}

/// `format.probe` 请求参数（X40：首 4KB + 尾 4KB 双采样；规格 §5.4）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ProbeParams {
    pub file_name: String,
    pub extension: String,
    #[serde(default)]
    pub size_bytes: u64,
    /// 文件头 hex 采样（≤4KB）
    #[serde(default)]
    pub header_hex: String,
    /// 文件尾 hex 采样（≤4KB；STag/QTag 尾标在此）
    #[serde(default)]
    pub tail_hex: String,
}

/// `format.probe` 结果：变体判定 + ekey 需求分流（RFC-0002）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProbeResult {
    pub supported: bool,
    #[serde(default)]
    pub format_id: Option<String>,
    #[serde(default)]
    pub estimated_output: Option<String>,
    pub confidence: f32,
    /// 高风险域恒 true（D10）
    #[serde(default)]
    pub requires_acknowledgement: bool,
    #[serde(default)]
    pub risk_level: Option<String>,
    /// X38：需要用户自备 ekey（尾标变体）
    #[serde(default)]
    pub requires_ekey: bool,
    /// X38：提取引导文案
    #[serde(default)]
    pub ekey_hint: Option<String>,
}

/// `format.validate` 结果（损坏显式报错，绝不静默产出损坏音频）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ValidateResult {
    pub valid: bool,
    #[serde(default)]
    pub issues: Vec<String>,
}

/// `format.cancel` 结果（P1-1：插件清理自身临时文件后响应）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CancelResult {
    pub cancelled: bool,
}

/// `format.migrate` 选项（X38：`ekey` 用户自备；本地传递非网络）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MigrateOptions {
    #[serde(default = "default_true")]
    pub preserve_source: bool,
    #[serde(default)]
    pub overwrite: bool,
    #[serde(default = "default_true")]
    pub verify_output: bool,
    /// QMCv2 类用户自备密钥
    #[serde(default)]
    pub ekey: Option<String>,
}

impl Default for MigrateOptions {
    fn default() -> Self {
        // 与 serde 缺省一致：保源 + 验产物为默认安全语义
        Self {
            preserve_source: true,
            overwrite: false,
            verify_output: true,
            ekey: None,
        }
    }
}

fn default_true() -> bool {
    true
}

/// `lyrics.search` 候选（规格 §5.3：lrc 文本内嵌 ≤1MB；禁止返回 URL）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LyricsCandidate {
    pub provider: String,
    pub score: f32,
    #[serde(default)]
    pub synced: bool,
    /// LRC 全文（≤1MB；禁止 URL）
    pub lrc: String,
}

/// `lyrics.search` 参数。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LyricsSearchParams {
    pub title: String,
    #[serde(default)]
    pub artists: Vec<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub language_hint: Option<String>,
}

/// `lyrics.search` 结果。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LyricsSearchResult {
    pub candidates: Vec<LyricsCandidate>,
}

/// X41 出站资源：响应中的**相对 work_dir 路径**数组。
///
/// Host resolve 后校验仍在 work_dir 内（拒 `../`/绝对路径/符号链接逃逸），
/// 校验失败 = 协议违规，产物不取用。
pub type Artifacts = Vec<String>;

// ---------------------------------------------------------------- D20 协商 --

/// D20：插件 `api_version` 是否落在 Host 支持区间内。
///
/// 本预研支持的区间语法子集：`">=M,<N"`（标准形态）与精确 `"M"`。
/// 语义：插件主版本 M 满足 `lower <= M < upper`（缺上界视为无上界）。
/// 完整 semver 表达式在 P6a 正式期评估 `semver` crate（MIT，需入白名单）。
pub fn api_compatible(plugin_api_version: &str, host_range: &str) -> bool {
    let plugin_major: u64 = match plugin_api_version.split('.').next().unwrap_or("").parse() {
        Ok(m) => m,
        Err(_) => return false,
    };
    let range = host_range.trim();
    if let Ok(major) = range.parse::<u64>() {
        return plugin_major == major;
    }
    // 稳定审计修复（2026-09-08）：此前约束解析失败（如 "1.0.0"/"garbage"）时
    // lower 默认 0 且无上界 → 隐式放行一切插件。修复为：未解析出任何合法约束
    // 或出现未知形态 → 显式不兼容（拒绝优于放行）。
    let mut lower: Option<u64> = None;
    let mut upper: Option<u64> = None;
    let mut saw_constraint = false;
    for part in range.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue; // 容忍尾部逗号的空白片段
        }
        if let Some(v) = part.strip_prefix(">=") {
            match v.trim().parse::<u64>() {
                Ok(n) => {
                    lower = Some(n);
                    saw_constraint = true;
                }
                Err(_) => return false,
            }
        } else if let Some(v) = part.strip_prefix('<') {
            match v.trim().parse::<u64>() {
                Ok(n) => {
                    upper = Some(n);
                    saw_constraint = true;
                }
                Err(_) => return false,
            }
        } else {
            return false; // 未知约束形态 → 显式不兼容
        }
    }
    if !saw_constraint {
        return false;
    }
    plugin_major >= lower.unwrap_or(0) && upper.map(|u| plugin_major < u).unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_roundtrip() {
        let req = Request {
            id: "req-1".into(),
            method: "ai.identify_track".into(),
            params: serde_json::json!({"normalized_filename": "x.wav"}),
        };
        let line = serde_json::to_string(&req).unwrap();
        let back: Request = serde_json::from_str(&line).unwrap();
        assert_eq!(back, req);

        let resp = Response::err("req-1", codes::METHOD_UNKNOWN, "no such method");
        let line = serde_json::to_string(&resp).unwrap();
        assert!(line.contains(r#""ok":false"#));
        let back: Response = serde_json::from_str(&line).unwrap();
        assert_eq!(back.error.unwrap().code, codes::METHOD_UNKNOWN);
    }

    #[test]
    fn d20_range_negotiation() {
        assert!(api_compatible("1.0.0", ">=1,<2"));
        assert!(api_compatible("1.9.9", ">=1,<2"));
        assert!(!api_compatible("2.0.0", ">=1,<2"));
        assert!(!api_compatible("0.9.0", ">=1,<2"));
        assert!(api_compatible("1.0.0", "1"));
        assert!(!api_compatible("2.0.0", "1"));
        assert!(!api_compatible("garbage", ">=1,<2"));
        // 稳定审计 B4：host_range 本身不可解析 → 必须显式拒绝（此前隐式放行一切）
        assert!(!api_compatible("1.0.0", "garbage"), "垃圾区间不得放行");
        assert!(
            !api_compatible("1.0.0", "1.0.0"),
            "完整 semver 误入区间位 → 拒绝而非放行"
        );
        assert!(!api_compatible("1.0.0", ">=x,<2"), "约束数值非法 → 拒绝");
        assert!(
            api_compatible("1.9.0", ">=1"),
            "仅下界（无上界）仍是合法形态"
        );
        assert!(
            api_compatible("2.0.0", ">=1"),
            "仅下界（无上界）→ 2.x 合法（与原实现语义一致）"
        );
    }

    #[test]
    fn manifest_parses() {
        let m: PluginManifest = serde_json::from_str(
            r#"{"name":"mock-ai","api_version":"1.0.0","kind":"ai","network":false,
                "data_sent":["normalized_filename","title","artists","album","duration_ms","format","language_hint"],
                "data_not_sent":["absolute_path","audio_bytes","cover_bytes"]}"#,
        )
        .unwrap();
        assert_eq!(m.kind, PluginKind::Ai);
        assert!(!m.network);
        assert!(!m.ack_required, "缺省 ack_required 必须 false（X35 兼容）");
        assert!(m.data_not_sent.contains(&"absolute_path".to_string()));
    }

    #[test]
    fn manifest_ack_required_opt_in() {
        let m: PluginManifest = serde_json::from_str(
            r#"{"name":"kwm-migration","api_version":"1.0.0","kind":"format-adapter",
                "network":false,"ack_required":true,"extensions":["kwm"]}"#,
        )
        .unwrap();
        assert!(m.ack_required, "高风险插件显式申报 ACK 闸");
        assert_eq!(
            m.extensions,
            vec!["kwm".to_string()],
            "能力声明：可迁移扩展名"
        );
        assert!(m.data_not_sent.is_empty());
    }

    #[test]
    fn format_migrate_types_roundtrip() {
        let params = FormatMigrateParams {
            job_id: "job-1".into(),
            input_path: "C:/music/song.kwm".into(),
            output_path: "C:/music/out/song.flac".into(),
            work_dir: "C:/music/.musicforge/work/job-1".into(),
            options: MigrateOptions {
                ekey: Some("k".into()),
                ..Default::default()
            },
        };
        let v = serde_json::to_value(&params).unwrap();
        assert_eq!(v["job_id"], "job-1");
        assert_eq!(v["options"]["ekey"], "k");
        assert_eq!(v["options"]["preserve_source"], true, "缺省 true");
        let back: FormatMigrateParams = serde_json::from_value(v).unwrap();
        assert_eq!(back, params);

        let result = FormatMigrateResult {
            status: "success".into(),
            output_format: "flac".into(),
            artifacts: vec!["out.flac".into()],
            output_path: "C:/music/out/song.flac".into(),
            verification: MigrateVerification {
                magic: "fLaC".into(),
                sample_rate: Some(44100),
                channels: Some(2),
                duration_s: Some(252.0),
            },
            audit: MigrateAudit {
                source_sha256: "a".repeat(64),
                output_sha256: "b".repeat(64),
                quarantined: false,
            },
            warnings: vec![],
        };
        let back: FormatMigrateResult =
            serde_json::from_value(serde_json::to_value(&result).unwrap()).unwrap();
        assert_eq!(back, result);
    }

    #[test]
    fn v1_init_and_error_source_code() {
        // X42：source_code 透传 + 缺省不序列化
        let e = PluginError::with_source(codes::FAILED, "ekey 校验失败", "QMC-EKEY-INVALID");
        let line = serde_json::to_string(&e).unwrap();
        assert!(line.contains("QMC-EKEY-INVALID"));
        let plain = PluginError::new(codes::FAILED, "x");
        assert!(
            !serde_json::to_string(&plain)
                .unwrap()
                .contains("source_code"),
            "无业务码时不得序列化 source_code"
        );

        // X39：init 参数/结果 roundtrip
        let init = InitParams {
            protocol_version: v1::PROTOCOL_VERSION,
            work_dir: "/w/job-1".into(),
            locale: "zh-CN".into(),
            host_capabilities: HostCapabilities {
                batch: true,
                events: true,
                artifacts: true,
            },
        };
        let back: InitParams =
            serde_json::from_value(serde_json::to_value(&init).unwrap()).unwrap();
        assert_eq!(back, init);

        // X40：probe 双采样 + ekey 分流
        let probe = ProbeParams {
            file_name: "song.mflac0".into(),
            extension: ".mflac0".into(),
            size_bytes: 48_213_311,
            header_hex: "66 4C".into(),
            tail_hex: "53 54 61 67".into(),
        };
        let back: ProbeParams =
            serde_json::from_value(serde_json::to_value(&probe).unwrap()).unwrap();
        assert_eq!(back.tail_hex, "53 54 61 67");

        // kind 值域：旧清单值 alias 兼容（R26 向后兼容）
        let legacy: PluginManifest = serde_json::from_str(
            r#"{"name":"mock-ai","api_version":"1.0.0","kind":"ai","network":false}"#,
        )
        .unwrap();
        assert_eq!(legacy.kind, PluginKind::Ai);
        let modern: PluginManifest = serde_json::from_str(
            r#"{"name":"ly","api_version":"1.0.0","kind":"lyrics-provider","network":true}"#,
        )
        .unwrap();
        assert_eq!(modern.kind, PluginKind::LyricsProvider);
    }
}
