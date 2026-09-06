//! 播放清单（P4.3）：M3U/M3U8 按分类导出 + 导入失效路径匹配修复。
//!
//! 设计要点（对齐 v2.4 方案 §P4 能力 #12「按分类导出；失效路径匹配修复」）：
//!
//! - **导出**：扫描曲库 → 按分类键（artist/album/none）分组 → 每组一个
//!   `.m3u8`（UTF-8 + `#EXTM3U` + `#EXTINF`）；分类键作为文件名，复用
//!   [`crate::template::sanitize`]（写盘命名场景必须走同一套清洗规则）；
//!   条目路径优先相对播放清单位置（M3U 惯例），不在其下则绝对路径；
//! - **导入修复**：失效条目按「同名文件（大小写不敏感）→ 时长 ±1s 消歧」
//!   在搜索根内定位新位置；修复**显式记录**（from→to），修不好的条目以
//!   注释行保留在输出里（审计不丢行），绝不静默丢弃；
//! - **不改动任何音乐文件**：export/import 只写清单文件本身。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::NcmError;
use crate::scan::{scan_library, Category, ScanOptions};

/// 分类键（v2.4 §P4「按分类导出」）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GroupBy {
    /// 按艺术家分组（默认；无标签 → 「未知艺术家」）
    #[default]
    Artist,
    /// 按专辑分组（无标签 → 「未知专辑」）
    Album,
    /// 不分组，全部进一个清单
    None,
}

impl GroupBy {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "artist" => Some(Self::Artist),
            "album" => Some(Self::Album),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Artist => "artist",
            Self::Album => "album",
            Self::None => "none",
        }
    }
}

/// 清单条目（导出侧：来自扫描；导入侧：来自解析/修复）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistEntry {
    /// 音频文件路径（导出=实际路径；导入=解析后的路径，修复=新位置）
    pub path: PathBuf,
    /// 标题（无标签 → 文件名）
    pub title: String,
    /// 时长秒（未知 = -1，M3U 惯例）
    pub duration_secs: i64,
}

/// 导出报告。
#[derive(Debug, Default)]
pub struct ExportReport {
    /// (清单路径, 条目数)
    pub playlists: Vec<(PathBuf, usize)>,
    pub files_seen: usize,
}

/// 按分类导出曲库为 M3U8 集合（只写清单文件，不动音乐文件）。
///
/// 输出文件名 = `<sanitize(分类键)>.m3u8`；目标已存在则覆盖（清单是
/// 可再生产物，与 db 同级定位；音乐文件永远不受影响）。
pub fn export_playlists(
    root: &Path,
    out_dir: &Path,
    group_by: GroupBy,
) -> Result<ExportReport, NcmError> {
    if !root.is_dir() {
        return Err(NcmError::Db(format!(
            "playlist: 曲库目录不存在或不可读: {}",
            root.display()
        )));
    }
    std::fs::create_dir_all(out_dir)?;
    let scan = scan_library(root, &ScanOptions::default())?;

    // 收集条目 + 分类键
    let mut groups: BTreeMap<String, Vec<PlaylistEntry>> = BTreeMap::new();
    let mut files_seen = 0usize;
    for item in scan.items.iter().filter(|i| i.category == Category::Audio) {
        files_seen += 1;
        let (title, duration) = audio_title_duration(&item.path);
        let key = match group_by {
            GroupBy::None => "library".to_string(),
            GroupBy::Artist => {
                let a = audio_tag_str(&item.path, |t| {
                    t.get_string(lofty::tag::ItemKey::TrackArtist)
                });
                a.unwrap_or_else(|| "未知艺术家".to_string())
            }
            GroupBy::Album => {
                let a = audio_tag_str(&item.path, |t| {
                    t.get_string(lofty::tag::ItemKey::AlbumTitle)
                });
                a.unwrap_or_else(|| "未知专辑".to_string())
            }
        };
        groups.entry(key).or_default().push(PlaylistEntry {
            path: item.path.clone(),
            title,
            duration_secs: duration,
        });
    }

    let mut report = ExportReport {
        files_seen,
        ..Default::default()
    };
    for (key, mut entries) in groups {
        // 同组内按路径排序，输出确定（两次导出逐字节一致）
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let file_name = format!("{}.m3u8", crate::template::sanitize(&key));
        let playlist_path = out_dir.join(file_name);
        let body = render_m3u8(&playlist_path, &entries)?;
        std::fs::write(&playlist_path, body)?;
        report.playlists.push((playlist_path, entries.len()));
    }
    report.playlists.sort();
    Ok(report)
}

/// 渲染 M3U8 文本（`#EXTM3U` + `#EXTINF`；条目路径优先相对清单位置）。
fn render_m3u8(playlist_path: &Path, entries: &[PlaylistEntry]) -> Result<String, NcmError> {
    let base = playlist_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let mut s = String::from("#EXTM3U\n");
    for e in entries {
        let dur = if e.duration_secs >= 0 {
            e.duration_secs
        } else {
            -1
        };
        s.push_str(&format!("#EXTINF:{dur},{}\n", e.title));
        s.push_str(&entry_line(&base, &e.path));
        s.push('\n');
    }
    Ok(s)
}

/// 条目路径：在清单目录下 → 相对路径（正斜杠），否则绝对路径。
fn entry_line(base: &Path, p: &Path) -> String {
    match p.strip_prefix(base) {
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => p.to_string_lossy().replace('\\', "/"),
    }
}

fn audio_tag_str(path: &Path, get: impl Fn(&lofty::tag::Tag) -> Option<&str>) -> Option<String> {
    use lofty::prelude::*;
    let tagged = lofty::read_from_path(path).ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    get(tag)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// (标题, 时长秒)；无标签标题 → 文件名；时长未知 → -1。
fn audio_title_duration(path: &Path) -> (String, i64) {
    use lofty::prelude::*;
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    match lofty::read_from_path(path) {
        Ok(tagged) => {
            let title = tagged
                .primary_tag()
                .or_else(|| tagged.first_tag())
                .and_then(|t| t.get_string(lofty::tag::ItemKey::TrackTitle))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or(name);
            let secs = tagged.properties().duration().as_secs() as i64;
            (title, secs)
        }
        Err(_) => (name, -1),
    }
}

// ---------------------------------------------------------------- 导入 --

/// 导入报告。
#[derive(Debug, Default)]
pub struct ImportReport {
    pub total_entries: usize,
    /// 直接命中（路径有效）
    pub ok: usize,
    /// 修复成功（失效 → 同名+时长匹配到新位置）
    pub repaired: Vec<(PathBuf, PathBuf)>,
    /// 无法修复（原行 + 原因）
    pub unresolved: Vec<(String, String)>,
    /// 修复后清单写出路径（调用方传了 out 才有）
    pub written: Option<PathBuf>,
}

/// 解析并修复播放清单（不改音乐文件；只产出修复后的清单文件）。
///
/// - 相对条目按**清单文件所在目录**解析（M3U 惯例）；
/// - 失效条目：在 `search_root` 递归找**同名文件**（大小写不敏感）；
///   多个候选时用 `#EXTINF` 时长（±1s）消歧；仍无法唯一定位 → unresolved；
/// - 输出（`out`，缺省 `<原名>.fixed.m3u8`）：UTF-8 `#EXTM3U`，已修复条目
///   用新位置，unresolved 条目以 `# FAIL:` 注释保留（审计不丢行）。
pub fn import_and_repair(
    playlist: &Path,
    search_root: &Path,
    out: Option<&Path>,
) -> Result<ImportReport, NcmError> {
    let text = std::fs::read_to_string(playlist)?;
    let base = playlist
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();

    // 候选索引：搜索根内 文件名(小写) -> [路径]
    let mut by_name: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    collect_names(search_root, 0, &mut by_name)?;

    let mut rep = ImportReport::default();
    let mut out_entries: Vec<PlaylistEntry> = Vec::new();
    let mut pending: Option<(i64, String)> = None;

    for line in text.lines() {
        let line = line.trim_end_matches(['\r']);
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(rest) = t.strip_prefix("#EXTINF:") {
            // #EXTINF:<dur>,<title>
            let (dur, title) = match rest.split_once(',') {
                Some((d, ti)) => (d.trim().parse::<i64>().unwrap_or(-1), ti.trim().to_string()),
                None => (-1, rest.trim().to_string()),
            };
            pending = Some((dur, title));
            continue;
        }
        if t.starts_with('#') {
            continue; // #EXTM3U 与其他注释
        }
        // 路径行
        rep.total_entries += 1;
        let raw = PathBuf::from(t);
        let resolved = if raw.is_absolute() {
            raw.clone()
        } else {
            base.join(&raw)
        };
        let (title, dur) = pending
            .take()
            .map(|(d, ti)| (Some(ti), Some(d)))
            .unwrap_or((None, None));

        if resolved.exists() {
            rep.ok += 1;
            let (t2, d2) = audio_title_duration(&resolved);
            out_entries.push(PlaylistEntry {
                path: resolved,
                title: title.unwrap_or(t2),
                duration_secs: dur.unwrap_or(d2),
            });
            continue;
        }

        // 失效 → 同名匹配
        let file_name = raw
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .unwrap_or_default();
        let candidates = by_name.get(&file_name).cloned().unwrap_or_default();
        match pick_candidate(&candidates, dur) {
            Some(fixed) => {
                rep.repaired.push((resolved.clone(), fixed.clone()));
                let (t2, d2) = audio_title_duration(&fixed);
                out_entries.push(PlaylistEntry {
                    path: fixed,
                    title: title.unwrap_or(t2),
                    duration_secs: dur.unwrap_or(d2),
                });
            }
            None => {
                let reason = if candidates.is_empty() {
                    "搜索根内无同名文件".to_string()
                } else {
                    format!("{} 个同名候选无法唯一定位", candidates.len())
                };
                rep.unresolved.push((t.to_string(), reason));
                // 审计不丢行：以注释保留在输出
                out_entries.push(PlaylistEntry {
                    path: PathBuf::from("# FAIL"),
                    title: t.to_string(),
                    duration_secs: -1,
                });
            }
        }
    }

    // 写修复后清单
    let out_path = out.map(|p| p.to_path_buf()).unwrap_or_else(|| {
        let mut s = playlist.to_string_lossy().into_owned();
        if !s.to_lowercase().ends_with(".m3u8") {
            s.push_str(".fixed.m3u8");
        } else {
            s.push_str(".fixed");
        }
        PathBuf::from(s)
    });
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut body = String::from("#EXTM3U\n");
    for e in &out_entries {
        if e.path.to_string_lossy() == "# FAIL" {
            body.push_str(&format!("# FAIL {}\n", e.title));
        } else {
            let dur = if e.duration_secs >= 0 {
                e.duration_secs
            } else {
                -1
            };
            body.push_str(&format!("#EXTINF:{dur},{}\n", e.title));
            body.push_str(&entry_line(
                out_path.parent().unwrap_or(Path::new("")),
                &e.path,
            ));
            body.push('\n');
        }
    }
    std::fs::write(&out_path, body)?;
    rep.written = Some(out_path);
    Ok(rep)
}

/// 候选消歧：唯一候选直接用；多候选时用时长 ±1s 过滤；命中唯一才修复。
fn pick_candidate(candidates: &[PathBuf], dur: Option<i64>) -> Option<PathBuf> {
    if candidates.is_empty() {
        return None;
    }
    if candidates.len() == 1 {
        return Some(candidates[0].clone());
    }
    let dur = dur?;
    if dur < 0 {
        return None;
    }
    let matched: Vec<&PathBuf> = candidates
        .iter()
        .filter(|c| {
            let (_, d) = audio_title_duration(c);
            d >= 0 && (d - dur).abs() <= 1
        })
        .collect();
    if matched.len() == 1 {
        Some(matched[0].clone())
    } else {
        None
    }
}

/// 递归收集文件名索引（大小写不敏感键；不跟随符号链接——复用扫描器语义）。
fn collect_names(
    dir: &Path,
    depth: usize,
    map: &mut BTreeMap<String, Vec<PathBuf>>,
) -> Result<(), NcmError> {
    const MAX_DEPTH: usize = 64;
    if depth > MAX_DEPTH {
        return Ok(());
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in rd.flatten() {
        let Ok(md) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if md.is_dir() {
            collect_names(&entry.path(), depth + 1, map)?;
        } else if md.is_file() {
            if let Some(name) = entry.file_name().to_str() {
                map.entry(name.to_lowercase())
                    .or_default()
                    .push(entry.path());
            }
        }
    }
    Ok(())
}
