//! 格式适配层（P1c：定义抽象 + 完成 NCM 适配，**不切换主调用路径**）。
//!
//! 绞杀者重构的顺序是「先让新路径与旧路径并存，再用等价性测试证明一致，
//! 最后才切换调用方」。因此本阶段：
//!
//! - 只新增 [`FormatAdapter`] / [`FormatRegistry`] / [`NcmAdapter`]；
//! - CLI 与 GUI 仍走旧路径（`crate::decoder::Decoder`），本模块不被主流程调用；
//! - `tests/p1c_adapter_equivalence.rs` 证明两条路径对同一 fixture 产出
//!   **字节级一致的音频负载**与相同的元数据字段。
//!
//! P1d 才把 CLI 切到 `FormatRegistry`；P1e 统一错误码。

use std::path::{Path, PathBuf};

use crate::decoder::Decoder;
use crate::error::NcmError;
use crate::format::Format;
use crate::metadata::Metadata;
use crate::MAGIC;

/// 探测输入：只暴露「扩展名 + 文件头」，不暴露全路径。
///
/// 这样将来对内存数据 / 非文件来源做探测时无需改动 trait。
pub struct ProbeInput<'a> {
    pub extension: Option<&'a str>,
    pub header: &'a [u8],
}

/// 探测结果。`confidence < 1.0` 表示「只是扩展名匹配、魔数不符」——
/// 调用方不应据此直接解码，而应交给旧路径去给出明确错误（硬约束 9）。
#[derive(Debug, PartialEq)]
pub struct ProbeResult {
    pub format_id: &'static str,
    pub confidence: f32,
}

/// 解封装产物：只描述结果，不负责写标签、不改动源文件。
#[derive(Debug)]
pub struct DecodedAudio {
    pub path: PathBuf,
    pub format: Format,
    pub audio_len: u64,
    pub metadata: Option<Metadata>,
}

/// 格式适配器。一个格式 = 一个实现；新增格式只需新增实现并注册。
pub trait FormatAdapter {
    /// 稳定格式标识（如 `"ncm"`）。
    fn id(&self) -> &'static str;

    /// 依据扩展名与文件头判断是否支持，返回置信度。
    fn probe(&self, input: &ProbeInput<'_>) -> Option<ProbeResult>;

    /// 解封装到 `out_dir` 下，返回产物与元数据。**不写标签**（标签由 CLI 决定）。
    fn decode(&self, input: &Path, out_dir: &Path) -> Result<DecodedAudio, NcmError>;
}

/// NCM 适配器：当前唯一实现，内部复用既有 `Decoder`（零算法重复）。
pub struct NcmAdapter;

impl FormatAdapter for NcmAdapter {
    fn id(&self) -> &'static str {
        "ncm"
    }

    fn probe(&self, input: &ProbeInput<'_>) -> Option<ProbeResult> {
        // 只认魔数，不认扩展名：靠 `.ncm` 后缀认领会把「损坏/非 ncm 文件」
        // 伪装成可处理对象，正是 G5 教训（兜底伪装成功）。认领不了就交给
        // 旧路径产出明确错误（NCM-BAD-MAGIC / NCM-FORMAT-UNKNOWN，硬约束 9）。
        let magic_matches = input.header.len() >= MAGIC.len() && input.header[..MAGIC.len()] == MAGIC;

        if magic_matches {
            Some(ProbeResult {
                format_id: "ncm",
                confidence: 1.0,
            })
        } else {
            None
        }
    }

    fn decode(&self, input: &Path, out_dir: &Path) -> Result<DecodedAudio, NcmError> {
        let mut decoder = Decoder::open(input)?;
        let format = decoder.detect_format()?;
        let audio_len = decoder.audio_len();
        let metadata = decoder.metadata().cloned();

        std::fs::create_dir_all(out_dir)?;
        let stem = input
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("output");
        let target = out_dir.join(format!("{stem}.{}", format.extension()));
        decoder.dump_to(&target)?;

        Ok(DecodedAudio {
            path: target,
            format,
            audio_len,
            metadata,
        })
    }
}

/// 格式注册表：内置适配器在前，外部注册在后，命中即返回。
pub struct FormatRegistry {
    adapters: Vec<Box<dyn FormatAdapter>>,
}

impl FormatRegistry {
    /// 空注册表（测试用 / 只想要外部插件的场景）。
    pub fn new() -> Self {
        Self {
            adapters: Vec::new(),
        }
    }

    /// 内置适配器注册表（当前只有 NCM；新格式在此登记）。
    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(NcmAdapter));
        registry
    }

    /// 注册一个适配器（P6b 起：外部格式插件经 `PluginFormatAdapter` 走同一入口）。
    pub fn register(&mut self, adapter: Box<dyn FormatAdapter>) {
        self.adapters.push(adapter);
    }

    /// 按探测输入找到第一个支持的适配器。
    pub fn detect<'a>(&'a self, input: &ProbeInput<'_>) -> Option<&'a dyn FormatAdapter> {
        self.adapters
            .iter()
            .find(|a| a.probe(input).is_some())
            .map(|a| a.as_ref())
    }

    /// 便捷入口：读文件头后按路径探测。
    pub fn detect_file<'a>(&'a self, path: &Path) -> Option<&'a dyn FormatAdapter> {
        let header = read_header(path).ok()?;
        self.detect(&ProbeInput {
            extension: path.extension().and_then(|e| e.to_str()),
            header: &header,
        })
    }

    /// 已注册的适配器数量（测试与诊断用）。
    pub fn len(&self) -> usize {
        self.adapters.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.adapters.is_empty()
    }
}

impl Default for FormatRegistry {
    fn default() -> Self {
        Self::with_builtins()
    }
}

/// 读取文件前若干字节用于探测。读失败返回 `Err`，由调用方决定降级策略。
fn read_header(path: &Path) -> Result<Vec<u8>, NcmError> {
    use std::io::Read;
    let mut buf = vec![0u8; MAGIC.len().max(64)];
    let mut file = std::fs::File::open(path)?;
    let mut read = 0usize;
    while read < buf.len() {
        match file.read(&mut buf[read..])? {
            0 => break,
            n => read += n,
        }
    }
    buf.truncate(read);
    Ok(buf)
}
