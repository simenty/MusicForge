//! 系统媒体键 / 媒体面板（P2）：Windows SMTC / macOS Now Playing。
//!
//! 仅 Windows / macOS 编译（见 Cargo.toml 的 target-specific 依赖注释：
//! souvlaki 的 Linux 后端依赖 libdbus，musl 交叉编译不可行，MPRIS 留待后续）。
//!
//! 线程模型：`MediaControls` 在**本模块自己的线程内**创建、attach 与更新
//! （souvlaki 句柄不保证 Send，同线程持有规避平台差异）；事件回调只 clone
//! `PlayerHandle`（Send）回发命令。元数据走 1s 轮询（与前端同思路：状态小、
//! 不对引擎做事件订阅假设）。初始化失败（无媒体面板环境/权限）静默降级——
//! 媒体键不可用不影响播放。

use std::time::Duration;

use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition,
    PlatformConfig,
};

use crate::audio::PlayerHandle;

/// 启动媒体键线程（main.rs 的 setup 只调一次）。
///
/// `hwnd` 以 `usize` 传递：窗口句柄是裸指针（`*mut c_void`），不实现 `Send`，
/// 无法跨线程移交；转 `usize` 后在新线程内还原（winit 生态的标准做法）。
pub fn spawn(handle: PlayerHandle, hwnd: Option<usize>) {
    let spawned = std::thread::Builder::new()
        .name("mf-media-controls".into())
        .spawn(move || run(handle, hwnd.map(|h| h as *mut std::ffi::c_void)));
    // 线程创建失败 → 媒体键降级，不影响主功能
    let _ = spawned;
}

fn run(handle: PlayerHandle, hwnd: Option<*mut std::ffi::c_void>) {
    let config = PlatformConfig {
        dbus_name: "musicforge",
        display_name: "MusicForge",
        hwnd,
    };
    let Ok(mut controls) = MediaControls::new(config) else {
        return; // 无媒体面板环境：静默降级
    };

    let cb = handle.clone();
    if controls
        .attach(move |event: MediaControlEvent| match event {
            MediaControlEvent::Play | MediaControlEvent::Pause | MediaControlEvent::Toggle => {
                let _ = cb.toggle();
            }
            MediaControlEvent::Next => {
                let _ = cb.next();
            }
            MediaControlEvent::Previous => {
                let _ = cb.prev();
            }
            MediaControlEvent::Stop => {
                let _ = cb.stop();
            }
            MediaControlEvent::SetPosition(p) => {
                let _ = cb.seek(p.0.as_millis() as i64);
            }
            _ => {}
        })
        .is_err()
    {
        return;
    }

    // 元数据轮询：仅在「曲目或状态」变化时更新面板（面板调用是有成本的）
    let mut last: Option<(Option<i64>, &'static str)> = None;
    loop {
        std::thread::sleep(Duration::from_millis(1000));
        let s = handle.status();
        let key = (s.track_id, s.state);
        if last.as_ref() == Some(&key) {
            continue;
        }
        last = Some(key);
        match s.state {
            "playing" | "paused" => {
                let _ = controls.set_metadata(MediaMetadata {
                    title: s.title.as_deref(),
                    artist: s.artist.as_deref(),
                    album: None,
                    cover_url: None,
                    duration: s.duration_ms.map(|d| Duration::from_millis(d as u64)),
                });
                let progress = Some(MediaPosition(Duration::from_millis(
                    s.position_ms.max(0) as u64,
                )));
                let _ = controls.set_playback(if s.state == "playing" {
                    MediaPlayback::Playing { progress }
                } else {
                    MediaPlayback::Paused { progress }
                });
            }
            "idle" => {
                let _ = controls.set_playback(MediaPlayback::Stopped);
            }
            _ => {}
        }
    }
}
