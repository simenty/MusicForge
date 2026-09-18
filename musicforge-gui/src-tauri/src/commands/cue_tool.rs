//! CUE 分轨工具（P4 工具箱）：选择 → 检视 → 整轨无损切分。
//!
//! 能力全部来自 `musicforge_core::cue`（解析 / 采样边界 / 切分 / 标签）。
//! 与 CLI `musicforge split` 的**行为契约**：
//! - 命名规则同为 `{n:02} {title}`（同名清洗由 core 统一执行）；
//! - APE/WV/TAK 源需要 ffmpeg（自动搜索 PATH，与 CLI 的 `Ffmpeg::find` 同一路径）；
//! - JSON 报告同形（cue/source/album/tracks/failed），GUI 只追加 `outDir`。
//!
//! 两条入口的产物因此可互换（同一批输出文件）。
//!
//! 本模块只负责：原生对话框挑选 + 报告 JSON 化 + 在阻塞池上执行。

use std::path::{Path, PathBuf};

use tauri_plugin_dialog::DialogExt;

/// 读 CUE 所指音频的扩展名（小写）。解析失败或无 FILE 指令 → None。
fn audio_ext_of(cue: &Path) -> Option<String> {
    let sheet = musicforge_core::cue::parse_cue_file(cue).ok()?;
    let dir = cue.parent().unwrap_or(Path::new("."));
    let ext = dir.join(sheet.file?).extension()?.to_str()?.to_ascii_lowercase();
    Some(ext)
}

/// 源是否需要 ffmpeg（与 CLI `run_split_sub` 同一判定：ape/wv/tak）。
fn cue_needs_ffmpeg(cue: &Path) -> bool {
    matches!(audio_ext_of(cue).as_deref(), Some("ape" | "wv" | "tak"))
}

/// 原生选择 .cue 文件。用户取消 → null。
#[tauri::command]
pub async fn cue_pick(app: tauri::AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .add_filter("CUE 分轨表", &["cue"])
        .set_title("选择 .cue 文件")
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
}

/// 检视：解析 CUE（**不触碰音频**）——曲目清单 + FILE 指向是否存在 + 是否需要 ffmpeg。
#[tauri::command]
pub async fn cue_inspect(path: String) -> Result<serde_json::Value, String> {
    inspect_inner(&path)
}

/// `cue_inspect` 的同步实现（命令只做 async 包装——便于命令层测试不经 runtime）。
fn inspect_inner(path: &str) -> Result<serde_json::Value, String> {
    let cue = PathBuf::from(path);
    let sheet = musicforge_core::cue::parse_cue_file(&cue).map_err(|e| e.to_string())?;
    let dir = cue.parent().unwrap_or(Path::new("."));
    let audio_exists = sheet
        .file
        .as_deref()
        .map(|f| dir.join(f).exists())
        .unwrap_or(false);
    Ok(serde_json::json!({
        "cue": path,
        "album": sheet.title,
        "performer": sheet.performer,
        "date": sheet.rem_date,
        "genre": sheet.rem_genre,
        "audioFile": sheet.file,
        "audioExists": audio_exists,
        "needsFfmpeg": cue_needs_ffmpeg(&cue),
        "tracks": sheet
            .tracks
            .iter()
            .map(|t| serde_json::json!({
                "number": t.number,
                "title": t.title,
                "performer": t.performer,
            }))
            .collect::<Vec<_>>(),
    }))
}

/// 整轨切分（长任务 → 阻塞池；校验在写盘前完成，失败轨不落盘）。
/// 返回与 CLI `split --json` 同形的报告（追加 `outDir`）。
#[tauri::command]
pub async fn cue_split(cue_path: String, out_dir: String) -> Result<serde_json::Value, String> {
    let cue = PathBuf::from(cue_path);
    let out = PathBuf::from(&out_dir);
    let report = tauri::async_runtime::spawn_blocking(move || {
        // ffmpeg 搜索放进闭包（不在 async 上下文做磁盘探测）
        let ff = if cue_needs_ffmpeg(&cue) {
            musicforge_core::ffmpeg::Ffmpeg::find(None).ok()
        } else {
            None
        };
        musicforge_core::cue::split_cue_ex(&cue, &out, None, ff.as_ref(), |n, t| {
            let title = t.title.as_deref().unwrap_or("Unknown Track");
            format!("{n:02} {title}")
        })
    })
    .await
    .map_err(|e| format!("任务执行失败：{e}"))?
    .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "outDir": out_dir,
        "source": report.source.display().to_string(),
        "album": report.sheet.title,
        "tracks": report.tracks.iter().map(|t| serde_json::json!({
            "index": t.index,
            "title": t.title,
            "path": t.dst.display().to_string(),
            "durationSecs": format!("{:.2}", t.duration_secs),
        })).collect::<Vec<_>>(),
        "failed": report.failed.iter().map(|(no, r)| serde_json::json!({
            "track": no, "reason": r,
        })).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 命令层回归：真实写入一个最小 CUE → 报告结构 + 音频缺失判定。
    #[test]
    fn inspect_reports_tracks_and_missing_audio() {
        let dir = std::env::temp_dir().join(format!("mf-cue-tool-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cue = dir.join("t.cue");
        std::fs::write(
            &cue,
            "PERFORMER \"Beyond\"\nTITLE \"乐与怒\"\nFILE \"not-there.flac\" WAVE\n  \
             TRACK 01 AUDIO\n    TITLE \"海阔天空\"\n    INDEX 01 00:00:00\n  \
             TRACK 02 AUDIO\n    TITLE \"爸爸妈妈\"\n    INDEX 01 05:00:00\n",
        )
        .unwrap();

        let v = inspect_inner(&cue.to_string_lossy()).unwrap();
        assert_eq!(v["album"], "乐与怒");
        assert_eq!(v["performer"], "Beyond");
        assert_eq!(v["audioFile"], "not-there.flac");
        assert_eq!(v["audioExists"], false, "音频不存在须如实报告");
        assert_eq!(v["needsFfmpeg"], false, "flac 源不需要 ffmpeg");
        assert_eq!(v["tracks"].as_array().unwrap().len(), 2);

        // APE 源 → needsFfmpeg=true（判定与 CLI 一致）
        let cue2 = dir.join("t2.cue");
        std::fs::write(
            &cue2,
            "FILE \"x.ape\" WAVE\n  TRACK 01 AUDIO\n    INDEX 01 00:00:00\n",
        )
        .unwrap();
        let v2 = inspect_inner(&cue2.to_string_lossy()).unwrap();
        assert_eq!(v2["needsFfmpeg"], true);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
