// 曲库数据 IPC 命令（P1 曲库体验）：维度层读写 + 媒体源管理 + 索引构建。
//
// 数据层 = core db v2（sources/artists/albums/tracks）+ core library 索引层；
// 命令层只做三件事：打开默认库 → 调 core → JSON 映射（camelCase）。
// **零业务逻辑**——GUI / CLI / NAS 三端共用同一份 core 实现，不可能分叉。

/// 打开默认状态库（本地配置目录；D16「库不放网络位置」守卫在 core 内）。
pub(crate) fn open_db() -> Result<musicforge_core::db::Db, String> {
    musicforge_core::db::Db::open(&musicforge_core::db::default_db_path()).map_err(|e| e.to_string())
}

/// 曲目行 → 前端 JSON（camelCase；分页契约见 `list_tracks`）。
pub(crate) fn track_json(t: &musicforge_core::db::TrackRow) -> serde_json::Value {
    serde_json::json!({
        "id": t.id,
        "sourceId": t.source_id,
        "path": t.path,
        "size": t.size,
        "title": t.title,
        "artist": t.artist,
        "album": t.album,
        "trackNo": t.track_no,
        "durationMs": t.duration_ms,
        "format": t.format,
        "sampleRate": t.sample_rate,
        "bitDepth": t.bit_depth,
        "channels": t.channels,
        "isLossless": t.is_lossless,
    })
}

/// 索引结果 → 前端 JSON。
fn outcome_json(o: &musicforge_core::library::IndexOutcome) -> serde_json::Value {
    serde_json::json!({
        "scannedFiles": o.scanned_files,
        "audio": o.audio,
        "indexed": o.indexed,
        "tagged": o.tagged,
        "untagged": o.untagged,
        "failed": o.failed,
        "removed": o.removed,
    })
}

/// 曲库总览统计。
#[tauri::command]
pub fn library_stats() -> Result<serde_json::Value, String> {
    let db = open_db()?;
    let st = db.library_stats().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "tracks": st.tracks,
        "artists": st.artists,
        "albums": st.albums,
        "totalSize": st.total_size,
        "totalDurationMs": st.total_duration_ms,
    }))
}

/// 分页读取曲目（按 path 稳定排序）。
///
/// **分页是契约而非建议**：core 侧 limit 硬上限 500，IPC 禁止全量序列化——
/// 十万级曲库靠前端虚拟列表按需取页（windowed fetch）。
#[tauri::command]
pub fn list_tracks(limit: Option<i64>, offset: Option<i64>) -> Result<Vec<serde_json::Value>, String> {
    let db = open_db()?;
    let rows = db
        .list_tracks(limit.unwrap_or(200), offset.unwrap_or(0))
        .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(track_json).collect())
}

/// 艺术家聚合列表（按曲目数降序）。
#[tauri::command]
pub fn list_artists() -> Result<Vec<serde_json::Value>, String> {
    let db = open_db()?;
    let rows = db.list_artists().map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .map(|a| {
            serde_json::json!({
                "id": a.id,
                "name": a.name,
                "trackCount": a.track_count,
            })
        })
        .collect())
}

/// 专辑聚合列表（按曲目数降序）。
#[tauri::command]
pub fn list_albums() -> Result<Vec<serde_json::Value>, String> {
    let db = open_db()?;
    let rows = db.list_albums().map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .map(|a| {
            serde_json::json!({
                "id": a.id,
                "title": a.title,
                "artist": a.artist,
                "year": a.year,
                "trackCount": a.track_count,
                "coverPath": a.cover_path,
            })
        })
        .collect())
}

/// 搜索曲目（标题 / 艺术家 / 专辑 / 路径；通配符按字面转义）。
#[tauri::command]
pub fn search_tracks(query: String, limit: Option<i64>) -> Result<Vec<serde_json::Value>, String> {
    let db = open_db()?;
    let rows = db
        .search_tracks(&query, limit.unwrap_or(200))
        .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(track_json).collect())
}

/// 全局搜索（P6.14）：一次返回 曲目 / 专辑 / 艺术家 / 歌单 四组命中
/// （搜索面板开一次只发一次 IPC）。JSON 形状与各自的 list 命令一致。
#[tauri::command]
pub fn search_all(query: String, limit: Option<i64>) -> Result<serde_json::Value, String> {
    let db = open_db()?;
    let hits = db
        .search_all(&query, limit.unwrap_or(8))
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "tracks": hits.tracks.iter().map(track_json).collect::<Vec<_>>(),
        "albums": hits
            .albums
            .iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "title": a.title,
                    "artist": a.artist,
                    "year": a.year,
                    "trackCount": a.track_count,
                    "coverPath": a.cover_path,
                })
            })
            .collect::<Vec<_>>(),
        "artists": hits
            .artists
            .iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "name": a.name,
                    "trackCount": a.track_count,
                })
            })
            .collect::<Vec<_>>(),
        "playlists": hits
            .playlists
            .iter()
            .map(|p| {
                serde_json::json!({
                    "id": p.id,
                    "name": p.name,
                    "trackCount": p.track_count,
                })
            })
            .collect::<Vec<_>>(),
    }))
}

/// 某艺术家的全部曲目（艺术家详情页；按专辑/轨号排序）。
#[tauri::command]
pub fn artist_tracks(artist_id: i64) -> Result<Vec<serde_json::Value>, String> {
    let db = open_db()?;
    let rows = db.tracks_by_artist(artist_id).map_err(|e| e.to_string())?;
    Ok(rows.iter().map(track_json).collect())
}

/// 某专辑的曲目（专辑详情页；按碟/轨号排序）。
#[tauri::command]
pub fn album_tracks(album_id: i64) -> Result<Vec<serde_json::Value>, String> {
    let db = open_db()?;
    let rows = db.tracks_by_album(album_id).map_err(|e| e.to_string())?;
    Ok(rows.iter().map(track_json).collect())
}

/// 媒体源列表（含每源已索引曲目数——源卡展示）。
#[tauri::command]
pub fn sources_list() -> Result<Vec<serde_json::Value>, String> {
    let db = open_db()?;
    let rows = db.list_sources().map_err(|e| e.to_string())?;
    let counts = db.source_track_counts().map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .map(|s| {
            serde_json::json!({
                "id": s.id,
                "path": s.path,
                "label": s.label,
                "enabled": s.enabled,
                "addedAt": s.added_at,
                "tracksCount": counts.get(&s.id).copied().unwrap_or(0),
            })
        })
        .collect())
}

/// 登记媒体源（幂等；重复登记返回同一 id）。
#[tauri::command]
pub fn sources_add(path: String, label: Option<String>) -> Result<serde_json::Value, String> {
    let db = open_db()?;
    let id = db
        .upsert_source(path.trim(), label.as_deref())
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "id": id }))
}

/// 移除媒体源（连带清理其曲目行）。
#[tauri::command]
pub fn sources_remove(id: i64) -> Result<serde_json::Value, String> {
    let db = open_db()?;
    let removed = db.remove_source(id).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "removedTracks": removed }))
}

/// 对已登记媒体源构建/刷新索引（扫描 → 读标签 → 入库 → 清理陈旧行）。
#[tauri::command]
pub fn index_source(source_id: i64) -> Result<serde_json::Value, String> {
    let db = open_db()?;
    let src = db
        .list_sources()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|s| s.id == source_id)
        .ok_or_else(|| format!("媒体源 {source_id} 不存在"))?;
    let out = musicforge_core::library::index_library(
        &db,
        source_id,
        std::path::Path::new(&src.path),
        &musicforge_core::scan::ScanOptions::default(),
    )
    .map_err(|e| e.to_string())?;
    Ok(outcome_json(&out))
}

/// 切换「喜欢」并返回切换后的状态。
#[tauri::command]
pub fn track_toggle_like(track_id: i64) -> Result<serde_json::Value, String> {
    let db = open_db()?;
    let liked = db.toggle_like(track_id).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "liked": liked }))
}

/// 全部已喜欢的曲目 id（一次拉取；前端 Set 判定行状态）。
#[tauri::command]
pub fn liked_ids() -> Result<Vec<i64>, String> {
    let db = open_db()?;
    db.all_liked_ids().map_err(|e| e.to_string())
}

/// 播放历史（倒序；Track 字段 + playedAt / msPlayed）。
#[tauri::command]
pub fn play_history(limit: Option<i64>) -> Result<Vec<serde_json::Value>, String> {
    let db = open_db()?;
    let rows = db
        .list_history(limit.unwrap_or(200))
        .map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .map(|h| {
            let mut v = track_json(&h.track);
            if let Some(obj) = v.as_object_mut() {
                obj.insert("playedAt".to_string(), serde_json::json!(h.played_at));
                obj.insert("msPlayed".to_string(), serde_json::json!(h.ms_played));
            }
            v
        })
        .collect())
}

/// 清空播放历史（返回清空条数）。
#[tauri::command]
pub fn history_clear() -> Result<serde_json::Value, String> {
    let db = open_db()?;
    let n = db.clear_history().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "cleared": n }))
}

/// 喜欢的曲目（分页；按收藏时间倒序）——「我喜欢的音乐」页。
#[tauri::command]
pub fn liked_tracks(limit: Option<i64>, offset: Option<i64>) -> Result<Vec<serde_json::Value>, String> {
    let db = open_db()?;
    let rows = db
        .list_liked(limit.unwrap_or(200), offset.unwrap_or(0))
        .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(track_json).collect())
}

/// 统计总览（一次拉取）：曲库规模 + 行为计数 + 近 7 天 + 最常播放 Top 10。
#[tauri::command]
pub fn stats_overview() -> Result<serde_json::Value, String> {
    let db = open_db()?;
    let lib = db.library_stats().map_err(|e| e.to_string())?;
    let liked = db.liked_count().map_err(|e| e.to_string())?;
    let (plays, played_tracks) = db.history_totals().map_err(|e| e.to_string())?;
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
        - 7 * 86_400;
    let daily = db.daily_play_counts(since).map_err(|e| e.to_string())?;
    let top = db.top_tracks(10).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "tracks": lib.tracks,
        "artists": lib.artists,
        "albums": lib.albums,
        "totalSize": lib.total_size,
        "totalDurationMs": lib.total_duration_ms,
        "liked": liked,
        "plays": plays,
        "playedTracks": played_tracks,
        "daily": daily
            .iter()
            .map(|(day, count)| serde_json::json!({ "day": day, "count": count }))
            .collect::<Vec<_>>(),
        "top": top
            .iter()
            .map(|t| {
                let mut v = track_json(&t.track);
                if let Some(obj) = v.as_object_mut() {
                    obj.insert("playCount".to_string(), serde_json::json!(t.play_count));
                }
                v
            })
            .collect::<Vec<_>>(),
    }))
}

/// 最近播放（按曲目去重）——首页「继续聆听」。
#[tauri::command]
pub fn recent_plays(limit: Option<i64>) -> Result<Vec<serde_json::Value>, String> {
    let db = open_db()?;
    let rows = db
        .recent_tracks(limit.unwrap_or(8))
        .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(track_json).collect())
}

/// 添加媒体源并立即索引（首次向导一键完成）。
#[tauri::command]
pub fn sources_add_and_index(
    path: String,
    label: Option<String>,
) -> Result<serde_json::Value, String> {
    let db = open_db()?;
    let p = path.trim().to_string();
    let id = db
        .upsert_source(&p, label.as_deref())
        .map_err(|e| e.to_string())?;
    let out = musicforge_core::library::index_library(
        &db,
        id,
        std::path::Path::new(&p),
        &musicforge_core::scan::ScanOptions::default(),
    )
    .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "id": id, "outcome": outcome_json(&out) }))
}
