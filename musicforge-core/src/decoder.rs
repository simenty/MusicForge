//! 流式解码器（io::Read 语义；硬约束 2/5/8 的运行时载体）

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::crypto::xor_stream;
use crate::error::NcmError;
use crate::format::{self, Format};
use crate::header::{self, Header};
use crate::metadata::Metadata;

/// 打开 `.ncm` 并完成头部解析 + CRC 校验。此后即可 `Read` 得到**明文**音频流。
pub struct Decoder {
    file: std::fs::File,
    header: Header,
    /// 全局音频偏移（硬约束 2：跨块续读正确性的来源）
    stream_offset: u64,
    format: Option<Format>,
    source_stem: Option<String>,
    source_parent: Option<PathBuf>,
}

impl Decoder {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, NcmError> {
        let path = path.as_ref();
        let mut file = std::fs::File::open(path)?;
        let header = header::parse_stream(&mut file)?;
        // QA-G5（K2 修复揭开的潜伏缺陷）：stream_offset 是**密钥流全局偏移**，
        // open 后文件位置恰在音频区首字节 → 偏移必须为 0。此前误赋 audio_offset
        // （文件偏移）→ detect_format 首读密钥流整体错位，读出的"头"是垃圾——
        // 一直被「兜底 flac」掩盖（K2 显式报错后暴露）。dump_to/detect_format
        // 尾部的 seek+归零只是补丁，首读仍错。
        let stream_offset = 0u64;
        let source_stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string());
        let source_parent = path.parent().map(|p| p.to_path_buf());
        Ok(Self { file, header, stream_offset, format: None, source_stem, source_parent })
    }

    /// 元数据（可能为 `None`——硬约束 11）
    pub fn metadata(&self) -> Option<&Metadata> {
        self.header.metadata.as_ref()
    }

    /// 内嵌封面（3.x 文件为空切片）
    pub fn cover(&self) -> &[u8] {
        &self.header.cover
    }

    /// 预期明文总长（= 文件长 - 头长）
    pub fn audio_len(&self) -> u64 {
        self.header.audio_len
    }

    /// 格式判定（三级融合：metadata.format 优先 → 魔数兜底；读取后回退文件位置）
    pub fn detect_format(&mut self) -> Result<Format, NcmError> {
        if let Some(f) = self.format {
            return Ok(f);
        }
        let mut head = [0u8; 16];
        let n = self.read(&mut head)?;
        self.file.seek(SeekFrom::Start(self.header.audio_offset))?;
        self.stream_offset = 0; // 回到音频起点：密钥流偏移归零
        let f =         // K2 修复（QA 审计决策项）：三级皆不可判 → 显式报错而非兜底 flac。
        // 兜底产出假 .flac 扩展名的垃圾文件 = 谎报成功（上游 B-3 同类），
        // 与硬约束 9「拒绝产出损坏音频」冲突。CLI/GUI 按单文件 Failed 处理，批次继续。
        format::resolve(
            self.header.metadata.as_ref().and_then(|m| m.format.as_deref()),
            &head[..n],
        )
        .ok_or(NcmError::UnknownFormat)?;
        self.format = Some(f);
        Ok(f)
    }

    /// 解密并落盘到**源目录**（硬约束 5：写后完整性校验；写失败清理半成品——修上游 B-3 谎报成功）
    pub fn dump(&mut self, out_dir: Option<&Path>) -> Result<PathBuf, NcmError> {
        let fmt = self.detect_format()?;
        let stem = self.source_stem.clone().unwrap_or_else(|| "musicforge-output".to_string());

        let target = match out_dir {
            Some(dir) => {
                std::fs::create_dir_all(dir)?;
                dir.join(format!("{stem}.{}", fmt.extension()))
            }
            None => {
                let parent = self.source_parent.clone().unwrap_or_else(|| PathBuf::from("."));
                parent.join(format!("{stem}.{}", fmt.extension()))
            }
        };
        self.dump_to(&target)?;
        Ok(target)
    }

    /// 解密并落盘到**指定完整路径**（CLI 结构保留模式用；扩展名由调用方按格式计算）
    pub fn dump_to(&mut self, target: &Path) -> Result<(), NcmError> {
        // 回到音频起点（detect_format 可能前探过）
        self.file.seek(SeekFrom::Start(self.header.audio_offset))?;
        self.stream_offset = 0; // 密钥流偏移归零

        let mut out = std::fs::File::create(target)?;
        // QA-B4：任何失败路径（含写盘中途 IO 错误）都清理半成品——此前仅
        // OutputIntegrity 路径清理，磁盘满/设备拔出会留下残缺文件
        let result = (|| -> Result<(), NcmError> {
            let mut buf = vec![0u8; 0x8000];
            let mut written: u64 = 0;
            loop {
                let n = self.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n])?;
                written += n as u64;
            }
            out.flush()?;

            // 硬约束 5：落盘字节数校验（修上游 B-3 谎报成功）
            if written != self.header.audio_len {
                return Err(NcmError::OutputIntegrity { written, expected: self.header.audio_len });
            }
            Ok(())
        })();
        match result {
            Ok(()) => Ok(()),
            Err(e) => {
                drop(out);
                let _ = std::fs::remove_file(target);
                Err(e)
            }
        }
    }
}

impl Read for Decoder {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.file.read(buf)?;
        if n == 0 {
            return Ok(0);
        }
        xor_stream(&mut buf[..n], &self.header.key_box, self.stream_offset);
        self.stream_offset += n as u64;
        Ok(n)
    }
}
