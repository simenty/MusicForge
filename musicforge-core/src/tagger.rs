//! tagger 层（硬约束 7 / 方案书 §6）：lofty 写标签，与解密解耦。
//!
//! 语义（方案书 §8，对齐竞品缺口分析）：
//! - **不覆盖已有值**：title/artist/album 仅在目标文件缺失该字段时写入
//! - **封面不覆盖**：仅当目标文件无任何内嵌图片时嵌入
//! - 元数据缺失（None）→ 本层不适用（调用方跳过，硬约束 11）
//!
//! API 对齐 lofty 0.25（0.x 大版本重构：Picture 走 Builder、save_to_path 带 WriteOptions）

use std::path::Path;

use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::*;
use lofty::tag::{ItemKey, Tag};
use lofty::config::WriteOptions;

use crate::error::NcmError;
use crate::format::Format;
use crate::metadata::Metadata;

/// 字段是否算「已有值」。
///
/// 空串/纯空白按**缺失**处理：ID3v1 之类定长标签常见「字段存在但内容全空」，
/// 若把 `Some("")` 当成已有值就会跳过写入 —— 元数据静默丢失。
fn has_value(tag: &Tag, key: &ItemKey) -> bool {
    tag.get_string(*key).is_some_and(|s| !s.trim().is_empty())
}

/// 把解码得到的元数据与封面写入目标音频文件（不覆盖已有值）。
///
/// 返回 (写入的字段数, 是否嵌入了封面) —— 供调用方输出可观测信息。
///
/// 写入目标恒为 lofty 按扩展名判定的**容器主标签类型**（MP3→ID3v2、FLAC→Vorbis
/// Comments、M4A→MP4 ilst）。绝不写到 `first_tag_mut()` 碰巧给出的次要标签上：
/// 只带 ID3v1 的 MP3 会落到 ID3v1 —— 它不支持图片、字段上限 30 字符，lofty 写回时
/// **静默丢弃图片、静默截断文本**，而本函数照常返回 `Ok((1, true))`，即谎报
/// 「已嵌入封面」。配合 sidecar 完整性标记与 `--skip-existing`，这类文件会被
/// 永久跳过：元数据永久丢失且全程零报错（QA 第二轮 B7）。
pub fn write_tags(
    target: &Path,
    fmt: Format,
    meta: &Metadata,
    cover: &[u8],
) -> Result<(usize, bool), NcmError> {
    let mut tagged =
        lofty::read_from_path(target).map_err(|e| NcmError::TagRead(e.to_string()))?;

    let tag_type = tagged.primary_tag_type();
    if tagged.tag(tag_type).is_none() {
        tagged.insert_tag(Tag::new(tag_type));
    }
    // `insert_tag` 对「容器不支持该标签类型」是**静默无操作**（返回 None 而不报错），
    // `save_to` 对「只读标签类型」是**静默跳过**。两种情况都必须显式转成错误，
    // 否则就会出现「报成功但磁盘上什么都没变」。这里是硬约束 1（零 panic）与
    // 「拒绝谎报成功」的交汇点：用 Err 取代原先的 `.expect(...)`。
    if tagged.tag(tag_type).is_none() || !tagged.tag_support(tag_type).is_writable() {
        return Err(NcmError::TagWrite(format!(
            "容器 {:?} 不支持写入 {:?} 标签（输出 {}，元数据声明格式 {}）",
            tagged.file_type(),
            tag_type,
            target.display(),
            fmt.extension()
        )));
    }
    let tag = match tagged.tag_mut(tag_type) {
        Some(t) => t,
        None => {
            return Err(NcmError::TagWrite(format!(
                "无法为输出 {} 创建 {:?} 标签（容器 {:?}）",
                target.display(),
                tag_type,
                tagged.file_type()
            )))
        }
    };

    let mut written = 0usize;

    // 文本字段：仅缺失时写（不覆盖已有值——竞品 A/B 的 MP3 分支强制覆盖是反面教材）
    if let Some(name) = &meta.name {
        if !has_value(tag, &ItemKey::TrackTitle) {
            tag.insert_text(ItemKey::TrackTitle, name.clone());
            written += 1;
        }
    }
    if let Some(artist) = &meta.artist {
        if !has_value(tag, &ItemKey::TrackArtist) && !has_value(tag, &ItemKey::AlbumArtist) {
            tag.insert_text(ItemKey::TrackArtist, artist.clone());
            written += 1;
        }
    }
    if let Some(album) = &meta.album {
        if !has_value(tag, &ItemKey::AlbumTitle) {
            tag.insert_text(ItemKey::AlbumTitle, album.clone());
            written += 1;
        }
    }

    // 封面：仅当无任何图片时嵌入（不覆盖已有封面）
    let mut embedded = false;
    if !cover.is_empty() && tag.pictures().is_empty() {
        let mime = if cover.starts_with(&[0x89, b'P', b'N', b'G']) {
            MimeType::Png
        } else {
            MimeType::Jpeg
        };
        let pic = Picture::unchecked(cover.to_vec())
            .mime_type(mime)
            .pic_type(PictureType::CoverFront)
            .build();
        tag.push_picture(pic);
        embedded = true;
        written += 1;
    }

    if written > 0 {
        tagged
            .save_to_path(target, WriteOptions::default())
            .map_err(|e| NcmError::TagWrite(e.to_string()))?;
    }
    Ok((written, embedded))
}
