//! 歌词（P6，桌面层）：LRCLIB 公开 API（<https://lrclib.net>）——社区维护、
//! 无 key、专为同步歌词（LRC）设计，是**合规性最高的选项**；非官方抓取类
//! 方案（各音乐平台私有接口）**刻意不做**。
//!
//! 网络边界（docs/dependency-policy.md §4）：仅当用户打开歌词面板且本地
//! 无缓存时请求一次；结果落盘 `<数据目录>/lyrics/{track_id}.lrc` 后永不再请求。
//! 失败返回 Err 由前端提示——不影响任何离线功能。

use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

use super::library_db::open_db;

/// 可识别的 User-Agent（与封面模块同一惯例）。
const UA: &str = concat!(
    "MusicForge/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/simenty/MusicForge)"
);

/// 歌词缓存目录（与状态库同目录：`<数据目录>/lyrics`）。
fn lyrics_dir() -> PathBuf {
    musicforge_core::db::default_db_path()
        .parent()
        .map(|p| p.join("lyrics"))
        .unwrap_or_else(|| PathBuf::from("lyrics"))
}

#[derive(Deserialize)]
struct LrclibResp {
    #[serde(rename = "syncedLyrics", default)]
    synced_lyrics: Option<String>,
    #[serde(rename = "plainLyrics", default)]
    plain_lyrics: Option<String>,
}

/// 取歌词（LRC 文本）。优先级：本地缓存 → LRCLIB（同步歌词优先，回退纯文本）。
/// 无歌词 → `Ok(None)`；网络失败 → `Err`（前端仅在你点开歌词时调用）。
#[tauri::command]
pub async fn lyrics_fetch(track_id: i64) -> Result<Option<String>, String> {
    // ① 本地缓存（命中即返回——缓存过就不再请求）
    let cache = lyrics_dir().join(format!("{track_id}.lrc"));
    if let Ok(s) = std::fs::read_to_string(&cache) {
        if !s.trim().is_empty() {
            return Ok(Some(s));
        }
    }

    // ② 曲目信息（标题/艺术家必填；专辑与时长可选，提高命中率）
    let track = {
        let db = open_db()?;
        db.get_track(track_id).map_err(|e| e.to_string())?
    };
    let Some(track) = track else {
        return Ok(None);
    };
    let (Some(title), Some(artist)) = (track.title.clone(), track.artist.clone()) else {
        return Ok(None);
    };
    let mut q: Vec<(&str, String)> = vec![("track_name", title), ("artist_name", artist)];
    if let Some(al) = track.album.clone() {
        q.push(("album_name", al));
    }
    if let Some(d) = track.duration_ms {
        if d > 0 {
            q.push(("duration", (d / 1000).to_string()));
        }
    }

    // ③ LRCLIB
    let client = reqwest::Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| format!("HTTP 客户端初始化失败：{e}"))?;
    let resp = client
        .get("https://lrclib.net/api/get")
        .query(&q)
        .send()
        .await
        .map_err(|e| format!("歌词服务请求失败：{e}"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None); // 库里没有这首
    }
    let data: LrclibResp = resp
        .error_for_status()
        .map_err(|e| format!("歌词服务返回错误：{e}"))?
        .json()
        .await
        .map_err(|e| format!("歌词响应解析失败：{e}"))?;
    let Some(text) = data.synced_lyrics.or(data.plain_lyrics) else {
        return Ok(None);
    };
    if text.trim().is_empty() {
        return Ok(None);
    }

    // ④ 落盘（下次直接命中缓存）
    let dir = lyrics_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建歌词目录：{e}"))?;
    std::fs::write(&cache, &text).map_err(|e| format!("歌词写入失败：{e}"))?;
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lyrics_dir_under_data_dir() {
        assert!(lyrics_dir().ends_with("lyrics"));
    }

    /// 真网冒烟（默认 ignore——CI 离线；本地 `cargo test -p musicforge-gui -- --ignored` 手动）。
    #[tokio::test]
    #[ignore = "requires network（本地手动执行）"]
    async fn live_lyrics_smoke() {
        let client = reqwest::Client::builder()
            .user_agent(UA)
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap();
        let resp = client
            .get("https://lrclib.net/api/get")
            .query(&[
                ("track_name", "海阔天空"),
                ("artist_name", "Beyond"),
                ("album_name", "乐与怒"),
            ])
            .send()
            .await
            .expect("LRCLIB 请求应成功");
        assert!(
            resp.status().is_success() || resp.status() == reqwest::StatusCode::NOT_FOUND,
            "LRCLIB 应返回歌词或 404，实际 {}",
            resp.status()
        );
        if resp.status().is_success() {
            let d: LrclibResp = resp.json().await.expect("响应应可解析");
            let has = d
                .synced_lyrics
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
                || d
                    .plain_lyrics
                    .as_deref()
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false);
            assert!(has, "命中时应至少有一种歌词文本");
        }
    }
}
