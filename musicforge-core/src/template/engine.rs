//! 命名模板引擎（方案书 §8 机会点 4：**全生态 17 个项目无人提供终端用户可配置的命名模板**）
//!
//! 语义：
//! - 模板按 `/` 拆分为路径段，逐段渲染后**逐段清洗**（值中的 `/` 变 `_`，不会伪造目录）
//! - 占位符：`{title}` `{artist}` `{album}` `{track}` `{track:0Nd}`（零填充宽度 N）`{format}`
//! - 缺失字段回退：title → 源文件名；artist/album → 「未知…」；track → `00`
//! - 清洗（Windows 安全）：`<>:"/\|?*` 与控制字符 → `_`；尾部 `. ` 去除；保留设备名加前缀；段长截断
//!
//! 实证依据：上游 Java 版硬编码 `艺术家 - 曲名` 且**不清洗**非法字符（`NcmDump.java:215`，
//! 曲名含 `?*:<>|` 时 Windows 写盘直接失败）。

use crate::metadata::Metadata;

const ILLEGAL: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];
/// 单路径段字符上限（UTF-8 安全按字符截断；防 MAX_PATH 爆炸）
const MAX_SEGMENT_CHARS: usize = 100;
/// Linux/macOS 文件名组件硬上限 255 **字节**（Windows 是 255 UTF-16 单元）。
/// 按 100 字符截断对多字节字符（CJK 3B、emoji 4B）仍会超限（100×4=400B），
/// 因此增加字节预算：≤200 字节，为扩展名（如 ".flac"）与多段路径留余量。
const MAX_SEGMENT_BYTES: usize = 200;

/// 缺失字段的回退文案
pub struct Fallbacks {
    pub artist: String,
    pub album: String,
}

impl Default for Fallbacks {
    fn default() -> Self {
        Self {
            artist: "未知艺术家".to_string(),
            album: "未知专辑".to_string(),
        }
    }
}

/// 渲染模板为**相对路径**（可能含子目录）。
///
/// `meta` 为 `None`（metadataLen=0 场景）时全部走回退；
/// `fallback_stem` 是源文件名（无扩展名），title 缺失时的最终兜底。
/// 返回值保证非空、逐段清洗、不含非法字符。
pub fn render_filename(template: &str, meta: Option<&Metadata>, fallback_stem: &str) -> String {
    let fb = Fallbacks::default();

    let rendered: Vec<String> = template
        .split('/')
        .filter(|seg| !seg.is_empty())
        .map(|seg| {
            let s = sanitize(&substitute(seg, meta, &fb, fallback_stem));
            truncate_chars(&s, MAX_SEGMENT_CHARS)
        })
        .filter(|s| !s.is_empty() && s != "_")
        .collect();

    // 全部段渲染为空 → 回退到源文件名。
    // QA 第二轮：回退值**同样**要过截断上界。此前这里只 `sanitize()` 就返回，
    // 绕过了段长 100 字符的钳制——超长源文件名（Windows MAX_PATH 只有 260）
    // 会在这条唯一分支上炸掉写盘，且与 clamp 的存在理由自相矛盾。
    if rendered.is_empty() {
        return truncate_chars(&sanitize(fallback_stem), MAX_SEGMENT_CHARS);
    }
    rendered.join("/")
}

/// 占位符替换（未知占位符原样保留，便于用户发现拼写错误）
fn substitute(seg: &str, meta: Option<&Metadata>, fb: &Fallbacks, fallback_stem: &str) -> String {
    let mut out = String::new();
    let mut rest = seg;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let Some(end_rel) = rest[start..].find('}') else {
            out.push_str(rest);
            return out;
        };
        let end = start + end_rel;
        let ph = &rest[start + 1..end];
        out.push_str(&value_for(ph, meta, fb, fallback_stem));
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

fn value_for(ph: &str, meta: Option<&Metadata>, fb: &Fallbacks, fallback_stem: &str) -> String {
    let (name, spec) = match ph.split_once(':') {
        Some((n, s)) => (n, Some(s)),
        None => (ph, None),
    };
    match name {
        "title" => meta
            .and_then(|m| m.name.clone())
            .unwrap_or_else(|| fallback_stem.to_string()),
        "artist" => meta
            .and_then(|m| m.artist.clone())
            .unwrap_or_else(|| fb.artist.clone()),
        "album" => meta
            .and_then(|m| m.album.clone())
            .unwrap_or_else(|| fb.album.clone()),
        "format" => meta.and_then(|m| m.format.clone()).unwrap_or_default(),
        "track" => track_value(meta, spec),
        other => format!("{{{other}}}"),
    }
}

fn track_value(meta: Option<&Metadata>, spec: Option<&str>) -> String {
    let t = meta.and_then(|m| m.track);
    match (t, spec) {
        (Some(t), Some(s)) if s.ends_with('d') => {
            // QA-B2：宽度规格必须有上界——std format! 对宽度 > u16::MAX 直接 panic
            //（"Formatting argument out of range"），且巨量宽度造成内存放大。
            // CLI --template 用户可直接触达，违反硬约束 1（零 panic）。钳制到合理宽度。
            let w: usize = s[..s.len() - 1].parse().unwrap_or(2);
            let w = w.clamp(1, 16);
            format!("{:0w$}", t, w = w)
        }
        (Some(t), _) => t.to_string(),
        (None, _) => "00".to_string(),
    }
}

/// 段清洗：非法字符/控制字符 → `_`；尾部 `. ` 去除；Windows 保留设备名加 `_` 前缀；空段 → `_`。
///
/// P4 起公开（playlist 按分类导出的文件名复用同一套清洗规则——
/// 凡是写盘的「用户数据变成文件名」场景都必须走这里，不得各写各的）。
pub fn sanitize(seg: &str) -> String {
    let mapped: String = seg
        .chars()
        .map(|c| {
            if ILLEGAL.contains(&c) || (c as u32) < 0x20 || c as u32 == 0x7f {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = mapped.trim_end_matches(['.', ' ']);
    let s = if trimmed.is_empty() {
        "_".to_string()
    } else {
        trimmed.to_string()
    };
    let first = s.split('.').next().unwrap_or("");
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(first)) {
        format!("_{s}")
    } else {
        s
    }
}

/// 按字符数截断（UTF-8 安全）
fn truncate_chars(s: &str, max: usize) -> String {
    // 先按字符数截断（可读性上限）
    let s: String = s.chars().take(max).collect();
    // 再按字节数截断（文件系统组件上限；绝不切开 UTF-8 字符）
    if s.len() <= MAX_SEGMENT_BYTES {
        return s;
    }
    let mut end = MAX_SEGMENT_BYTES;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(name: &str, artist: &str, album: &str, track: Option<u64>) -> Metadata {
        Metadata {
            name: Some(name.to_string()),
            artist: Some(artist.to_string()),
            album: Some(album.to_string()),
            format: Some("flac".to_string()),
            bitrate: None,
            duration: None,
            track,
            album_pic_url: None,
        }
    }

    #[test]
    fn full_template_with_dirs() {
        let m = meta("贝贝", "李荣浩", "耳朵", Some(1));
        assert_eq!(
            render_filename("{artist}/{album}/{track:02d} {title}", Some(&m), "src"),
            "李荣浩/耳朵/01 贝贝"
        );
    }

    #[test]
    fn missing_track_pads_to_00() {
        let m = meta("贝贝", "李荣浩", "耳朵", None);
        assert!(render_filename("{track:02d} {title}", Some(&m), "s").starts_with("00 "));
    }

    #[test]
    fn track_width_spec() {
        let m = meta("t", "a", "al", Some(7));
        assert_eq!(render_filename("{track:03d}", Some(&m), "s"), "007");
        assert_eq!(render_filename("{track}", Some(&m), "s"), "7");
    }

    #[test]
    fn missing_metadata_falls_back() {
        assert_eq!(
            render_filename("{artist} - {title}", None, "orig"),
            "未知艺术家 - orig"
        );
    }

    #[test]
    fn sanitizes_illegal_chars() {
        let m = meta(
            "AC/DC: Back? *In* \"Black\" <Vol|1>",
            "AC\\DC",
            "耳朵",
            None,
        );
        let out = render_filename("{title}", Some(&m), "s");
        assert!(
            !out.contains('/')
                && !out.contains(':')
                && !out.contains('?')
                && !out.contains('*')
                && !out.contains('"')
                && !out.contains('<')
                && !out.contains('>')
                && !out.contains('|'),
            "{out}"
        );
        assert_eq!(out, "AC_DC_ Back_ _In_ _Black_ _Vol_1_");
    }

    #[test]
    fn sanitizes_control_chars_and_trailing_dots() {
        // 控制字符 → '_'；尾部 '.' 与空格去除（注意：'_' 不在尾部修剪集，故结果为 name._）
        let m = meta("name.\x07 ", "a", "al", None);
        assert_eq!(render_filename("{title}", Some(&m), "s"), "name._");
        let m2 = meta("name. ", "a", "al", None);
        assert_eq!(render_filename("{title}", Some(&m2), "s"), "name");
    }

    #[test]
    fn reserved_device_names_prefixed() {
        let m = meta("CON", "a", "al", None);
        assert_eq!(render_filename("{title}", Some(&m), "s"), "_CON");
        let m2 = meta("com1.txt", "a", "al", None);
        assert_eq!(render_filename("{title}", Some(&m2), "s"), "_com1.txt");
    }

    #[test]
    fn value_slash_does_not_create_dirs() {
        // 多艺术家以 "/" 连接 —— 值中的分隔符必须被清洗，不得伪造目录
        let m = meta("t", "A/B", "al", None);
        let out = render_filename("{artist}/{title}", Some(&m), "s");
        assert_eq!(out.matches('/').count(), 1, "{out}");
        assert_eq!(out, "A_B/t");
    }

    #[test]
    fn long_name_truncated_by_bytes() {
        // 段长上界 = min(100 字符, 200 字节)。"曲" 为 3 字节/字符：
        // 300 字符 → 100 字符(300B) → 字节截断至 66 字符(198B ≤ 200B)。
        let long = "曲".repeat(300);
        let m = meta(&long, "a", "al", None);
        let out = render_filename("{title}", Some(&m), "s");
        assert_eq!(out.chars().count(), 66);
        assert!(
            out.len() <= MAX_SEGMENT_BYTES,
            "组件字节必须 ≤ {MAX_SEGMENT_BYTES}，实际 {}",
            out.len()
        );
    }

    #[test]
    fn unknown_placeholder_kept_literal() {
        let m = meta("t", "a", "al", None);
        assert_eq!(render_filename("{bogus}", Some(&m), "s"), "{bogus}");
    }

    #[test]
    fn all_empty_segments_fall_back_to_stem() {
        // format 字段缺失 → 段渲染为空 → 整体回退到源文件名
        let mut m = meta("t", "a", "al", None);
        m.format = None;
        assert_eq!(render_filename("{format}", Some(&m), "src"), "src");
    }

    /// QA 第二轮：回退分支此前只 `sanitize()` 就返回，绕过段长上界。
    /// 触发条件很窄（模板所有段都渲染为空），但一旦命中就是 MAX_PATH 级别的写盘失败。
    #[test]
    fn fallback_stem_is_truncated_too() {
        let mut m = meta("t", "a", "al", None);
        m.format = None; // 让 {format} 渲染为空 → 走回退分支
        let long_stem = "曲".repeat(300);
        let out = render_filename("{format}", Some(&m), &long_stem);
        assert!(
            out.len() <= MAX_SEGMENT_BYTES && out.chars().count() <= MAX_SEGMENT_CHARS,
            "回退值也必须受段长上界约束（字符+字节双上限），实际 {} chars / {} bytes",
            out.chars().count(),
            out.len()
        );
    }

    /// P1a 保护网：中文快照。中文标题/艺人/专辑 + track 零填充 + 目录段
    /// 组合渲染必须保持稳定 —— 这是绞杀者重构时最容易被打断的用户可见契约。
    ///
    /// 注意：`render_filename` 返回**相对路径且不含扩展名**——扩展名由 CLI
    /// `plan_one` 按判定格式追加（见 `batch_template_dirs_and_padding` 的
    /// `测试曲目.flac` 断言）。故期望值末尾没有 `.flac`。
    #[test]
    fn chinese_template_snapshot() {
        let m = meta("晴天", "周杰伦", "叶惠美", Some(1));
        assert_eq!(
            render_filename("{artist}/{album}/{track:02d} {title}", Some(&m), "flac"),
            "周杰伦/叶惠美/01 晴天",
            "中文模板快照变化：渲染输出不得改变（扩展名由 CLI 层追加，不归本函数管）"
        );
    }
}
