// 曲库数据 IPC 命令（P1 曲库体验）：维度层读写 + 媒体源管理 + 索引构建。
//
// 数据层 = core db v2（sources/artists/albums/tracks）+ core library 索引层；
// 命令层只做三件事：打开默认库 → 调 core → JSON 映射（camelCase）。
// **零业务逻辑**——GUI / CLI / NAS 三端共用同一份 core 实现，不可能分叉。

/// 打开默认状态库（本地配置目录；D16「库不放网络位置」守卫在 core 内）。
fn open_db() -> Result<musicforge_core::db::Db, String> {
    musicforge_core::db::Db::open(&musicforge_core::db::default_db_path()).map_err(|e| e.to_string())
}

/// 曲目行 → 前端 JSON（camelCase；分页契约见 `list_tracks`）。
fn track_json(t: &musicforge_core::db::TrackRow) -> serde_json::Value {
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
