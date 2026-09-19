//! 在线封面补全（P6，桌面层）：MusicBrainz 搜索 → Cover Art Archive 下载 →
//! 本地缓存 → 写回 `db.albums.cover_cache`。
//!
//! **网络边界**（docs/dependency-policy.md §4）：桌面 shell 层是 core 之外允许
//! 出网的两处之一（另一处是 updater）。本模块的硬约束：
//! - 仅在用户显式点击「补全封面」时发起请求（无后台轮询、无遥测）；
//! - 匿名访问 MusicBrainz 政策：≤1 req/s 限速 + 可识别 User-Agent（本模块强制）；
//! - 失败一律返回 Err 由前端提示，**不影响任何离线功能**；
//! - 只写封面文件与 `albums.cover_cache` 路径，不碰用户音频。

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Deserializer};

use super::library_db::open_db;

/// MusicBrainz 要求的可识别 User-Agent。
const UA: &str = concat!(
    "MusicForge/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/simenty/MusicForge)"
);

/// 匿名访问 MusicBrainz 的限速：≤1 req/s（政策要求；对 CAA 一并节流）。
static LAST_REQUEST: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// 等待到距上次出网请求 ≥1.1s（全局串行化，所有出网调用共用）。
async fn throttle() {
    loop {
        let wait = {
            let mut g = match LAST_REQUEST.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(), // 中毒只意味着别处 panic 过，数据仍是 Option<Instant>
            };
            match *g {
                Some(t) => {
                    let elapsed = t.elapsed();
                    if elapsed >= Duration::from_millis(1100) {
                        *g = Some(std::time::Instant::now());
                        None
                    } else {
                        Some(Duration::from_millis(1100) - elapsed)
                    }
                }
                None => {
                    *g = Some(std::time::Instant::now());
                    None
                }
            }
        };
        match wait {
            None => return,
            Some(d) => tokio::time::sleep(d).await,
        }
    }
}

/// 封面缓存目录（与状态库同目录：`<数据目录>/covers`）。
fn covers_dir() -> PathBuf {
    musicforge_core::db::default_db_path()
        .parent()
        .map(|p| p.join("covers"))
        .unwrap_or_else(|| PathBuf::from("covers"))
}

/// MusicBrainz release-group 搜索查询（标题 + 可选艺人）。
fn mb_query(title: &str, artist: &str) -> String {
    if artist.trim().is_empty() {
        format!("releasegroup:\"{title}\"")
    } else {
        format!("releasegroup:\"{title}\" AND artist:\"{artist}\"")
    }
}

#[derive(Deserialize)]
struct MbSearch {
    #[serde(rename = "release-groups", default)]
    release_groups: Vec<MbReleaseGroup>,
}

#[derive(Deserialize)]
struct MbReleaseGroup {
    id: String,
    /// 相关度 0-100。真网实测 MB 返回**整数**（旧文档/部分实现为字符串）——
    /// 用宽松反序列化两种都吃（P6 真网冒烟抓到的第一枚）。
    #[serde(default, deserialize_with = "de_score_lenient")]
    score: Option<String>,
}

/// 容忍 `score` 为整数或字符串，统一存为 String。
fn de_score_lenient<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(match v {
        Some(serde_json::Value::Number(n)) => Some(n.to_string()),
        Some(serde_json::Value::String(s)) => Some(s),
        _ => None,
    })
}

/// 相关度闸：<60 视为误配；缺字段或非法值保守放行（MB 偶有省略）。
fn score_ok(score: Option<&str>) -> bool {
    score.and_then(|s| s.parse::<i32>().ok()).is_none_or(|s| s >= 60)
}

/// 为单个专辑抓取封面。返回缓存文件路径；未找到封面 → `Ok(None)`；
/// 网络失败 → `Err`（前端提示，可重试——不影响离线功能）。
#[tauri::command]
pub async fn cover_fetch(album_id: i64) -> Result<Option<String>, String> {
    let (title, artist) = {
        let db = open_db()?;
        let albums = db.list_albums().map_err(|e| e.to_string())?;
        let a = albums
            .into_iter()
            .find(|a| a.id == album_id)
            .ok_or_else(|| format!("专辑 {album_id} 不存在"))?;
        (a.title, a.artist.unwrap_or_default())
    };

    let client = reqwest::Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("HTTP 客户端初始化失败：{e}"))?;

    // ① MusicBrainz：release-group 搜索（取相关度过闸的第一条）
    throttle().await;
    let q = mb_query(&title, &artist);
    let search: MbSearch = client
        .get("https://musicbrainz.org/ws/2/release-group")
        .query(&[("query", q.as_str()), ("fmt", "json"), ("limit", "1")])
        .send()
        .await
        .map_err(|e| format!("MusicBrainz 请求失败：{e}"))?
        .error_for_status()
        .map_err(|e| format!("MusicBrainz 返回错误：{e}"))?
        .json()
        .await
        .map_err(|e| format!("MusicBrainz 响应解析失败：{e}"))?;

    let Some(rg) = search
        .release_groups
        .into_iter()
        .find(|rg| score_ok(rg.score.as_deref()))
    else {
        return Ok(None);
    };

    // ② Cover Art Archive：front-250（302 → 图片；该 release-group 无封面 → 404）
    throttle().await;
    let resp = client
        .get(format!(
            "https://coverartarchive.org/release-group/{}/front-250",
            rg.id
        ))
        .send()
        .await
        .map_err(|e| format!("Cover Art Archive 请求失败：{e}"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let resp = resp
        .error_for_status()
        .map_err(|e| format!("Cover Art Archive 返回错误：{e}"))?;
    let ext = match resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
    {
        Some(ct) if ct.contains("png") => "png",
        Some(ct) if ct.contains("webp") => "webp",
        _ => "jpg",
    };
    let bytes = resp.bytes().await.map_err(|e| format!("封面下载失败：{e}"))?;
    if bytes.len() < 512 {
        return Ok(None); // 可疑小文件不落盘
    }

    // ③ 落盘 + 写回 db（core 只存路径，不做 IO）
    let dir = covers_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建封面目录：{e}"))?;
    let path = dir.join(format!("{album_id}.{ext}"));
    std::fs::write(&path, &bytes).map_err(|e| format!("封面写入失败：{e}"))?;
    let path_s = path.to_string_lossy().into_owned();
    {
        let db = open_db()?;
        db.set_album_cover(album_id, &path_s)
            .map_err(|e| e.to_string())?;
    }
    Ok(Some(path_s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_shape() {
        assert_eq!(
            mb_query("乐与怒", "Beyond"),
            "releasegroup:\"乐与怒\" AND artist:\"Beyond\""
        );
        assert_eq!(mb_query("乐与怒", "  "), "releasegroup:\"乐与怒\"");
    }

    #[test]
    fn score_gate() {
        assert!(score_ok(Some("100")));
        assert!(score_ok(Some("60")));
        assert!(!score_ok(Some("59")), "低相关度拒绝");
        assert!(score_ok(None), "缺字段保守放行");
        assert!(score_ok(Some("abc")), "非法值保守放行");
    }

    #[test]
    fn covers_dir_under_data_dir() {
        assert!(covers_dir().ends_with("covers"));
    }

    /// 真网冒烟（默认 ignore——CI 全程离线；本地 `cargo test -p musicforge-gui -- --ignored` 手动验证）。
    /// 覆盖：MusicBrainz 搜索 → CAA 下载 完整链路 + 限速器真实等待。
    #[tokio::test]
    #[ignore = "requires network（本地手动执行）"]
    async fn live_fetch_smoke() {
        let client = reqwest::Client::builder()
            .user_agent(UA)
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();

        throttle().await;
        let q = mb_query("乐与怒", "Beyond");
        let search: MbSearch = client
            .get("https://musicbrainz.org/ws/2/release-group")
            .query(&[("query", q.as_str()), ("fmt", "json"), ("limit", "1")])
            .send()
            .await
            .expect("MusicBrainz 请求应成功")
            .error_for_status()
            .expect("MusicBrainz 应返回 2xx")
            .json()
            .await
            .expect("响应应可解析");
        let rg = search
            .release_groups
            .into_iter()
            .find(|rg| score_ok(rg.score.as_deref()))
            .expect("应能搜到「乐与怒」");

        throttle().await;
        let resp = client
            .get(format!(
                "https://coverartarchive.org/release-group/{}/front-250",
                rg.id
            ))
            .send()
            .await
            .expect("CAA 请求应成功");
        assert!(
            resp.status().is_success() || resp.status() == reqwest::StatusCode::NOT_FOUND,
            "CAA 应返回图片或 404，实际 {}",
            resp.status()
        );
    }
}
