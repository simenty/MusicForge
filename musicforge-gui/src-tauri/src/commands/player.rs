// 播放 IPC 命令（P2）：薄层——命令转发 + 状态读取，零业务逻辑
// （引擎在 crate::audio；本层只做 DTO 转换与 State 取用）。
use tauri::State;

use crate::audio::{PlayerHandle, PlayerSnapshot, QueueItem};

/// 队列项 DTO（前端从曲目行构造，camelCase 对齐 TS 侧）。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItemDto {
    pub track_id: i64,
    pub path: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub duration_ms: Option<i64>,
}

impl From<QueueItemDto> for QueueItem {
    fn from(d: QueueItemDto) -> Self {
        QueueItem {
            track_id: d.track_id,
            path: d.path,
            title: d.title,
            artist: d.artist,
            duration_ms: d.duration_ms,
        }
    }
}

/// 设置队列并从 `index` 开始播放（替换旧队列）。
#[tauri::command]
pub fn player_play_queue(
    state: State<'_, PlayerHandle>,
    items: Vec<QueueItemDto>,
    index: usize,
) -> Result<(), String> {
    state.play(items.into_iter().map(Into::into).collect(), index)
}

/// 播放/暂停切换。
#[tauri::command]
pub fn player_toggle(state: State<'_, PlayerHandle>) -> Result<(), String> {
    state.toggle()
}

/// 暂停。
#[tauri::command]
pub fn player_pause(state: State<'_, PlayerHandle>) -> Result<(), String> {
    state.pause()
}

/// 停止（清空当前曲目，保留队列）。
#[tauri::command]
pub fn player_stop(state: State<'_, PlayerHandle>) -> Result<(), String> {
    state.stop()
}

/// 下一首。
#[tauri::command]
pub fn player_next(state: State<'_, PlayerHandle>) -> Result<(), String> {
    state.next()
}

/// 上一首。
#[tauri::command]
pub fn player_prev(state: State<'_, PlayerHandle>) -> Result<(), String> {
    state.prev()
}

/// 跳到队列中的指定位置（队列抽屉点选）。
#[tauri::command]
pub fn player_jump(state: State<'_, PlayerHandle>, index: usize) -> Result<(), String> {
    state.jump(index)
}

/// 跳转到指定毫秒。
#[tauri::command]
pub fn player_seek(state: State<'_, PlayerHandle>, ms: i64) -> Result<(), String> {
    state.seek(ms)
}

/// 设置音量（0.0–1.0）。
#[tauri::command]
pub fn player_set_volume(state: State<'_, PlayerHandle>, volume: f32) -> Result<(), String> {
    state.set_volume(volume)
}

/// 队列内重排（P6.15）：`from` 移到 `to` 前（越界/相等由引擎忽略）。
#[tauri::command]
pub fn player_queue_move(state: State<'_, PlayerHandle>, from: usize, to: usize) -> Result<(), String> {
    state.queue_move(from, to)
}

/// 从队列移除指定位置（P6.15）。
#[tauri::command]
pub fn player_queue_remove(state: State<'_, PlayerHandle>, index: usize) -> Result<(), String> {
    state.queue_remove(index)
}

/// 播放状态快照（前端轮询：含动态位置/欠载计数）。
#[tauri::command]
pub fn player_status(state: State<'_, PlayerHandle>) -> PlayerSnapshot {
    state.status()
}
