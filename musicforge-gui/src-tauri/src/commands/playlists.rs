//! 歌单（P6.4）：CRUD 与曲目增删。
//!
//! 数据层全部在 core db（含去重、顺序维护、单事务语义）；本模块只做 IPC 编组。
//! 注意与 `musicforge_core::playlist`（M3U 导入导出，P4.3）区分——那是**文件级**
//! 清单工具，这里是**库内**歌单。

use super::library_db::{open_db, track_json};

/// 创建歌单（返回新 id）。
#[tauri::command]
pub fn playlist_create(name: String) -> Result<i64, String> {
    let db = open_db()?;
    db.create_playlist(&name).map_err(|e| e.to_string())
}

/// 歌单列表（创建序；含曲目数）。
#[tauri::command]
pub fn playlists_list() -> Result<Vec<serde_json::Value>, String> {
    let db = open_db()?;
    let rows = db.list_playlists().map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "name": p.name,
                "trackCount": p.track_count,
            })
        })
        .collect())
}

/// 歌单内曲目（按歌单顺序；显示名已解析）。
#[tauri::command]
pub fn playlist_tracks(playlist_id: i64) -> Result<Vec<serde_json::Value>, String> {
    let db = open_db()?;
    let rows = db.playlist_tracks(playlist_id).map_err(|e| e.to_string())?;
    Ok(rows.iter().map(track_json).collect())
}

/// 追加曲目（重复与不存在的 id 跳过；返回实际追加数）。
#[tauri::command]
pub fn playlist_add(playlist_id: i64, track_ids: Vec<i64>) -> Result<usize, String> {
    let db = open_db()?;
    db.playlist_add_tracks(playlist_id, &track_ids)
        .map_err(|e| e.to_string())
}

/// 从歌单移除曲目（后续顺序前移）。
#[tauri::command]
pub fn playlist_remove(playlist_id: i64, track_id: i64) -> Result<(), String> {
    let db = open_db()?;
    db.playlist_remove_track(playlist_id, track_id)
        .map_err(|e| e.to_string())
}

/// 重命名歌单。
#[tauri::command]
pub fn playlist_rename(playlist_id: i64, name: String) -> Result<(), String> {
    let db = open_db()?;
    db.playlist_rename(playlist_id, &name)
        .map_err(|e| e.to_string())
}

/// 删除歌单（条目连带清理；**不动曲目行**）。
#[tauri::command]
pub fn playlist_delete(playlist_id: i64) -> Result<(), String> {
    let db = open_db()?;
    db.playlist_delete(playlist_id).map_err(|e| e.to_string())
}
