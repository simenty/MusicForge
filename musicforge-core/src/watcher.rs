//! D13 watcher 三级自动化安全模型（P8 尾项，2026-09-10 主理人批准 notify 依赖）。
//!
//! **三级语义**（ROADMAP D13）：
//! - **T0 登记（默认）**：只登记新文件事件，不自动执行任何操作（观察模式）；
//! - **T1 新文件自动整理**：新音频文件 → 自动 organize 到目标库
//!   （目录级幂等 plan+apply——同目录旧文件 in_place 零动作，新文件被搬）；
//! - **T2 全自动白名单**：T1 + **垃圾文件自动清洗**（只进回收站可还原——
//!   「永不删/移源文件」铁律由既有安全模型保证：clean 走 `.musicforge/trash`，
//!   organize 落回滚清单）。
//!
//! **防抖合并**（P8 验收项）：同路径事件在 `debounce_ms` 窗口内反复出现 =
//! 文件仍在写入——合并为一次处理（最后一次事件起算窗口）；时间由调用方注入
//! （`now_ms`）——测试可控、不依赖真实时钟。
//!
//! 依赖：`notify`（cross-platform FS 通知，零网络）——dependency-policy 登记。

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::error::NcmError;
use crate::organize::{apply_organize_plan, plan_organize, ConflictStrategy, OrganizeOptions};
use crate::scan::{build_clean_plan, is_audio_ext, is_junk_name, scan_library, ScanOptions};

// ---------------------------------------------------------------- 配置 --

/// 自动化等级（D13）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchLevel {
    /// 只登记，不动文件（默认/观察模式）
    T0Register,
    /// 新音频文件自动整理（organize 到目标库）
    T1AutoOrganize,
    /// T1 + 垃圾自动清洗（只进回收站）
    T2AutoWhitelist,
}

impl WatchLevel {
    /// CLI/配置字符串解析（`t0`/`t1`/`t2`）。
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "t0" | "register" => Some(Self::T0Register),
            "t1" | "organize" => Some(Self::T1AutoOrganize),
            "t2" | "whitelist" => Some(Self::T2AutoWhitelist),
            _ => None,
        }
    }
}

/// watcher 配置。
#[derive(Debug, Clone)]
pub struct WatcherConfig {
    pub level: WatchLevel,
    /// T1/T2：整理目标根目录（organize 的 target_root）
    pub target_root: Option<PathBuf>,
    /// 整理命名模板（organize 语义）
    pub template: String,
    /// 防抖窗口（毫秒）——同路径事件最后一次出现起算
    pub debounce_ms: u64,
}

// ---------------------------------------------------------------- 防抖 --

/// 防抖合并器：`path -> 最后一次事件时间（ms）`。
#[derive(Debug, Default)]
pub struct Debouncer {
    pending: HashMap<PathBuf, u64>,
    debounce_ms: u64,
}

impl Debouncer {
    pub fn new(debounce_ms: u64) -> Self {
        Self {
            pending: HashMap::new(),
            debounce_ms,
        }
    }

    /// 事件到达：刷新该路径的窗口起点（持续写入 = 持续推迟处理）。
    pub fn feed(&mut self, path: PathBuf, now_ms: u64) {
        self.pending.insert(path, now_ms);
    }

    /// 窗口已稳定（`now - last_seen >= debounce_ms`）的路径出列。
    pub fn settle(&mut self, now_ms: u64) -> Vec<PathBuf> {
        let mut ready = Vec::new();
        let mut keep = HashMap::new();
        for (p, t) in self.pending.drain() {
            if now_ms.saturating_sub(t) >= self.debounce_ms {
                ready.push(p);
            } else {
                keep.insert(p, t);
            }
        }
        self.pending = keep;
        ready.sort();
        ready
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

// ---------------------------------------------------------------- 分派 --

/// 一批稳定事件的处理结果。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct WatchActions {
    /// T0：登记的文件数
    pub registered: usize,
    /// T1/T2：organize 实际移动的文件数
    pub organized: usize,
    /// T2：清洗进回收站的文件数
    pub cleaned: usize,
}

fn ext_of(p: &Path) -> Option<String> {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

fn is_junk_path(p: &Path) -> bool {
    let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    is_junk_name(name).is_some()
        || std::fs::symlink_metadata(p)
            .map(|m| m.len() == 0)
            .unwrap_or(false)
}

/// 处理一批稳定事件（防抖出列的路径）——按等级分派。
///
/// **语义**：
/// - T0：全部登记（零文件操作）；
/// - T1：音频域文件按**父目录**分组 → 目录级幂等 organize（`conflict=Skip`——
///   watcher 自动化绝不覆盖；同目录旧文件 in_place 零动作）；
/// - T2：T1 + 垃圾文件（规则名/零字节）所在目录 → clean plan+apply
///   （**只进回收站**——铁律由 clean 执行器保证：绝不直接删除）。
pub fn handle_event_batch(
    paths: &[PathBuf],
    cfg: &WatcherConfig,
) -> Result<WatchActions, NcmError> {
    let mut actions = WatchActions::default();
    match cfg.level {
        WatchLevel::T0Register => {
            actions.registered = paths.len();
            Ok(actions)
        }
        WatchLevel::T1AutoOrganize | WatchLevel::T2AutoWhitelist => {
            // 配置防呆：无目标库的自动整理 = 文件去向不明——显式报错而非静默跳过
            let Some(target_root) = cfg.target_root.clone() else {
                return Err(NcmError::Db(
                    "watch: T1/T2 需要配置 target_root（整理目标根目录）".into(),
                ));
            };
            let mut audio_dirs: VecDeque<PathBuf> = VecDeque::new();
            let mut junk_dirs: Vec<PathBuf> = Vec::new();
            let mut seen_audio = HashSet::new();
            let mut seen_junk = HashSet::new();
            for p in paths {
                match ext_of(p) {
                    Some(e) if is_audio_ext(&e) => {
                        if let Some(d) = p.parent() {
                            if seen_audio.insert(d.to_path_buf()) {
                                audio_dirs.push_back(d.to_path_buf());
                            }
                        }
                    }
                    _ => {}
                }
                if cfg.level == WatchLevel::T2AutoWhitelist && is_junk_path(p) {
                    if let Some(d) = p.parent() {
                        if seen_junk.insert(d.to_path_buf()) {
                            junk_dirs.push(d.to_path_buf());
                        }
                    }
                }
            }
            // T1/T2：目录级幂等 organize
            for dir in &audio_dirs {
                let opts = OrganizeOptions {
                    template: &cfg.template,
                    target_root: &target_root,
                    conflict: ConflictStrategy::Skip,
                };
                let plan = plan_organize(dir, &opts)?;
                let outcome = apply_organize_plan(&plan, &task_id())?;
                actions.organized += outcome.moved;
            }
            // T2：垃圾清洗（只进回收站）
            if cfg.level == WatchLevel::T2AutoWhitelist {
                for dir in &junk_dirs {
                    let report = scan_library(dir, &ScanOptions::default())?;
                    let trash_root = dir.join(".musicforge").join("trash");
                    let all_rules: std::collections::HashSet<&'static str> =
                        crate::scan::RULE_CARDS.iter().map(|c| c.id).collect();
                    let plan = build_clean_plan(&report, &all_rules, &trash_root, dir);
                    let outcome = crate::scan::apply_clean_plan(&plan, &task_id())?;
                    actions.cleaned += outcome.moved;
                }
            }
            // 未整理的登记数（垃圾路径等非音频项的可见性）
            actions.registered = paths.len();
            Ok(actions)
        }
    }
}

fn task_id() -> String {
    format!(
        "watch-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    )
}

// ---------------------------------------------------------------- notify --

/// notify 真实监听循环（CLI `watch` 子命令）：事件 → 防抖 → 批处理。
///
/// 退出：进程终止（CLI 形态无优雅停机需求——测试不经过本函数）。
pub fn run_watch(
    watch_dir: &Path,
    cfg: &WatcherConfig,
    on_line: &dyn Fn(&str),
) -> Result<(), NcmError> {
    use notify::Watcher as _;
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher =
        notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if let Ok(ev) = res {
                for p in ev.paths {
                    let _ = tx.send(p);
                }
            }
        })
        .map_err(|e| NcmError::Db(format!("watch: 监听器创建失败: {e}")))?;
    watcher
        .watch(watch_dir, notify::RecursiveMode::Recursive)
        .map_err(|e| NcmError::Db(format!("watch: 监听目录失败: {e}")))?;
    on_line(&format!(
        "watch: 已监听 {}（level={:?}，防抖 {}ms）——Ctrl-C 退出",
        watch_dir.display(),
        cfg.level,
        cfg.debounce_ms
    ));

    let mut deb = Debouncer::new(cfg.debounce_ms);
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(p) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                deb.feed(p, now);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(NcmError::Db("watch: 事件通道断开".into()));
            }
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let settled = deb.settle(now);
        if settled.is_empty() {
            continue;
        }
        let actions = handle_event_batch(&settled, cfg)?;
        if actions.registered > 0 || actions.organized > 0 || actions.cleaned > 0 {
            on_line(&format!(
                "watch: {} 个稳定事件 → 登记 {} / 整理 {} / 清洗 {}",
                settled.len(),
                actions.registered,
                actions.organized,
                actions.cleaned
            ));
        }
    }
}
