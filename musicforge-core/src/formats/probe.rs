//! 格式判定（硬约束 8：三级融合——metadata.format 优先 → 魔数兜底 → 解析终检）
//!
//! 实证依据（本区 fixtures）：裸 MP3（`ff fb` 帧同步字，无 ID3v2 头）被上游盲判为 flac——
//! metadata.format **必须**优先于魔数；魔数只做 metadata 缺失时的兜底。
//! 终检（解析容器头确认）在 tagger/应用层执行，本库 Day-1 留 TODO。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Mp3,
    Flac,
    M4a,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Mp3 => "mp3",
            Format::Flac => "flac",
            Format::M4a => "m4a",
        }
    }

    /// 从元数据声明的字符串解析（容忍大小写与未知值）
    pub fn from_metadata(s: &str) -> Option<Format> {
        match s.to_ascii_lowercase().as_str() {
            "mp3" => Some(Format::Mp3),
            "flac" => Some(Format::Flac),
            "m4a" | "mp4" => Some(Format::M4a),
            _ => None,
        }
    }
}

/// 魔数嗅探（兜底级）：返回 `None` 表示无法从魔数识别
pub fn sniff_magic(head: &[u8]) -> Option<Format> {
    if head.len() >= 3 && &head[..3] == b"ID3" {
        return Some(Format::Mp3);
    }
    if head.len() >= 4 && &head[..4] == b"fLaC" {
        return Some(Format::Flac);
    }
    // MPEG 帧同步字（裸 MP3，无 ID3v2 头）：11111111 111x / 11111111 101x 等
    if head.len() >= 2 && head[0] == 0xff && (head[1] & 0xe0) == 0xe0 {
        return Some(Format::Mp3);
    }
    // MP4：offset 4..8 == "ftyp"
    if head.len() >= 8 && &head[4..8] == b"ftyp" {
        return Some(Format::M4a);
    }
    None
}

/// 三级融合判定（硬约束 8）：
/// 1. metadata.format 存在且可识别 → 直接采信（第一级）
/// 2. 否则魔数嗅探（第二级）
/// 3. 都失败 → `None`（应用层按未知格式处理；解析终检为 P1 后续项，见模块注释）
pub fn resolve(meta_format: Option<&str>, decrypted_head: &[u8]) -> Option<Format> {
    meta_format
        .and_then(Format::from_metadata)
        .or_else(|| sniff_magic(decrypted_head))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归实证：裸 MP3（ff fb，无 ID3v2）——上游 Go 版盲判 flac 的缺陷不得复现
    #[test]
    fn bare_mp3_resolves_via_metadata() {
        let head = [0xff, 0xfb, 0x90, 0x64];
        assert_eq!(resolve(Some("mp3"), &head), Some(Format::Mp3));
        assert_eq!(resolve(Some("flac"), &head), Some(Format::Flac)); // metadata 优先于魔数
        assert_eq!(resolve(None, &head), Some(Format::Mp3)); // 魔数兜底（MPEG 同步字）
    }

    #[test]
    fn magic_fallback() {
        assert_eq!(resolve(None, b"fLaCxxxx"), Some(Format::Flac));
        assert_eq!(resolve(None, b"ID3\x03xx"), Some(Format::Mp3));
        assert_eq!(
            resolve(None, b"\x00\x00\x00\x18ftypM4A "),
            Some(Format::M4a)
        );
        assert_eq!(resolve(None, b"\x00\x00\x00\x00"), None);
    }
}
