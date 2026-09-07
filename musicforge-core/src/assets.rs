//! 资产策略（D22）+ provenance（X36）——P6a 资产链路的核心决策层。
//!
//! 设计要点（对齐 ROADMAP D22 与 `docs/p6a-ai-interface.md` §2/§5）：
//!
//! - **Embed-first 来源链**：封面 `内嵌 > 附属图 > 在线 > AI`；
//!   歌词 `内嵌 > .lrc > 在线 > AI`；
//! - **`asset_mode` 三模式，默认 `embedded-only`**——离线默认铁律的资产域体现；
//! - **质量门限 <500px 提示替换**（D22）；
//! - **插件触发 = 来源优先级 + 质量门限双重判定**：已有达标内嵌 → 不发起请求
//!   （P6a 验收断言，[`cover_needs_external`] / [`lyrics_needs_external`]）；
//! - **X36**：provenance 机制从第一天含 `ai` 枚举值
//!   （`embedded/filename/ai/manual/provider`）；
//! - 本模块**零网络**——只做决策与本地存量吸收（`.lrc`/附属图探测），
//!   实际获取由插件进程（独立仓）完成，core 永远持有执行权（X13）。

use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- X36 provenance --

/// 资产来源溯源（X36：枚举从第一天含 `ai`；P6a 资产策略实现）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// 内嵌于音频文件的标签资产
    Embedded,
    /// 由文件名解析得到（无标签兜底）
    Filename,
    /// AI 插件建议（X13：只建议，写入必经用户确认 → Plan → Apply）
    Ai,
    /// 用户手工指定
    Manual,
    /// 非 AI 插件 / 在线提供方（cover-online、lyrics-online 等）
    Provider,
}

impl Provenance {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Embedded => "embedded",
            Self::Filename => "filename",
            Self::Ai => "ai",
            Self::Manual => "manual",
            Self::Provider => "provider",
        }
    }

    /// 反序列化（未知字符串 → None，由调用方显式处理，绝不静默映射）。
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "embedded" => Some(Self::Embedded),
            "filename" => Some(Self::Filename),
            "ai" => Some(Self::Ai),
            "manual" => Some(Self::Manual),
            "provider" => Some(Self::Provider),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------- D22 来源链 --

/// 封面来源链位次（D22：内嵌 > 附属图 > 在线 > AI）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverSource {
    Embedded,
    /// 存量附属图（cover.jpg / folder.jpg / `<stem>.jpg|png`）
    SidecarImage,
    Online,
    Ai,
}

/// 歌词来源链位次（D22：内嵌 > .lrc > 在线 > AI）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsSource {
    Embedded,
    /// 存量 `.lrc`（吸入规则见 [`sidecar_lyrics_path`]）
    SidecarLrc,
    Online,
    Ai,
}

/// D22 封面链（固定位次；索引即优先级，越小越优先）。
pub const COVER_CHAIN: [CoverSource; 4] = [
    CoverSource::Embedded,
    CoverSource::SidecarImage,
    CoverSource::Online,
    CoverSource::Ai,
];

/// D22 歌词链（固定位次）。
pub const LYRICS_CHAIN: [LyricsSource; 4] = [
    LyricsSource::Embedded,
    LyricsSource::SidecarLrc,
    LyricsSource::Online,
    LyricsSource::Ai,
];

// ---------------------------------------------------------------- 三模式 --

/// `asset_mode` 三模式（D22：默认 `embedded-only`；fnOS 兼容默认值 P8 实测定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AssetMode {
    /// 只用内嵌，**永不**发起外部请求（离线默认；P6a 降级完整性的资产域体现）
    #[default]
    EmbeddedOnly,
    /// 内嵌优先：缺失或未达标时按来源链补齐（在线/AI 仍列最末，D22）
    EmbedFirst,
    /// 外部优先（风险最高，须用户显式选择；达标内嵌仍优先于在线——Embed-first 铁律）
    ExternalFirst,
}

impl AssetMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EmbeddedOnly => "embedded-only",
            Self::EmbedFirst => "embed-first",
            Self::ExternalFirst => "external-first",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "embedded-only" => Some(Self::EmbeddedOnly),
            "embed-first" => Some(Self::EmbedFirst),
            "external-first" => Some(Self::ExternalFirst),
            _ => None,
        }
    }

    /// 本模式是否允许（在双重判定的其他条件满足时）发起外部请求。
    pub fn allows_external(&self) -> bool {
        !matches!(self, Self::EmbeddedOnly)
    }
}

// ---------------------------------------------------------------- 策略与双重判定 --

/// 资产策略（D22：模式 + 质量门限）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetPolicy {
    pub mode: AssetMode,
    /// 封面质量门限：短边或任一边低于该值视为「未达标」，可提示替换（D22：<500px）
    pub min_cover_px: u32,
}

impl Default for AssetPolicy {
    fn default() -> Self {
        Self {
            mode: AssetMode::EmbeddedOnly,
            min_cover_px: 500,
        }
    }
}

impl AssetPolicy {
    /// 附件图位次的封面是否「达标」（宽高均 ≥ 门限）。
    fn cover_meets_threshold(min_cover_px: u32, px: Option<(u32, u32)>) -> bool {
        match px {
            None => false, // 不可得 = 未达标（不猜，绝不静默当达标）
            Some((w, h)) => w >= min_cover_px && h >= min_cover_px,
        }
    }
}

/// **双重判定（P6a 验收断言核心）**：封面是否应发起外部请求。
///
/// 条件 = 模式允许外部 **且** 无达标内嵌（无内嵌，或内嵌 < 质量门限）。
/// 已有达标内嵌 → 一律 `false`（D22：绝不浪费请求，Embed-first）。
pub fn cover_needs_external(policy: &AssetPolicy, embedded_px: Option<(u32, u32)>) -> bool {
    policy.mode.allows_external()
        && !AssetPolicy::cover_meets_threshold(policy.min_cover_px, embedded_px)
}

/// 双重判定（歌词链）：是否应发起外部请求。
///
/// 歌词无像素门限——有内嵌（或存量 `.lrc` 已被吸入为上位来源）即不请求。
pub fn lyrics_needs_external(policy: &AssetPolicy, has_embedded: bool) -> bool {
    policy.mode.allows_external() && !has_embedded
}

// ---------------------------------------------------------------- 存量吸收规则 --

/// 存量 `.lrc` 吸入：同目录 `<stem>.lrc` 存在 → 歌词链第二位（内嵌之后）。
///
/// 只探测不读取内容——内容校验/解析在 tagger 层；不存在 → `None`。
pub fn sidecar_lyrics_path(audio: &Path) -> Option<PathBuf> {
    let stem = audio.file_stem()?;
    let cand = audio.with_file_name(format!("{}.lrc", stem.to_string_lossy()));
    cand.is_file().then_some(cand)
}

/// 存量附属图吸入：同目录 `cover.{jpg,png}` / `folder.{jpg,png}` / `<stem>.{jpg,png}`
/// 存在 → 封面链第二位（内嵌之后）。按优先顺序探测，命中即返回。
pub fn sidecar_cover_path(audio: &Path) -> Option<PathBuf> {
    let stem = audio.file_stem()?.to_string_lossy().into_owned();
    let dir = audio.parent()?;
    let mut candidates: Vec<String> = vec![
        "cover.jpg".into(),
        "cover.png".into(),
        "folder.jpg".into(),
        "folder.png".into(),
    ];
    candidates.push(format!("{stem}.jpg"));
    candidates.push(format!("{stem}.png"));
    candidates
        .into_iter()
        .map(|name| dir.join(name))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- X36 provenance ----

    #[test]
    fn provenance_roundtrip_includes_ai_from_day_one() {
        for (v, s) in [
            (Provenance::Embedded, "embedded"),
            (Provenance::Filename, "filename"),
            (Provenance::Ai, "ai"),
            (Provenance::Manual, "manual"),
            (Provenance::Provider, "provider"),
        ] {
            assert_eq!(v.as_str(), s);
            assert_eq!(Provenance::parse(s), Some(v));
        }
        assert_eq!(
            Provenance::parse("unknown"),
            None,
            "未知值显式 None，绝不静默映射"
        );
    }

    // ---- D22 三模式 + 双重判定（P6a 验收断言）----

    #[test]
    fn default_policy_is_embedded_only_and_500px() {
        let p = AssetPolicy::default();
        assert_eq!(p.mode, AssetMode::EmbeddedOnly);
        assert_eq!(p.min_cover_px, 500);
        // 铁律断言：embedded-only 无论内嵌什么都绝不发起请求
        assert!(!cover_needs_external(&p, None));
        assert!(!cover_needs_external(&p, Some((100, 100))));
        assert!(!lyrics_needs_external(&p, false));
    }

    #[test]
    fn qualified_embedded_cover_never_triggers_request() {
        // P6a 硬验收：已有达标内嵌 → 不发起请求（三种模式一律如此）
        for mode in [
            AssetMode::EmbeddedOnly,
            AssetMode::EmbedFirst,
            AssetMode::ExternalFirst,
        ] {
            let p = AssetPolicy {
                mode,
                ..Default::default()
            };
            assert!(
                !cover_needs_external(&p, Some((500, 500))),
                "{mode:?}: 达标内嵌（=500px）不得触发请求"
            );
            assert!(
                !cover_needs_external(&p, Some((1000, 1000))),
                "{mode:?}: 达标内嵌不得触发请求"
            );
        }
    }

    #[test]
    fn below_threshold_embedded_triggers_in_non_default_modes() {
        let p = AssetPolicy {
            mode: AssetMode::EmbedFirst,
            ..Default::default()
        };
        assert!(
            cover_needs_external(&p, Some((499, 800))),
            "499px < 500px 门限 → 可请求"
        );
        assert!(cover_needs_external(&p, None), "无内嵌 → 可请求");
        // 不可得（解析失败）= 未达标，不猜
        assert!(cover_needs_external(&p, None));
        // 但 embedded-only 仍然不请求
        let d = AssetPolicy::default();
        assert!(!cover_needs_external(&d, None));
    }

    #[test]
    fn lyrics_double_gate() {
        let p = AssetPolicy {
            mode: AssetMode::EmbedFirst,
            ..Default::default()
        };
        assert!(!lyrics_needs_external(&p, true), "有内嵌歌词 → 不请求");
        assert!(lyrics_needs_external(&p, false), "无内嵌 → 按链补齐");
        let d = AssetPolicy::default();
        assert!(!lyrics_needs_external(&d, false), "embedded-only 零请求");
    }

    #[test]
    fn mode_parse_roundtrip() {
        for s in ["embedded-only", "embed-first", "external-first"] {
            let m = AssetMode::parse(s).unwrap();
            assert_eq!(m.as_str(), s);
        }
        assert_eq!(AssetMode::parse("online"), None);
    }

    // ---- 存量吸收规则 ----

    #[test]
    fn sidecar_lyrics_ingested_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("song.flac");
        std::fs::write(&audio, b"x").unwrap();
        assert_eq!(sidecar_lyrics_path(&audio), None, "无 .lrc → None");

        let lrc = dir.path().join("song.lrc");
        std::fs::write(&lrc, "[00:00.00] test").unwrap();
        assert_eq!(sidecar_lyrics_path(&audio).as_deref(), Some(lrc.as_path()));
    }

    #[test]
    fn sidecar_cover_priority_order() {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("song.flac");
        std::fs::write(&audio, b"x").unwrap();
        assert_eq!(sidecar_cover_path(&audio), None, "无附属图 → None");

        // 只有 <stem>.jpg 时吸入它
        let stem_img = dir.path().join("song.jpg");
        std::fs::write(&stem_img, b"x").unwrap();
        assert_eq!(
            sidecar_cover_path(&audio).as_deref(),
            Some(stem_img.as_path())
        );

        // 出现 cover.jpg 后优先后者（D22 链序探测顺序）
        let cover = dir.path().join("cover.jpg");
        std::fs::write(&cover, b"x").unwrap();
        assert_eq!(sidecar_cover_path(&audio).as_deref(), Some(cover.as_path()));
    }
}
