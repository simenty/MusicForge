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

/// 歌单内移动曲目（拖拽排序；`to_index` 为 0-based 目标下标）。
#[tauri::command]
pub fn playlist_move(playlist_id: i64, track_id: i64, to_index: i64) -> Result<(), String> {
    let db = open_db()?;
    db.playlist_move_track(playlist_id, track_id, to_index)
        .map_err(|e| e.to_string())
}

/// 导出歌单为 `.m3u8`（原生保存对话框；格式与「按分类导出」**共用同一实现**）。
/// 用户取消 → `Ok(None)`；歌单为空 → `Err`（明确提示）。
#[tauri::command]
pub async fn playlist_export(
    app: tauri::AppHandle,
    playlist_id: i64,
) -> Result<Option<serde_json::Value>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (name, items) = {
        let db = open_db()?;
        let name = db
            .list_playlists()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|p| p.id == playlist_id)
            .map(|p| p.name)
            .ok_or_else(|| format!("歌单 {playlist_id} 不存在"))?;
        let items = db.playlist_tracks(playlist_id).map_err(|e| e.to_string())?;
        (name, items)
    };
    if items.is_empty() {
        return Err("歌单为空，无可导出内容".to_string());
    }
    let default_name = format!("{}.m3u8", musicforge_core::template::sanitize(&name));
    let chosen = app
        .dialog()
        .file()
        .add_filter("M3U8 播放列表", &["m3u8"])
        .set_file_name(&default_name)
        .set_title("导出歌单")
        .blocking_save_file();
    let Some(p) = chosen else {
        return Ok(None); // 用户取消
    };
    let dst = p.into_path().map_err(|e| format!("保存路径无效：{e}"))?;
    let entries: Vec<(std::path::PathBuf, String, i64)> = items
        .iter()
        .map(|t| {
            (
                std::path::PathBuf::from(&t.path),
                t.title.clone().unwrap_or_default(),
                t.duration_ms.map(|d| d / 1000).unwrap_or(-1),
            )
        })
        .collect();
    let n =
        musicforge_core::playlist::export_one_m3u8(&dst, &entries).map_err(|e| e.to_string())?;
    Ok(Some(serde_json::json!({
        "path": dst.to_string_lossy(),
        "tracks": n,
    })))
}

/// 歌单封面拼贴（P6.12）：`{ "<playlist_id>": ["<cover_path>", ...] }`——
/// 每单至多 4 张、按曲序去重；无封面/空歌单不出现在映射中。
#[tauri::command]
pub fn playlists_covers() -> Result<serde_json::Value, String> {
    let db = open_db()?;
    let rows = db.playlist_covers(4).map_err(|e| e.to_string())?;
    let mut map: std::collections::HashMap<i64, Vec<String>> = std::collections::HashMap::new();
    for (pid, path) in rows {
        map.entry(pid).or_default().push(path);
    }
    Ok(serde_json::json!(map))
}
