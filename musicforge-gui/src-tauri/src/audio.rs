//! 播放引擎（P2）：symphonia 解码 + cpal 输出。
//!
//! 架构（为什么这样分层）：
//! - **引擎线程**：唯一持有 `cpal::Stream` 的地方（`Stream` 在 Windows 上
//!   `!Send`，必须创建/持有/销毁于同一线程）；它接收命令、驱动解码、维护队列。
//! - **音频回调**（cpal 实时线程）：只做三件事——从 channel 取块、拷贝并乘
//!   音量、统计。**零锁、零分配**（Mutex 进实时线程 = 优先级反转风险）。
//! - **背压**：解码侧以 `in_flight` 原子计数控制（低于阈值才解码），回调消费
//!   后递减。不用有界 channel 的阻塞 send——阻塞会卡住命令处理（seek/pause
//!   延迟最高可达整个缓冲时长）。
//! - **seek 代际**：seek 递增 `generation`，回调丢弃旧代际的块——无需与实时
//!   线程同步地清空缓冲。
//! - **零 panic**：全文件无 unwrap/expect（锁中毒走 `into_inner` 恢复）。
//!
//! 本轮范围：播放/暂停/跳转/音量/队列/自动下一首/播放历史入库。
//! 托盘与媒体键、设备热插拔、重采样留待后续（蓝图 P2 遗留清单）。

use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use serde::Serialize;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::Decoder;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

/// 队列项（前端从曲目行构造，随 `player_play_queue` 一次性提交）。
#[derive(Debug, Clone)]
pub struct QueueItem {
    pub track_id: i64,
    pub path: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub duration_ms: Option<i64>,
}

/// 播放模式（P6.18）：随机 / 单曲循环 / 列表循环。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PlayMode {
    /// 顺序（默认）
    #[default]
    Normal,
    /// 随机
    Shuffle,
    /// 单曲循环
    RepeatOne,
    /// 列表循环
    RepeatAll,
}

/// 播放状态快照（`position_ms` 由 [`PlayerHandle::status`] 动态计算填充）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerSnapshot {
    /// idle | playing | paused | error
    pub state: &'static str,
    pub error: Option<String>,
    pub track_id: Option<i64>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub duration_ms: Option<i64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub queue_len: usize,
    pub queue_index: Option<usize>,
    /// 播放模式（P6.18）：normal / shuffle / repeatOne / repeatAll
    pub play_mode: PlayMode,
    pub volume: f32,
    pub position_ms: i64,
    pub underruns: u64,
}

impl Default for PlayerSnapshot {
    fn default() -> Self {
        Self {
            state: "idle",
            error: None,
            track_id: None,
            title: None,
            artist: None,
            duration_ms: None,
            sample_rate: None,
            channels: None,
            queue_len: 0,
            queue_index: None,
            play_mode: PlayMode::default(),
            volume: 1.0,
            position_ms: 0,
            underruns: 0,
        }
    }
}

/// 引擎命令。
enum Cmd {
    Play { items: Vec<QueueItem>, index: usize },
    Toggle,
    Pause,
    Stop,
    Next,
    Prev,
    /// 跳到队列中的指定位置（队列抽屉点选）
    Jump { index: usize },
    Seek { ms: i64 },
    SetVolume(f32),
    /// 队列内重排（from→to；越界/相等忽略）。保持当前曲目不变、播放进度不中断。
    QueueMove { from: usize, to: usize },
    /// 从队列移除指定位置（移除当前曲目则从其开头重新加载；否则保持播放）。
    QueueRemove { index: usize },
    /// 追加到队尾（P6.16）：不改动当前播放项，播放进度不受影响。
    QueueAppend { items: Vec<QueueItem> },
    /// 清空队列（P6.16）：停止播放并清空。
    QueueClear,
    /// 插入到当前曲目之后（P6.17）：「下一首播放」，不改动当前项。
    QueueInsertNext { items: Vec<QueueItem> },
    /// 设置播放模式（P6.18）：normal / shuffle / repeatOne / repeatAll
    SetMode { mode: PlayMode },
}

/// 解码产出的样本块（带代际——seek 后旧块被回调丢弃）。
struct Chunk {
    generation: u64,
    data: Vec<f32>,
}

/// 回调正在消费的块。
struct CurrentChunk {
    generation: u64,
    data: Vec<f32>,
    pos: usize,
}

/// 跨线程共享（快照 + 热路径原子计数）。
struct Shared {
    snapshot: Mutex<PlayerSnapshot>,
    generation: AtomicU64,
    /// 当前位置（样本数 = 帧 × 声道；seek 时直接改写为绝对位置）
    samples_played: AtomicU64,
    /// 已解码未消费的样本数（背压信号）
    in_flight: AtomicUsize,
    underruns: AtomicU64,
    volume: AtomicU32,
    /// 输出流已中断（设备拔出/驱动错误）——恢复播放时据此重建流
    stream_broken: std::sync::atomic::AtomicBool,
}

impl Shared {
    fn new() -> Self {
        Self {
            snapshot: Mutex::new(PlayerSnapshot::default()),
            generation: AtomicU64::new(1),
            samples_played: AtomicU64::new(0),
            in_flight: AtomicUsize::new(0),
            underruns: AtomicU64::new(0),
            volume: AtomicU32::new(1.0f32.to_bits()),
            stream_broken: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn lock_snapshot(&self) -> std::sync::MutexGuard<'_, PlayerSnapshot> {
        match self.snapshot.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// 播放器句柄（Tauri State；只含 Send 成员——`Stream` 在引擎线程内）。
///
/// `Clone`：托盘菜单 / 媒体键回调各持一份（共享同一命令通道）。
#[derive(Clone)]
pub struct PlayerHandle {
    tx: Sender<Cmd>,
    shared: Arc<Shared>,
}

impl PlayerHandle {
    /// 启动引擎线程（幂等的前提是调用方只调一次——main.rs 的 manage 保证）。
    pub fn spawn(db_path: PathBuf) -> Self {
        let (tx, rx) = channel::<Cmd>();
        let shared = Arc::new(Shared::new());
        let shared_engine = Arc::clone(&shared);
        let spawned = std::thread::Builder::new()
            .name("mf-audio-engine".into())
            .spawn(move || Engine::new(db_path, shared_engine).run(rx));
        if spawned.is_err() {
            // 线程创建失败（资源耗尽，极罕见）：置错误态——命令会在 send 时失败
            let mut s = shared.lock_snapshot();
            s.state = "error";
            s.error = Some("播放引擎线程创建失败".into());
        }
        Self { tx, shared }
    }

    pub fn play(&self, items: Vec<QueueItem>, index: usize) -> Result<(), String> {
        self.send(Cmd::Play { items, index })
    }

    pub fn toggle(&self) -> Result<(), String> {
        self.send(Cmd::Toggle)
    }

    pub fn pause(&self) -> Result<(), String> {
        self.send(Cmd::Pause)
    }

    pub fn stop(&self) -> Result<(), String> {
        self.send(Cmd::Stop)
    }

    pub fn next(&self) -> Result<(), String> {
        self.send(Cmd::Next)
    }

    pub fn prev(&self) -> Result<(), String> {
        self.send(Cmd::Prev)
    }

    /// 跳到队列中的指定位置（越界忽略）。
    pub fn jump(&self, index: usize) -> Result<(), String> {
        self.send(Cmd::Jump { index })
    }

    pub fn seek(&self, ms: i64) -> Result<(), String> {
        self.send(Cmd::Seek { ms })
    }

    pub fn set_volume(&self, v: f32) -> Result<(), String> {
        self.send(Cmd::SetVolume(v))
    }

    /// 队列内重排（前端队列抽屉的上/下）。
    pub fn queue_move(&self, from: usize, to: usize) -> Result<(), String> {
        self.send(Cmd::QueueMove { from, to })
    }

    /// 从队列移除（前端队列抽屉的移除）。
    pub fn queue_remove(&self, index: usize) -> Result<(), String> {
        self.send(Cmd::QueueRemove { index })
    }

    /// 追加到队尾（前端「加入队列」）。
    pub fn queue_append(&self, items: Vec<QueueItem>) -> Result<(), String> {
        self.send(Cmd::QueueAppend { items })
    }

    /// 清空队列（前端队列抽屉的清空）。
    pub fn queue_clear(&self) -> Result<(), String> {
        self.send(Cmd::QueueClear)
    }

    /// 插入到当前曲目之后（前端「下一首播放」）。
    pub fn queue_insert_next(&self, items: Vec<QueueItem>) -> Result<(), String> {
        self.send(Cmd::QueueInsertNext { items })
    }

    /// 设置播放模式（P6.18）。
    pub fn set_mode(&self, mode: PlayMode) -> Result<(), String> {
        self.send(Cmd::SetMode { mode })
    }

    /// 状态快照（含动态位置；前端轮询此命令）。
    pub fn status(&self) -> PlayerSnapshot {
        let mut s = self.shared.lock_snapshot().clone();
        let rate = s.sample_rate.unwrap_or(48_000).max(1) as u64;
        let channels = s.channels.unwrap_or(2).max(1) as u64;
        let played = self.shared.samples_played.load(Ordering::Relaxed);
        s.position_ms = (played.saturating_mul(1000) / (rate * channels)) as i64;
        s.underruns = self.shared.underruns.load(Ordering::Relaxed);
        s.volume = f32::from_bits(self.shared.volume.load(Ordering::Relaxed));
        s
    }

    fn send(&self, cmd: Cmd) -> Result<(), String> {
        self.tx.send(cmd).map_err(|_| "播放引擎不可用".to_string())
    }
}

/// 背压阈值：在飞样本超过 ~1.5 秒（48kHz 立体声折算）即暂停解码。
const MAX_IN_FLIGHT: usize = 48_000 * 2 * 3 / 2;
/// 解码块送入通道的容量（每块约 1024–8192 帧；容量给足避免 try_send 失败）
const CHANNEL_CAP: usize = 64;

/// 引擎（运行于专属线程；持有 cpal stream 与队列）。
struct Engine {
    db_path: PathBuf,
    shared: Arc<Shared>,
    queue: Vec<QueueItem>,
    index: usize,
    playing: bool,
    /// 当前输出流对应的 (采样率, 声道)——切换不同参数的曲目时重建
    stream_spec: Option<(u32, u16)>,
    stream: Option<cpal::Stream>,
    sample_tx: Option<SyncSender<Chunk>>,
    decoder: Option<ActiveDecoder>,
    generation: u64,
    /// 播放模式（P6.18）
    mode: PlayMode,
    /// 随机模式用的轻量 PRNG 种子（xorshift64；时间播种，仅供 shuffle，无需加密强度）
    rng_seed: u64,
}

impl Engine {
    fn new(db_path: PathBuf, shared: Arc<Shared>) -> Self {
        Self {
            db_path,
            shared,
            queue: Vec::new(),
            index: 0,
            playing: false,
            stream_spec: None,
            stream: None,
            sample_tx: None,
            decoder: None,
            generation: 1,
            mode: PlayMode::Normal,
            rng_seed: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x1234_5678_90AB_CDEF),
        }
    }

    fn run(mut self, rx: Receiver<Cmd>) {
        loop {
            let mut idle = true;
            loop {
                match rx.try_recv() {
                    Ok(cmd) => {
                        idle = false;
                        self.handle(cmd);
                    }
                    Err(TryRecvError::Empty) => break,
                    // 句柄已全部释放（应用退出路径）：结束线程，Stream 随之 drop
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            self.pump();
            if idle {
                std::thread::sleep(Duration::from_millis(4));
            }
        }
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Play { items, index } => {
                // 越界 index 会被静默滞留（`load_current` 用 `queue.get()` 兜住），
                // 直到下一次 `insert_next` 用它算 splice 起点 → 越界 panic；
                // release 是 panic=abort，等于整个应用退出。故入口即钳到合法范围。
                self.queue = items;
                self.index = if self.queue.is_empty() {
                    0
                } else {
                    index.min(self.queue.len() - 1)
                };
                self.load_current(true);
            }
            Cmd::Toggle => {
                if self.playing {
                    self.pause();
                } else {
                    self.resume();
                }
            }
            Cmd::Pause => self.pause(),
            Cmd::Stop => {
                self.playing = false;
                self.decoder = None;
                self.silence_output();
                self.set_idle();
            }
            Cmd::Next => self.advance(1),
            Cmd::Prev => self.advance(-1),
            Cmd::Jump { index } => {
                if index < self.queue.len() {
                    self.index = index;
                    self.load_current(true);
                }
            }
            Cmd::Seek { ms } => self.seek(ms),
            Cmd::SetVolume(v) => {
                let v = v.clamp(0.0, 1.0);
                self.shared.volume.store(v.to_bits(), Ordering::Relaxed);
                self.shared.lock_snapshot().volume = v;
            }
            Cmd::QueueMove { from, to } => self.reorder(from, to),
            Cmd::QueueRemove { index } => self.remove_from_queue(index),
            Cmd::QueueAppend { items } => self.append(items),
            Cmd::QueueClear => self.clear_queue(),
            Cmd::QueueInsertNext { items } => self.insert_next(items),
            Cmd::SetMode { mode } => {
                self.mode = mode;
                self.shared.lock_snapshot().play_mode = mode;
            }
            }
    }

    /// 解码推进（背压内）。
    fn pump(&mut self) {
        if !self.playing {
            return;
        }
        if self.shared.in_flight.load(Ordering::Relaxed) > MAX_IN_FLIGHT {
            return;
        }
        let Some(dec) = self.decoder.as_mut() else { return };
        match dec.next_chunk() {
            Ok(Some(data)) => {
                let n = data.len();
                if let Some(tx) = &self.sample_tx {
                    let chunk = Chunk { generation: self.generation, data };
                    // 通道满（背压已在上方控制）：本块丢弃且位置不前移
                    if tx.try_send(chunk).is_ok() {
                        self.shared.in_flight.fetch_add(n, Ordering::Relaxed);
                    }
                }
            }
            Ok(None) => {
                // 曲终 → 自动下一首（按当前模式：顺序停 / 随机 / 单曲重放 / 列表循环）
                self.auto_advance();
            }
            Err(e) => self.fail(e),
        }
    }

    fn load_current(&mut self, autoplay: bool) {
        self.decoder = None;
        self.flush_channel();
        let Some(item) = self.queue.get(self.index).cloned() else {
            self.set_idle();
            return;
        };
        match ActiveDecoder::open(Path::new(&item.path)) {
            Ok(dec) => {
                let spec = (dec.sample_rate(), dec.channels());
                // 断流状态下加载新曲：强制重建（复用会拿到已死的流）
                if self.shared.stream_broken.swap(false, Ordering::Relaxed) {
                    self.stream = None;
                    self.sample_tx = None;
                    self.stream_spec = None;
                }
                if let Err(e) = self.ensure_stream(spec) {
                    self.fail(e);
                    return;
                }
                self.shared.generation.fetch_add(1, Ordering::Relaxed);
                self.generation = self.shared.generation.load(Ordering::Relaxed);
                self.shared.samples_played.store(0, Ordering::Relaxed);
                self.shared.in_flight.store(0, Ordering::Relaxed);
                self.shared.underruns.store(0, Ordering::Relaxed);
                self.decoder = Some(dec);
                self.playing = autoplay;
                {
                    let mut s = self.shared.lock_snapshot();
                    s.state = if autoplay { "playing" } else { "paused" };
                    s.error = None;
                    s.track_id = Some(item.track_id);
                    s.title = item.title.clone();
                    s.artist = item.artist.clone();
                    s.duration_ms = item.duration_ms;
                    s.sample_rate = Some(spec.0);
                    s.channels = Some(spec.1);
                    s.queue_len = self.queue.len();
                    s.queue_index = Some(self.index);
                }
                if let Some(st) = &self.stream {
                    let _ = st.play();
                }
                // 播放历史（追加日志；每秒级时间戳即可）
                self.record_history(item.track_id);
            }
            Err(e) => self.fail(format!("无法播放 {}: {e}", item.path)),
        }
    }

    fn ensure_stream(&mut self, spec: (u32, u16)) -> Result<(), String> {
        if self.stream.is_some() && self.stream_spec == Some(spec) {
            return Ok(());
        }
        // 参数变化：重建（旧流先 drop，其回调随之结束）
        self.stream = None;
        self.sample_tx = None;
        let (stream, tx) = build_output(spec.0, spec.1, Arc::clone(&self.shared))?;
        stream.play().map_err(|e| format!("音频输出启动失败: {e}"))?;
        self.stream = Some(stream);
        self.sample_tx = Some(tx);
        self.stream_spec = Some(spec);
        Ok(())
    }

    fn pause(&mut self) {
        if self.playing {
            self.playing = false;
            // B7：停声（否则缓冲里最多还有 ~1.5s 继续响，且 underruns 持续累加）
            self.silence_output();
            self.shared.lock_snapshot().state = "paused";
        }
    }

    fn resume(&mut self) {
        if self.decoder.is_none() {
            // 曾在停止态：尝试重新加载当前队列项
            if !self.queue.is_empty() {
                self.load_current(true);
            }
            return;
        }
        // 断流恢复（设备热插拔的自动重试路径）：重建输出流，保留解码器与位置
        if self.shared.stream_broken.swap(false, Ordering::Relaxed) {
            let Some(dec) = self.decoder.as_ref() else { return };
            let spec = (dec.sample_rate(), dec.channels());
            self.stream = None;
            self.sample_tx = None;
            self.stream_spec = None;
            if let Err(e) = self.ensure_stream(spec) {
                self.shared.lock_snapshot().error = Some(e);
                return;
            }
            self.shared.lock_snapshot().error = None;
        }
        self.playing = true;
        self.shared.lock_snapshot().state = "playing";
        if let Some(st) = &self.stream {
            let _ = st.play();
        }
    }

    /// 用户手动上一首/下一首（P6.18：按模式分派；单曲循环忽略、随机取随机项）。
    fn advance(&mut self, delta: i32) {
        match self.mode {
            PlayMode::Shuffle => self.goto_shuffle(),
            _ => self.goto_sequential(delta),
        }
    }

    /// 曲终自动推进（P6.18）：单曲循环重放当前；其余与手动一致。
    fn auto_advance(&mut self) {
        match self.mode {
            PlayMode::RepeatOne => self.load_current(true),
            PlayMode::Shuffle => self.goto_shuffle(),
            _ => self.goto_sequential(1),
        }
    }

    /// 顺序 / 列表循环：相对跳转；列表循环在越界处回卷，否则到尾停止。
    fn goto_sequential(&mut self, delta: i32) {
        let len = self.queue.len();
        if len == 0 {
            return;
        }
        let next = self.index as i64 + delta as i64;
        if next < 0 {
            if self.mode == PlayMode::RepeatAll {
                self.index = len - 1;
                self.load_current(true);
            }
            return;
        }
        if next as usize >= len {
            if self.mode == PlayMode::RepeatAll {
                self.index = 0;
                self.load_current(true);
            } else {
                self.playing = false;
                self.decoder = None;
                self.silence_output();
                self.set_idle();
            }
            return;
        }
        self.index = next as usize;
        self.load_current(true);
    }

    /// 随机：选一个与当前不同的随机项（P6.18）。上一首/下一首在随机模式下皆随机。
    fn goto_shuffle(&mut self) {
        let len = self.queue.len();
        if len == 0 {
            return;
        }
        if len == 1 {
            self.load_current(true);
            return;
        }
        let mut cand = self.index;
        while cand == self.index {
            cand = self.next_random(len);
        }
        self.index = cand;
        self.load_current(true);
    }

    /// 轻量 xorshift64 PRNG（P6.18）。
    fn next_random(&mut self, n: usize) -> usize {
        let mut x = self.rng_seed;
        if x == 0 {
            x = 0x9E37_79B9_7F4A_7C15;
        }
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng_seed = x;
        (x % n as u64) as usize
    }

    fn seek(&mut self, ms: i64) {
        let Some(dec) = self.decoder.as_mut() else { return };
        let ms = ms.max(0);
        match dec.seek(ms) {
            Ok(()) => {
                // 代际 +1：回调丢弃旧块；位置直接改写为绝对样本数
                self.shared.generation.fetch_add(1, Ordering::Relaxed);
                self.generation = self.shared.generation.load(Ordering::Relaxed);
                self.shared.in_flight.store(0, Ordering::Relaxed);
                let rate = dec.sample_rate() as u64;
                let ch = dec.channels() as u64;
                let frames = (ms as u64) * rate / 1000;
                self.shared
                    .samples_played
                    .store(frames * ch, Ordering::Relaxed);
            }
            Err(e) => self.fail(format!("跳转失败: {e}")),
        }
    }

    /// 清空未消费的样本（seek / 切歌时）。
    ///
    /// **B8 根因修复**：注释原写「新增一代际并依赖回调丢弃」，但代码只有
    /// `let _ = tx`——**代际从未被推进**。于是「记账已归零」与「块仍在通道里」
    /// 同时成立：回调照常消费这些**同代际**的旧块并继续 `fetch_sub` → release
    /// 下 `usize` 下溢回绕成天文数字 → `pump` 的 `in_flight > MAX_IN_FLIGHT`
    /// 从此永真 → **解码被背压永久掐死**（用户侧症状：播着播着突然哑掉，无报错，
    /// 直到下一次 load_current 复位才发现得了）。
    ///
    /// 推进代际后，回调对这些旧块走「丢弃且不减计数」分支，与归零后的账自洽。
    fn flush_channel(&mut self) {
        self.shared.generation.fetch_add(1, Ordering::Relaxed);
        self.generation = self.shared.generation.load(Ordering::Relaxed);
        self.shared.in_flight.store(0, Ordering::Relaxed);
    }

    /// 停止出声（B7）：作废残块 + 记账归零 + **暂停 cpal 流**。
    ///
    /// 只置 `playing = false` 是不够的：缓冲里最多还有约 1.5 秒（`MAX_IN_FLIGHT`）
    /// 会继续播出，且其后每个回调（~10ms 一次）仍在累加 `underruns`、占着音频
    /// 设备不放。
    fn silence_output(&mut self) {
        self.flush_channel();
        if let Some(st) = &self.stream {
            let _ = st.pause();
        }
    }

    /// 队列内重排（P6.15）：保持「正在播放的曲目」仍为当前项，故播放进度不中断。
    fn reorder(&mut self, from: usize, to: usize) {
        let n = self.queue.len();
        if n == 0 || from >= n || to >= n || from == to {
            return;
        }
        let cur_id = self.queue.get(self.index).map(|x| x.track_id);
        let item = self.queue.remove(from);
        // 注意：从 `from` 移除后，原 `to` 处的索引需前移一格
        let dest = if to > from { to - 1 } else { to };
        self.queue.insert(dest, item);
        if let Some(id) = cur_id {
            if let Some(p) = self.queue.iter().position(|x| x.track_id == id) {
                self.index = p;
            }
        }
        self.sync_queue_meta();
    }

    /// 从队列移除（P6.15）。
    fn remove_from_queue(&mut self, index: usize) {
        if index >= self.queue.len() {
            return;
        }
        let removing_current = index == self.index;
        self.queue.remove(index);
        if self.queue.is_empty() {
            self.playing = false;
            self.decoder = None;
            self.flush_channel();
            self.set_idle();
            return;
        }
        if removing_current {
            // 删的是正在听的：从新位置（或末项）开头重新加载
            self.index = if index >= self.queue.len() {
                self.queue.len() - 1
            } else {
                index
            };
            self.load_current(true);
        } else {
            // 删的是前面的：当前项左移一位；删后面/本身不是当前 → 索引不变
            if index < self.index {
                self.index -= 1;
            }
            self.sync_queue_meta();
        }
    }

    /// 仅把队列长度与当前索引同步到快照（不重载、不动播放进度）。
    fn sync_queue_meta(&mut self) {
        let mut s = self.shared.lock_snapshot();
        s.queue_len = self.queue.len();
        s.queue_index = Some(self.index);
    }

    /// 追加到队尾（P6.16）：不改动当前播放项，播放进度不受影响。
    fn append(&mut self, items: Vec<QueueItem>) {
        if items.is_empty() {
            return;
        }
        let was_empty = self.queue.is_empty();
        self.queue.extend(items);
        if was_empty {
            self.index = 0;
        }
        self.sync_queue_meta();
    }

    /// 清空队列（P6.16）：停止播放并清空。
    fn clear_queue(&mut self) {
        self.queue.clear();
        self.index = 0;
        self.playing = false;
        self.decoder = None;
        self.silence_output();
        self.set_idle();
    }

    /// 插入到当前曲目之后（P6.17）：「下一首播放」。当前项位置不变 → 播放不中断。
    fn insert_next(&mut self, items: Vec<QueueItem>) {
        if items.is_empty() {
            return;
        }
        let at = if self.queue.is_empty() {
            0
        } else {
            // 同样钳到 len：`index` 可能来自越界 Jump/Play，splice 越界即 panic
            // （saturating_add 防 usize::MAX 回绕）。
            self.index.saturating_add(1).min(self.queue.len())
        };
        self.queue.splice(at..at, items);
        self.sync_queue_meta();
    }

    fn set_idle(&mut self) {        let mut s = self.shared.lock_snapshot();
        s.state = "idle";
        s.track_id = None;
        s.title = None;
        s.artist = None;
        s.duration_ms = None;
        s.sample_rate = None;
        s.channels = None;
        s.queue_index = None;
        s.queue_len = self.queue.len();
    }

    fn fail(&mut self, msg: String) {
        self.playing = false;
        self.decoder = None;
        let mut s = self.shared.lock_snapshot();
        s.state = "error";
        s.error = Some(msg);
    }

    /// 播放历史入库（失败忽略——历史不是播放的前置条件）。
    fn record_history(&self, track_id: i64) {
        let Ok(db) = musicforge_core::db::Db::open(&self.db_path) else {
            return;
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let _ = db.record_play(track_id, now, 0);
    }
}

// ------------------------------------------------------------ cpal 输出 --

/// 建立输出流（内部新建解码→回调通道，返回发送端）。
///
/// 采样格式策略：先试 `f32`（WASAPI/CoreAudio 原生），失败回退 `i16`
/// （部分 ALSA 设备）。参数不支持时给出**可操作**的错误（重采样为后续版本）。
fn build_output(
    rate: u32,
    channels: u16,
    shared: Arc<Shared>,
) -> Result<(cpal::Stream, SyncSender<Chunk>), String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "系统没有可用的音频输出设备".to_string())?;

    // 支持性预检：错误信息比 cpal 的 BuildStreamError 更可操作
    let mut supported_range = false;
    if let Ok(ranges) = device.supported_output_configs() {
        for r in ranges {
            if r.channels() == channels
                && r.min_sample_rate().0 <= rate
                && r.max_sample_rate().0 >= rate
            {
                supported_range = true;
                break;
            }
        }
    }
    if !supported_range {
        return Err(format!(
            "输出设备不支持 {rate}Hz / {channels} 声道（重采样将在后续版本提供）"
        ));
    }

    let cfg = cpal::StreamConfig {
        channels,
        sample_rate: cpal::SampleRate(rate),
        buffer_size: cpal::BufferSize::Default,
    };

    match build_typed::<f32>(&device, &cfg, Arc::clone(&shared)) {
        Ok(pair) => Ok(pair),
        Err(_) => build_typed::<i16>(&device, &cfg, shared)
            .map_err(|e| format!("无法建立音频输出: {e}")),
    }
}

fn build_typed<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    cfg: &cpal::StreamConfig,
    shared: Arc<Shared>,
) -> Result<(cpal::Stream, SyncSender<Chunk>), cpal::BuildStreamError> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Chunk>(CHANNEL_CAP);
    let err_shared = Arc::clone(&shared);
    let stream = device.build_output_stream(
        cfg,
        make_callback::<T>(shared, rx),
        move |_e| {
            // 输出流错误（设备拔出/驱动错误）：标记断流 + 回到「暂停」而非「错误」——
            // 解码器与播放位置得以保留，用户点播放即触发流重建（见 Engine::resume）。
            err_shared
                .stream_broken
                .store(true, std::sync::atomic::Ordering::Relaxed);
            let mut s = err_shared.lock_snapshot();
            s.state = "paused";
            s.error = Some("音频输出中断（设备被移除？点播放重试）".into());
        },
        None,
    )?;
    Ok((stream, tx))
}

/// 回调内的取块状态。
///
/// 与 [`make_callback`] 拆开的原因：这里是多个 Bug 的策源地——**代际丢弃**与
/// **在飞记账**全在这段逻辑里，而它原先被封在 cpal 的回调签名内（`&mut [T]`
/// + `&OutputCallbackInfo`），无法在 CI 里构造，只能靠耳朵/播放器验证。
struct ChunkConsumer {
    current: Option<CurrentChunk>,
}

impl ChunkConsumer {
    fn new() -> Self {
        Self { current: None }
    }

    /// 取块 → 拷贝（并乘音量）→ 统计。零锁零分配（实时线程约束）。
    fn fill<T: SizedSample + FromSample<f32>>(
        &mut self,
        data: &mut [T],
        shared: &Shared,
        rx: &Receiver<Chunk>,
    ) {
        let generation = shared.generation.load(Ordering::Relaxed);
        let volume = f32::from_bits(shared.volume.load(Ordering::Relaxed));
        let n = data.len();
        let mut filled = 0usize;
        while filled < n {
            if let Some(cur) = self.current.as_mut() {
                if cur.generation != generation {
                    // 过期块（seek / 切歌 / 暂停后产生的残块）：丢弃且**不推进
                    // 位置、不减在飞计数**——配合 load_current 的归零才自洽。
                    self.current = None;
                    continue;
                }
                let take = (cur.data.len() - cur.pos).min(n - filled);
                for i in 0..take {
                    data[filled + i] = T::from_sample(cur.data[cur.pos + i] * volume);
                }
                cur.pos += take;
                filled += take;
                // 饱和减（B8 兜底）：即便记账与产量因极端时序出现不一致，release
                // 下裸 `fetch_sub` 下溢会回绕成天文数字 → `pump` 的
                // `in_flight > MAX_IN_FLIGHT` 从此永真 → 解码被背压永久掐死。
                let _ = shared.in_flight.fetch_update(
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                    |v| Some(v.saturating_sub(take)),
                );
                shared
                    .samples_played
                    .fetch_add(take as u64, Ordering::Relaxed);
                if cur.pos >= cur.data.len() {
                    self.current = None;
                }
            } else {
                match rx.try_recv() {
                    Ok(c) => {
                        self.current = Some(CurrentChunk {
                            generation: c.generation,
                            data: c.data,
                            pos: 0,
                        });
                    }
                    Err(_) => {
                        // 欠载：填静音（比循环等待更安全——实时线程不能阻塞）
                        let silent = n - filled;
                        for d in &mut data[filled..] {
                            *d = T::from_sample(0.0f32);
                        }
                        shared.underruns.fetch_add(1, Ordering::Relaxed);
                        // 这部分静音是**真实播出**的时间（单位与 seek / status 一致：
                        // 样本数 = 帧 × 声道）。不推进会让进度系统性落后于真实输出，
                        // 慢设备上累积漂移可达数秒（歌词、进度条对不上）。
                        shared
                            .samples_played
                            .fetch_add(silent as u64, Ordering::Relaxed);
                        break;
                    }
                }
            }
        }
    }
}

/// 实时回调（cpal）：状态与逻辑委托给 [`ChunkConsumer`]。
fn make_callback<T: SizedSample + FromSample<f32>>(
    shared: Arc<Shared>,
    rx: Receiver<Chunk>,
) -> impl FnMut(&mut [T], &cpal::OutputCallbackInfo) + Send + 'static {
    let mut consumer = ChunkConsumer::new();
    move |data: &mut [T], _| {
        consumer.fill(data, &shared, &rx);
    }
}

// ------------------------------------------------------------ symphonia --

/// symphonia 解码器（含格式读取器 + 编解码器 + 首帧预读）。
struct SymDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    sample_rate: u32,
    channels: u16,
    sample_buf: Option<SampleBuffer<f32>>,
    pending: Option<Vec<f32>>,
    eof: bool,
}

impl SymDecoder {
    /// 打开文件并**预读首块**——采样率/声道以真实解码结果为准
    /// （容器头的 codec_params 可能缺失或与真实帧不符）。
    fn open(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("打开文件失败: {e}"))?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &Default::default(), &Default::default())
            .map_err(|e| format!("无法识别的音频格式: {e}"))?;
        let format = probed.format;
        let track = format.default_track().ok_or_else(|| "没有可播放的音轨".to_string())?;
        let track_id = track.id;
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &Default::default())
            .map_err(|e| format!("不支持的编码: {e}"))?;

        let mut dec = Self {
            format,
            decoder,
            track_id,
            sample_rate: 0,
            channels: 0,
            sample_buf: None,
            pending: None,
            eof: false,
        };
        // 预读：拿到首块（同时确定真实采样率/声道）
        let first = dec.decode_next()?;
        match first {
            Some(data) => dec.pending = Some(data),
            None => return Err("音频文件没有可解码的数据".to_string()),
        }
        if dec.sample_rate == 0 || dec.channels == 0 {
            return Err("未能确定音频参数".to_string());
        }
        Ok(dec)
    }

    fn next_chunk(&mut self) -> Result<Option<Vec<f32>>, String> {
        if let Some(p) = self.pending.take() {
            return Ok(Some(p));
        }
        self.decode_next()
    }

    fn decode_next(&mut self) -> Result<Option<Vec<f32>>, String> {
        loop {
            if self.eof {
                return Ok(None);
            }
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => {
                    self.eof = true;
                    return Ok(None);
                }
                Err(SymError::ResetRequired) => {
                    self.eof = true;
                    return Ok(None);
                }
                Err(e) => return Err(format!("读取数据包失败: {e}")),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(audio_buf) => {
                    let spec = *audio_buf.spec();
                    self.sample_rate = spec.rate;
                    self.channels = spec.channels.count() as u16;
                    let frames = audio_buf.capacity() as u64;
                    let need_new = match &self.sample_buf {
                        Some(b) => b.capacity() < audio_buf.capacity(),
                        None => true,
                    };
                    if need_new {
                        self.sample_buf = Some(SampleBuffer::<f32>::new(frames, spec));
                    }
                    let Some(buf) = self.sample_buf.as_mut() else {
                        return Err("内部缓冲区初始化失败".to_string());
                    };
                    buf.copy_interleaved_ref(audio_buf);
                    return Ok(Some(buf.samples().to_vec()));
                }
                // 单个损坏包：跳过（容错——坏一包不应中断整曲）
                Err(SymError::DecodeError(_)) => continue,
                Err(SymError::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => {
                    self.eof = true;
                    return Ok(None);
                }
                Err(e) => return Err(format!("解码失败: {e}")),
            }
        }
    }

    fn seek(&mut self, ms: i64) -> Result<(), String> {
        let time = Time {
            seconds: (ms / 1000) as u64,
            frac: (ms % 1000) as f64 / 1000.0,
        };
        self.decoder.reset();
        self.format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| format!("{e}"))?;
        self.eof = false;
        self.pending = None;
        Ok(())
    }
}

// ------------------------------------------------------------ DSD（P6.3）--

/// DSD 解码器：DSF/DFF → PCM 88.2kHz f32（**非原生 DoP**——原生输出是路线图项）。
/// 位流抽取质量见 core::formats::dsd 的如实标注（8 位求和 + box，非发烧级 FIR）。
struct DsdDecoder {
    reader: musicforge_core::formats::dsd::DsdReader,
    out_rate: u32,
    channels: u16,
}

impl DsdDecoder {
    fn open(path: &Path) -> Result<Self, String> {
        let reader = musicforge_core::formats::dsd::DsdReader::open(path)
            .map_err(|e| format!("DSD 打开失败: {e}"))?;
        let out_rate = reader.out_rate;
        let channels = reader.channels as u16;
        Ok(Self {
            reader,
            out_rate,
            channels,
        })
    }

    fn next_chunk(&mut self) -> Result<Option<Vec<f32>>, String> {
        // 4096 帧/块（~46ms @88.2k）——与 symphonia 路径的块量级一致
        let v = self
            .reader
            .read_pcm_f32(4096)
            .map_err(|e| format!("DSD 解码失败: {e}"))?;
        Ok(if v.is_empty() { None } else { Some(v) })
    }

    fn seek(&mut self, ms: i64) -> Result<(), String> {
        self.reader
            .seek_ms(ms)
            .map_err(|e| format!("DSD 定位失败: {e}"))
    }
}

/// 活动解码器：symphonia（常规格式）/ DSD（DSF/DFF → PCM 抽取）/ ffmpeg 回退。
enum ActiveDecoder {
    Sym(SymDecoder),
    Dsd(DsdDecoder),
    Ff(FfmpegDecoder),
}

impl ActiveDecoder {
    /// 按扩展名分派（dsf/dff → DSD 路径；其余 symphonia），
    /// symphonia 打不开时**回退 ffmpeg**（APE/WavPack/WMA 等——错误信息拼接两段，便于诊断）。
    fn open(path: &Path) -> Result<Self, String> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        if ext == "dsf" || ext == "dff" {
            return DsdDecoder::open(path).map(Self::Dsd);
        }
        match SymDecoder::open(path) {
            Ok(d) => Ok(Self::Sym(d)),
            Err(e) => FfmpegDecoder::open(path, 0)
                .map(Self::Ff)
                .map_err(|fe| format!("{e}；ffmpeg 回退失败：{fe}")),
        }
    }

    fn sample_rate(&self) -> u32 {
        match self {
            Self::Sym(d) => d.sample_rate,
            Self::Dsd(d) => d.out_rate,
            Self::Ff(d) => d.out_rate,
        }
    }

    fn channels(&self) -> u16 {
        match self {
            Self::Sym(d) => d.channels,
            Self::Dsd(d) => d.channels,
            Self::Ff(d) => d.out_channels,
        }
    }

    fn next_chunk(&mut self) -> Result<Option<Vec<f32>>, String> {
        match self {
            Self::Sym(d) => d.next_chunk(),
            Self::Dsd(d) => d.next_chunk(),
            Self::Ff(d) => d.next_chunk(),
        }
    }

    fn seek(&mut self, ms: i64) -> Result<(), String> {
        match self {
            Self::Sym(d) => d.seek(ms),
            Self::Dsd(d) => d.seek(ms),
            Self::Ff(d) => d.seek(ms),
        }
    }
}

// ------------------------------------------------------ ffmpeg 回退（P6.5）--

/// ffmpeg 解码器：symphonia 打不开的格式（APE / WavPack / WMA…）回退到
/// ffmpeg 解码为 **44.1kHz 立体声 f32** PCM（stdout 管道流式读取）。
///
/// - ffmpeg 查找走 core 的五级探测（显式 → **应用同目录** → PATH → 常见位置）——
///   即支持"把 ffmpeg.exe 放到应用旁边"的 sidecar 分发；找不到时报错并给出获取指引，
///   **绝不静默失败**。
/// - seek = 重启进程 + `-ss`（输入级快速定位）；进程在 Drop 时杀掉（防切歌残留）。
const FF_OUT_RATE: u32 = 44_100;
const FF_OUT_CHANNELS: u16 = 2;

/// ffmpeg 输出 PCM 的参数（f32le → stdout）。
/// `-ss` 放 `-i` **前**（输入级 seek）；`-nostdin` 防其抢读 stdin。
fn ffmpeg_pcm_args(path: &Path, ss_ms: i64) -> Vec<std::ffi::OsString> {
    let mut a: Vec<std::ffi::OsString> =
        vec!["-nostdin".into(), "-loglevel".into(), "error".into()];
    if ss_ms > 0 {
        a.push("-ss".into());
        a.push(format!("{:.3}", ss_ms as f64 / 1000.0).into());
    }
    a.push("-i".into());
    a.push(path.into());
    a.extend([
        "-f".into(),
        "f32le".into(),
        "-acodec".into(),
        "pcm_f32le".into(),
        "-ar".into(),
        FF_OUT_RATE.to_string().into(),
        "-ac".into(),
        FF_OUT_CHANNELS.to_string().into(),
        "-".into(),
    ]);
    a
}

struct FfmpegDecoder {
    proc: std::process::Child,
    stdout: std::io::BufReader<std::process::ChildStdout>,
    path: std::path::PathBuf,
    out_rate: u32,
    out_channels: u16,
    eof: bool,
}

impl FfmpegDecoder {
    fn open(path: &Path, at_ms: i64) -> Result<Self, String> {
        use std::process::{Command, Stdio};
        let ff = musicforge_core::ffmpeg::Ffmpeg::find(None).map_err(|e| {
            format!(
                "此格式需要 ffmpeg 解码，但未找到（{e}）。\
                 请安装 ffmpeg 并加入 PATH，或把 ffmpeg.exe 放到应用同目录"
            )
        })?;
        let mut proc = Command::new(&ff.path)
            .args(ffmpeg_pcm_args(path, at_ms))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("启动 ffmpeg 失败: {e}"))?;
        let stdout = proc
            .stdout
            .take()
            .ok_or_else(|| "ffmpeg stdout 不可用".to_string())?;
        Ok(Self {
            proc,
            stdout: std::io::BufReader::new(stdout),
            path: path.to_path_buf(),
            out_rate: FF_OUT_RATE,
            out_channels: FF_OUT_CHANNELS,
            eof: false,
        })
    }

    fn next_chunk(&mut self) -> Result<Option<Vec<f32>>, String> {
        use std::io::Read;
        if self.eof {
            return Ok(None);
        }
        const FRAMES: usize = 4096;
        let ch = FF_OUT_CHANNELS as usize;
        let mut buf = vec![0u8; FRAMES * ch * 4];
        let mut filled = 0usize;
        // read 可能短读：读满或 EOF
        while filled < buf.len() {
            match self.stdout.read(&mut buf[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) => return Err(format!("读取 ffmpeg 输出失败: {e}")),
            }
        }
        if filled == 0 {
            self.eof = true;
            return Ok(None);
        }
        let samples = filled / 4; // 残字节（非 4 对齐）丢弃
        let mut out = Vec::with_capacity(samples);
        for i in 0..samples {
            let b = &buf[i * 4..i * 4 + 4];
            out.push(f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        }
        Ok(Some(out))
    }

    fn seek(&mut self, ms: i64) -> Result<(), String> {
        // 杀掉旧进程再重启（防两个 ffmpeg 并存输出到同一管道）
        let _ = self.proc.kill();
        let _ = self.proc.wait();
        let mut fresh = Self::open(&self.path, ms.max(0))?;
        std::mem::swap(self, &mut fresh);
        // fresh（持有旧的已杀资源）离开作用域 → Drop（再次 kill 无害）
        Ok(())
    }
}

impl Drop for FfmpegDecoder {
    fn drop(&mut self) {
        // 切歌/停止时必须杀掉 ffmpeg——否则进程泄漏
        let _ = self.proc.kill();
        let _ = self.proc.wait();
    }
}

#[cfg(test)]
mod ffmpeg_tests {
    use super::*;

    /// 参数构造：`-ss` 必须在 `-i` 之前（输入级 seek）；0ms 不带 `-ss`；
    /// 末尾必须是 stdout 占位 `-`。
    #[test]
    fn pcm_args_shape() {
        let a = ffmpeg_pcm_args(Path::new("x.ape"), 0);
        let s: Vec<String> = a.iter().map(|x| x.to_string_lossy().into_owned()).collect();
        assert!(s.contains(&"-nostdin".to_string()));
        assert!(!s.contains(&"-ss".to_string()), "0ms 不带 -ss");
        assert_eq!(s.last().map(String::as_str), Some("-"));
        assert!(s.windows(2).any(|w| w[0] == "-ar" && w[1] == "44100"));
        assert!(s.windows(2).any(|w| w[0] == "-ac" && w[1] == "2"));

        let a2 = ffmpeg_pcm_args(Path::new("x.wv"), 12_345);
        let s2: Vec<String> = a2
            .iter()
            .map(|x| x.to_string_lossy().into_owned())
            .collect();
        let ss = s2.iter().position(|x| x == "-ss").unwrap();
        assert_eq!(s2[ss + 1], "12.345");
        let i = s2.iter().position(|x| x == "-i").unwrap();
        assert!(ss < i, "-ss 必须在 -i 之前");
    }

    /// 队列重排 / 移除（P6.15）：保持「正在播放的曲目」不变、索引随之调整；
    /// 移除非当前项不动播放；移除当前项则落到新位置并重新加载。
    #[test]
    fn queue_reorder_and_remove_keep_current() {
        let mut e = Engine::new(PathBuf::from(":memory:"), Arc::new(Shared::new()));
        let mk = |id: i64| QueueItem {
            track_id: id,
            path: format!("t{id}.flac"),
            title: Some(format!("T{id}")),
            artist: Some("a".into()),
            duration_ms: Some(1000),
        };
        e.queue = vec![mk(1), mk(2), mk(3)];
        e.index = 0; // 正在听 track 1

        // 末项移到最前：顺序变 [3,1,2]，但「正在听」仍是 track 1 → index 1
        e.reorder(2, 0);
        assert_eq!(
            e.queue.iter().map(|x| x.track_id).collect::<Vec<_>>(),
            vec![3, 1, 2]
        );
        assert_eq!(e.index, 1, "正在听 track1 保持为当前项");

        // 移除前面的项（track3，index0）：当前项 track1 左移 → index 0，顺序 [1,2]
        e.remove_from_queue(0);
        assert_eq!(
            e.queue.iter().map(|x| x.track_id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(e.index, 0);

        // 移除当前项（track1）：队列剩 [2]，index 落到新位置 0
        e.remove_from_queue(0);
        assert_eq!(
            e.queue.iter().map(|x| x.track_id).collect::<Vec<_>>(),
            vec![2]
        );
        assert_eq!(e.index, 0);
        assert_eq!(e.queue.len(), 1);
    }

    /// 追加到队尾 / 清空（P6.16）：追加不动当前项；清空后队列空且回到 idle。
    #[test]
    fn queue_append_and_clear() {
        let mut e = Engine::new(PathBuf::from(":memory:"), Arc::new(Shared::new()));
        let mk = |id: i64| QueueItem {
            track_id: id,
            path: format!("t{id}.flac"),
            title: Some(format!("T{id}")),
            artist: Some("a".into()),
            duration_ms: Some(1000),
        };
        e.queue = vec![mk(1)];
        e.index = 0;
        // 追加两首：顺序 [1,2,3]，当前项仍是正在听的 track1
        e.append(vec![mk(2), mk(3)]);
        assert_eq!(
            e.queue.iter().map(|x| x.track_id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(e.index, 0, "追加不改当前播放项");
        // 清空：队列空、index 归零
        e.clear_queue();
        assert!(e.queue.is_empty());
        assert_eq!(e.index, 0);
    }

    /// 「下一首播放」（P6.17）：插入到当前曲目之后，当前项位置不变。
    #[test]
    fn queue_insert_next_keeps_current_then_plays() {
        let mut e = Engine::new(PathBuf::from(":memory:"), Arc::new(Shared::new()));
        let mk = |id: i64| QueueItem {
            track_id: id,
            path: format!("t{id}.flac"),
            title: Some(format!("T{id}")),
            artist: Some("a".into()),
            duration_ms: Some(1000),
        };
        e.queue = vec![mk(1), mk(2), mk(3)];
        e.index = 0; // 正在听 track1
        // 在 track1 后插 track9 → [1,9,2,3]，当前项仍是 track1（index 0 不变）
        e.insert_next(vec![mk(9)]);
        assert_eq!(
            e.queue.iter().map(|x| x.track_id).collect::<Vec<_>>(),
            vec![1, 9, 2, 3]
        );
        assert_eq!(e.index, 0, "当前项位置不变");
        // 末尾插入：队列 [1,9,2,3]，index=3（track3）后再插 → 落到尾部
        e.index = 3;
        e.insert_next(vec![mk(8)]);
        assert_eq!(
            e.queue.iter().map(|x| x.track_id).collect::<Vec<_>>(),
            vec![1, 9, 2, 3, 8]
        );
        assert_eq!(e.index, 3, "最后一项后仍插在尾部，index 不变");
    }

    /// 播放模式（P6.18）：列表循环在越界处回卷，顺序模式在尾部停止。
    #[test]
    fn playback_mode_repeat_all_wraps() {
        let mut e = Engine::new(PathBuf::from(":memory:"), Arc::new(Shared::new()));
        let mk = |id: i64| QueueItem {
            track_id: id,
            path: format!("t{id}.flac"),
            title: Some(format!("T{id}")),
            artist: Some("a".into()),
            duration_ms: Some(1000),
        };
        e.queue = vec![mk(1), mk(2), mk(3)];
        e.mode = PlayMode::RepeatAll;
        e.index = 2;
        e.advance(1);
        assert_eq!(e.index, 0, "末首→下一首包到首");
        e.advance(-1); // 从 0 上一首包到尾
        assert_eq!(e.index, 2, "首→上一首包到尾");
        // 顺序模式：从末首下一首→停止（playing=false）
        e.mode = PlayMode::Normal;
        e.index = 2;
        e.advance(1);
        assert!(!e.playing, "顺序模式到尾停止");
    }

    /// 播放模式（P6.18）：单曲循环曲终自动重放当前（index 不变）。
    #[test]
    fn playback_mode_repeat_one_replays_same() {
        let mut e = Engine::new(PathBuf::from(":memory:"), Arc::new(Shared::new()));
        let mk = |id: i64| QueueItem {
            track_id: id,
            path: format!("t{id}.flac"),
            title: Some(format!("T{id}")),
            artist: Some("a".into()),
            duration_ms: Some(1000),
        };
        e.queue = vec![mk(1), mk(2)];
        e.mode = PlayMode::RepeatOne;
        e.index = 1;
        e.auto_advance(); // 曲终自动：仍是当前曲（路径无效只会 fail，index 不变）
        assert_eq!(e.index, 1, "repeat-one：index 不变");
    }

    /// 播放模式（P6.18）：随机模式不会立即重复当前曲。
    #[test]
    fn playback_mode_shuffle_picks_different() {
        let mut e = Engine::new(PathBuf::from(":memory:"), Arc::new(Shared::new()));
        let mk = |id: i64| QueueItem {
            track_id: id,
            path: format!("t{id}.flac"),
            title: Some(format!("T{id}")),
            artist: Some("a".into()),
            duration_ms: Some(1000),
        };
        e.queue = vec![mk(1), mk(2), mk(3)];
        e.mode = PlayMode::Shuffle;
        e.index = 0;
        for _ in 0..8 {
            e.advance(1);
            assert_ne!(e.index, 0, "shuffle：不会立即重复当前曲");
            e.index = 0; // 复位再验
        }
    }

    /// 真机冒烟（需要 ffmpeg；找不到则**跳过不红**——CI 保持离线可过）。
    /// 链路：ffmpeg 生成 0.2s 440Hz 正弦 WavPack → FfmpegDecoder 解码 → 校验帧数与能量。
    #[test]
    fn ffmpeg_decoder_real_smoke() {
        let Ok(ff) = musicforge_core::ffmpeg::Ffmpeg::find(None) else {
            eprintln!("（跳过：本机无 ffmpeg）");
            return;
        };
        let dir = std::env::temp_dir().join(format!("mf-ff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wv = dir.join("sine.wv");
        let st = std::process::Command::new(&ff.path)
            .args([
                "-nostdin",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=0.2",
                "-c:a",
                "wavpack",
                "-y",
            ])
            .arg(&wv)
            .status()
            .expect("ffmpeg 生成 wv 应可执行");
        assert!(st.success(), "ffmpeg 生成 WavPack 失败（编码器缺失？）");

        let mut d = FfmpegDecoder::open(&wv, 0).expect("WavPack 应可打开");
        let mut total = 0usize;
        let mut energy = 0.0f64;
        while let Some(c) = d.next_chunk().unwrap() {
            for v in &c {
                energy += (*v as f64) * (*v as f64);
            }
            total += c.len();
        }
        let frames = total / FF_OUT_CHANNELS as usize;
        assert!(
            (frames as i64 - 8820).abs() < 441,
            "0.2s @44.1k 应约 8820 帧，实际 {frames}"
        );
        let rms = (energy / total as f64).sqrt();
        assert!(rms > 0.2 && rms < 1.0, "440Hz 正弦 RMS 应约 0.7，实际 {rms:.3}");

        // seek：从头 100ms 起 → 约 4410 帧
        d.seek(100).unwrap();
        let mut t2 = 0usize;
        while let Some(c) = d.next_chunk().unwrap() {
            t2 += c.len();
        }
        let f2 = t2 / FF_OUT_CHANNELS as usize;
        assert!(
            (f2 as i64 - 4410).abs() < 441,
            "seek 100ms 后应约 4410 帧，实际 {f2}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod audio_state_tests {
    use super::*;

    fn eng() -> Engine {
        // 构造引擎但不启线程（run() 另需 Receiver）；flush/channel 逻辑与线程无关
        Engine::new(PathBuf::from("noop.db"), Arc::new(Shared::new()))
    }

    fn sample_chan() -> (SyncSender<Chunk>, Receiver<Chunk>) {
        std::sync::mpsc::sync_channel(CHANNEL_CAP)
    }

    /// **B8 根因回归**：`flush_channel` 必须**真的推进代际**。
    ///
    /// 修复前它只把 `in_flight` 归零、代际纹丝不动（注释却写着已抬代际）→
    /// 「记账已归零」与「旧块仍在通道里」同时成立 → 回调照常消费并继续
    /// `fetch_sub` → release 下 `usize` 下溢回绕成天文数字 → `pump` 的
    /// `in_flight > MAX_IN_FLIGHT` 从此永真 → 解码被背压**永久掐死**（哑掉）。
    #[test]
    fn engine_flush_bumps_generation_and_zeroes_in_flight() {
        let shared = Arc::new(Shared::new());
        let mut e = Engine::new(PathBuf::from("noop.db"), Arc::clone(&shared));
        let gen0 = shared.generation.load(Ordering::Relaxed);
        shared.in_flight.store(1234, Ordering::Relaxed);

        e.flush_channel();

        assert_eq!(
            shared.in_flight.load(Ordering::Relaxed),
            0,
            "flush 必须把在飞计数归零"
        );
        assert!(
            shared.generation.load(Ordering::Relaxed) > gen0,
            "flush 必须推进代际，否则通道里的旧块仍会被正常消费并继续减计数（B8）"
        );
        assert_eq!(
            e.generation,
            shared.generation.load(Ordering::Relaxed),
            "引擎本地代际必须与共享代际同步，否则新产出的块会被回调判为过期"
        );
    }

    /// 推进代际后，通道里的旧块必须：不播出 / 不推进位置 / 不减已归零的计数。
    #[test]
    fn stale_blocks_are_dropped_silently_after_flush() {
        let shared = Arc::new(Shared::new());
        let (tx, rx) = sample_chan();
        let gen0 = shared.generation.load(Ordering::Relaxed);
        tx.send(Chunk {
            generation: gen0,
            data: vec![1.0f32; 4],
        })
        .unwrap();
        // 模拟修复后的 flush（见上一条用例保证 Engine 真的这么做）
        shared.generation.store(gen0 + 1, Ordering::Relaxed);
        shared.in_flight.store(0, Ordering::Relaxed);

        let mut c = ChunkConsumer::new();
        let mut buf = [0f32; 8];
        c.fill(&mut buf, &shared, &rx);

        assert_eq!(
            shared.in_flight.load(Ordering::Relaxed),
            0,
            "旧块不得再减已归零的在飞计数（下溢会回绕并永久掐死解码）"
        );
        assert_eq!(
            shared.underruns.load(Ordering::Relaxed),
            1,
            "通道里只剩过期块 → 必然走欠载填静音路径"
        );
        assert_eq!(
            shared.samples_played.load(Ordering::Relaxed),
            8,
            "位置只能来自填充的静音（8 个样本），旧块本身贡献 0"
        );
        assert!(
            buf.iter().all(|&s| s == 0.0),
            "旧块的数据不得播出（其数据是 1.0，一旦播出 buffer 里必现 1.0）"
        );
    }

    /// 对照：同代际的块仍须正常播出并正确记账（确认上述修复没有误伤正常播放）。
    #[test]
    fn current_generation_block_plays_and_accounts() {
        let shared = Arc::new(Shared::new());
        let (tx, rx) = sample_chan();
        let gen = shared.generation.load(Ordering::Relaxed);
        shared.in_flight.store(4, Ordering::Relaxed);
        tx.send(Chunk {
            generation: gen,
            data: vec![0.5f32; 4],
        })
        .unwrap();

        let mut c = ChunkConsumer::new();
        let mut buf = [0f32; 4];
        c.fill(&mut buf, &shared, &rx);

        assert_eq!(shared.in_flight.load(Ordering::Relaxed), 0, "消费完毕应归零");
        assert_eq!(shared.samples_played.load(Ordering::Relaxed), 4);
        assert!(
            buf.iter().all(|&s| (s - 0.5).abs() < 1e-6),
            "音量 1.0 时应原样播出"
        );
    }

    /// 欠载填充的静音是**真实播出**的时间：不计入位置会让进度系统性落后
    /// （慢设备/大曲子上累积漂移可达数秒，歌词与进度条对不上）。
    #[test]
    fn underrun_silence_still_advances_position() {
        let shared = Arc::new(Shared::new());
        let (_tx, rx) = sample_chan();
        let mut c = ChunkConsumer::new();
        let mut buf = [0f32; 16];
        c.fill(&mut buf, &shared, &rx);

        assert_eq!(shared.underruns.load(Ordering::Relaxed), 1, "应记一次欠载");
        assert_eq!(
            shared.samples_played.load(Ordering::Relaxed),
            16,
            "欠载期间播出的静音必须计入位置"
        );
    }

    /// `eng()` 兜底：确保上述用例依赖的构造路径始终可用（无音频设备也可构造）。
    #[test]
    fn engine_constructs_without_audio_device() {
        let e = eng();
        assert!(!e.playing);
        assert!(e.queue.is_empty());
        assert_eq!(e.index, 0);
    }
}
