//! 元数据（硬约束 11：缺失 → `None`，调用方跳过打标签；绝不因元数据问题失败整个转换）

use serde_json::Value;

/// 从 `.ncm` 元数据 JSON 提取的字段（全部允许缺失——真实 3.x 文件字段可缺）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Metadata {
    pub name: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// 元数据声明的格式（"mp3"/"flac"/"m4a"...），三级格式判定的第一优先级
    pub format: Option<String>,
    pub track: Option<u64>,
    pub bitrate: Option<u64>,
    pub duration: Option<u64>,
    /// 专辑封面 URL（本库**不下载**——硬约束 6：零网络；仅供调用方自行决策）
    pub album_pic_url: Option<String>,
}

/// 从解密后的 JSON 文本解析。字段缺失/类型不符一律容忍（不失败）。
pub fn parse(json_text: &[u8]) -> Result<Metadata, crate::error::NcmError> {
    let text = std::str::from_utf8(json_text).map_err(|e| {
        crate::error::NcmError::MetadataJson(serde_json::Error::io(std::io::Error::new(
            std::io::ErrorKind::InvalidData, e)))
    })?;
    let v: Value = serde_json::from_str(text)?;
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(|s| s.to_string());
    Ok(Metadata {
        name: s("musicName"),
        artist: v.get("artist").and_then(|a| a.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|a| a.get(0).and_then(|x| x.as_str()))
                .collect::<Vec<_>>()
                .join("/")
        }),
        album: s("album"),
        format: s("format"),
        track: v.get("track").and_then(|x| x.as_u64()),
        bitrate: v.get("bitrate").and_then(|x| x.as_u64()),
        duration: v.get("duration").and_then(|x| x.as_u64()),
        album_pic_url: s("albumPic").filter(|s| !s.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F1 根因守卫：坏 UTF-8 / 非法 JSON 必须返回 `Err`。
    /// 调用方（header.rs）以 `.ok()` 降级为 `None`，使「元数据损坏」不再整文件 Failed
    /// （硬约束 11）。若此函数改为吞错返回 `Ok`，F1 防护形同虚设。
    #[test]
    fn parse_rejects_garbage_metadata() {
        // 非法 UTF-8（截断的多字节序列）
        assert!(parse(&[0xff, 0xfe, 0x00]).is_err(), "非法 UTF-8 应 Err");
        // 合法 UTF-8 但非法 JSON
        assert!(parse(b"not valid json \x00\x01").is_err(), "非法 JSON 应 Err");
    }

    #[test]
    fn parse_tolerates_fields() {
        let m = parse(br#"{"musicName":"bei","artist":[["li"]],"album":"er"}"#).unwrap();
        assert_eq!(m.name.as_deref(), Some("bei"));
        assert_eq!(m.artist.as_deref(), Some("li"));
        assert_eq!(m.album.as_deref(), Some("er"));
    }
}
