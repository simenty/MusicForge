//! FFmpeg sidecar（P5b.1）：五级探测 + 有损导出预设 + 回读校验。
//!
//! 设计要点（D1 / D7 / ROADMAP P5b）：
//!
//! - **永不默认捆绑**：ffmpeg 由用户自备（ffmpeg.org 或发行版包管理器）；
//!   本模块只做**探测与调用**，探测失败 = `MF-FFMPEG-MISSING` + 安装引导；
//! - **五级探测**：`--ffmpeg-path` 显式 → 可执行文件同目录 → PATH → 常见
//!   安装位置 → 放弃；每个候选以 `ffmpeg -version` 退出码验证（存在但不可
//!   执行的候选自动跳过）；
//! - **有损导出预设**：MP3 320（libmp3lame）/ AAC 256（native aac）/ Opus 160
//!   （libopus）；目标容器由扩展名决定；
//! - **回读校验**：容器魔数 + 时长差 <1s（时长从 `ffmpeg -i` 的 stderr 解析，
//!   不依赖 ffprobe）；校验失败删产物 + 显式报错；
//! - **升级转换拦截**：有损源（MP3 等）→ 无损目标（FLAC/WAV）= 伪升级，
//!   `MF-LOSSY-TO-LOSSLESS` 拦截，`--i-know-lossy-to-lossless` 显式放行。

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::NcmError;

/// 有损导出预设（ROADMAP P5b）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LossyPreset {
    /// MP3 320kbps（libmp3lame）
    Mp3,
    /// AAC 256kbps（native aac，容器 .m4a）
    Aac,
    /// Opus 160kbps（libopus）
    Opus,
}

impl LossyPreset {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "mp3" => Some(Self::Mp3),
            "aac" => Some(Self::Aac),
            "opus" => Some(Self::Opus),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Aac => "m4a",
            Self::Opus => "opus",
        }
    }

    /// 编码参数（ffmpeg CLI）。
    pub fn codec_args(self) -> &'static [&'static str] {
        match self {
            Self::Mp3 => &["-codec:a", "libmp3lame", "-b:a", "320k"],
            Self::Aac => &["-codec:a", "aac", "-b:a", "256k"],
            Self::Opus => &["-codec:a", "libopus", "-b:a", "160k"],
        }
    }
}

/// 有损/无损源分类（升级拦截依据）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceClass {
    Lossless,
    Lossy,
}

const LOSSY_EXTS: &[&str] = &["mp3", "aac", "m4a", "opus", "ogg", "wma"];

/// 按扩展名分类源文件（探测失败/不认识的扩展 → None）。
pub fn classify_source(path: &Path) -> Option<SourceClass> {
    if crate::lossless::probe_lossless_file(path).is_some() {
        return Some(SourceClass::Lossless);
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())?
        .to_ascii_lowercase();
    if LOSSY_EXTS.contains(&ext.as_str()) && path.is_file() {
        return Some(SourceClass::Lossy);
    }
    None
}

// ---------------------------------------------------------------- 五级探测 --

/// 解析出的 ffmpeg 可执行文件。
#[derive(Debug, Clone)]
pub struct Ffmpeg {
    pub path: PathBuf,
}

impl Ffmpeg {
    /// 五级探测：显式路径 → exe 同目录 → PATH → 常见位置 → 失败。
    ///
    /// `explicit` 来自 CLI `--ffmpeg-path`：可以是可执行文件本身，也可以是
    /// 其所在目录。每个候选都用 `ffmpeg -version` 退出码验证。
    pub fn find(explicit: Option<&Path>) -> Result<Self, NcmError> {
        let exe_ext = if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        };
        let mut searched = 0usize;

        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(p) = explicit {
            if p.is_dir() {
                candidates.push(p.join(exe_ext));
            } else {
                candidates.push(p.to_path_buf());
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join(exe_ext));
            }
        }
        // Level 3：PATH（直接尝试裸命令，退出码 0 = 找到）
        if let Ok(out) = Command::new(exe_ext).arg("-version").output() {
            if out.status.success()
                && String::from_utf8_lossy(&out.stdout).contains("ffmpeg version")
            {
                return Ok(Self {
                    path: PathBuf::from(exe_ext),
                });
            }
        }
        searched += 1;
        // Level 4：常见安装位置
        for dir in common_dirs() {
            candidates.push(dir.join(exe_ext));
        }

        for c in &candidates {
            searched += 1;
            if !c.is_file() {
                continue;
            }
            if let Ok(out) = Command::new(c).arg("-version").output() {
                if out.status.success()
                    && String::from_utf8_lossy(&out.stdout).contains("ffmpeg version")
                {
                    return Ok(Self { path: c.clone() });
                }
            }
        }
        Err(NcmError::FfmpegMissing { searched })
    }

    /// 从 `ffmpeg -i <file>` 的 stderr 解析时长（秒）。不依赖 ffprobe。
    pub fn probe_duration(&self, path: &Path) -> Result<f64, NcmError> {
        let out = Command::new(&self.path)
            .args(["-hide_banner", "-i"])
            .arg(path)
            .output()?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        for line in stderr.lines() {
            let Some(rest) = line.trim().strip_prefix("Duration:") else {
                continue;
            };
            // "Duration: 00:01:23.45, start: ..." → 00:01:23.45
            let tok = rest.split(',').next().unwrap_or("").trim();
            let mut it = tok.split(':');
            let h: f64 = it.next().unwrap_or("0").parse().unwrap_or(0.0);
            let m: f64 = it.next().unwrap_or("0").parse().unwrap_or(0.0);
            let s: f64 = it.next().unwrap_or("0").parse().unwrap_or(0.0);
            return Ok(h * 3600.0 + m * 60.0 + s);
        }
        Err(NcmError::Lossless(format!(
            "无法从 ffmpeg 输出解析时长（{path:?}）"
        )))
    }

    /// 有损导出：编码 → 回读校验（魔数 + 时长差 <1s）→ 字节数。
    pub fn export_lossy(
        &self,
        src: &Path,
        dst: &Path,
        preset: LossyPreset,
    ) -> Result<u64, NcmError> {
        // 覆盖守卫（稳定审计 B11）：与 transcode 同语义，绝不覆盖既有文件
        if dst.exists() {
            return Err(NcmError::OutputExists {
                path: dst.display().to_string(),
            });
        }
        let out = Command::new(&self.path)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(src)
            .args(preset.codec_args())
            .arg(dst)
            .output()?;
        if !out.status.success() {
            let _ = std::fs::remove_file(dst);
            return Err(NcmError::Lossless(format!(
                "ffmpeg 编码失败: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        verify_lossy_output(self, src, dst, preset)?;
        file_len(dst)
    }

    /// 自定义参数导出（升级转换放行后的有损→无损，如 `-codec:a flac`）。
    pub fn export_custom(&self, src: &Path, dst: &Path, args: &[&str]) -> Result<u64, NcmError> {
        // 覆盖守卫（同 export_lossy）
        if dst.exists() {
            return Err(NcmError::OutputExists {
                path: dst.display().to_string(),
            });
        }
        let out = Command::new(&self.path)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(src)
            .args(args)
            .arg(dst)
            .output()?;
        if !out.status.success() {
            let _ = std::fs::remove_file(dst);
            return Err(NcmError::Lossless(format!(
                "ffmpeg 编码失败: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        file_len(dst)
    }
}

fn common_dirs() -> Vec<PathBuf> {
    let mut v = vec![
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/opt/homebrew/bin"),
    ];
    if cfg!(windows) {
        if let Ok(pf) = std::env::var("ProgramFiles") {
            v.push(PathBuf::from(pf).join("ffmpeg").join("bin"));
        }
        v.push(PathBuf::from(r"C:\ffmpeg\bin"));
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            v.push(PathBuf::from(local).join("ffmpeg").join("bin"));
        }
    }
    v
}

/// 回读校验：容器魔数匹配 + 时长差 <1s；失败删产物。
fn verify_lossy_output(
    ff: &Ffmpeg,
    src: &Path,
    dst: &Path,
    preset: LossyPreset,
) -> Result<(), NcmError> {
    let data = std::fs::read(dst)?;
    let magic_ok = match preset {
        LossyPreset::Mp3 => {
            data.starts_with(b"ID3")
                || (!data.is_empty() && data[0] == 0xFF && data[1] & 0xE0 == 0xE0)
        }
        LossyPreset::Aac => data.len() > 8 && &data[4..8] == b"ftyp",
        LossyPreset::Opus => data.starts_with(b"OggS"),
    };
    if !magic_ok {
        let _ = std::fs::remove_file(dst);
        return Err(NcmError::Lossless(format!(
            "回读校验失败：{} 的容器魔数与目标格式不符（已删除未验证产物）",
            dst.display()
        )));
    }
    let d_src = ff.probe_duration(src)?;
    let d_dst = ff.probe_duration(dst)?;
    if (d_src - d_dst).abs() >= 1.0 {
        let _ = std::fs::remove_file(dst);
        return Err(NcmError::Lossless(format!(
            "回读校验失败：时长偏差 {:.2}s ≥1s（源 {:.2}s / 产物 {:.2}s，已删除未验证产物）",
            (d_src - d_dst).abs(),
            d_src,
            d_dst
        )));
    }
    Ok(())
}

fn file_len(path: &Path) -> Result<u64, NcmError> {
    Ok(std::fs::metadata(path)?.len())
}
