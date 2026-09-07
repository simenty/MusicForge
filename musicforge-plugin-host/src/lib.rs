//! MusicForge 插件 Host（P6a 预研）：进程 spawn + D20 握手 + 带超时的 NDJSON 调用。
//!
//! 预研范围（排雷目标）：
//! 1. **持久子进程 + 行协议**：stdin 写请求行、stdout 读响应行，按 `id` 路由；
//! 2. **超时隔离**：单请求超时 → kill 子进程 → `MF-PLUGIN-TIMEOUT`（kill -9
//!    后主进程存活是 P6a 硬验收，本预研实证机制）；
//! 3. **D20 握手**：`plugin.manifest` → api_version 区间协商，
//!    不兼容 → `MF-PLUGIN-API-INCOMPATIBLE`；
//! 4. **最小请求模型**：调用方只可能传 `IdentifyRequest` 等强类型——
//!    禁发字段在类型层面不存在（协议 crate 设计），e2e 再做序列化 grep 断言。
//!
//! 本 crate **零网络**（子进程是本地 stdio；CI 离线闸覆盖）。
//! feature 隔离（D8）：CLI/GUI 在正式 P6a 才以 `plugin-host` feature 可选接入
//! ——预研阶段它们不依赖本 crate（默认构建天然无 host 符号）。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use musicforge_plugin_api::{api_compatible, codes, PluginManifest, Request, Response};

// ---------------------------------------------------------------- 限制三件套 --

/// P6a 限制三件套（handover §6.9）：超时 10–30s / 并发 1–2 / 降级完整性。
///
/// 降级完整性（插件全挂/全禁时五域 100% 可用）不落在本 crate——
/// 它是「默认构建无 host 符号」（D8/T1）+ 「core 零插件依赖」的结构性保证，
/// 由 CI feature 断言与 `musicforge-cli/tests/p6a_degradation.rs` 验收。
pub mod limits {
    /// 单请求超时下限（太短的窗口会把慢模型误杀为超时）
    pub const TIMEOUT_MIN_MS: u64 = 10_000;
    /// 单请求超时上限（防插件拖死主流程；超时即 kill，P6a 硬验收）
    pub const TIMEOUT_MAX_MS: u64 = 30_000;
    /// 同时存活的插件进程数上限（P6a 限 2：识别 + 歌词/封面各一即够）
    pub const MAX_CONCURRENT_PLUGINS: usize = 2;

    /// 把调用方给的超时夹取到允许窗口 [`TIMEOUT_MIN_MS`], [`TIMEOUT_MAX_MS`]。
    pub fn clamp_timeout(ms: u64) -> u64 {
        ms.clamp(TIMEOUT_MIN_MS, TIMEOUT_MAX_MS)
    }
}

// ---- 并发槽位（进程级；RAII 释放，防误配置 spawn 出超限插件进程群）----

static SLOT_USED: Mutex<usize> = Mutex::new(0);
static SLOT_CV: Condvar = Condvar::new();

/// 一个插件进程占用槽位的 RAII 守卫：Drop 时释放并唤醒等待者。
#[derive(Debug)]
pub struct PluginSlotGuard;

impl Drop for PluginSlotGuard {
    fn drop(&mut self) {
        let mut used = SLOT_USED.lock().unwrap_or_else(|e| e.into_inner());
        *used = used.saturating_sub(1);
        SLOT_CV.notify_one();
    }
}

/// 尝试获取一个插件进程槽位（已满 → `None`，调用方显式排队或放弃）。
pub fn try_acquire_plugin_slot() -> Option<PluginSlotGuard> {
    let mut used = SLOT_USED.lock().unwrap_or_else(|e| e.into_inner());
    if *used >= limits::MAX_CONCURRENT_PLUGINS {
        return None;
    }
    *used += 1;
    Some(PluginSlotGuard)
}

/// 阻塞获取一个插件进程槽位（有释放即唤醒）。
pub fn acquire_plugin_slot() -> PluginSlotGuard {
    let mut used = SLOT_USED.lock().unwrap_or_else(|e| e.into_inner());
    while *used >= limits::MAX_CONCURRENT_PLUGINS {
        used = SLOT_CV.wait(used).unwrap_or_else(|e| e.into_inner());
    }
    *used += 1;
    PluginSlotGuard
}

/// Host 侧错误（独立于 core 的 `NcmError`——D8：core 不依赖 host）。
#[derive(Debug)]
pub enum PluginHostError {
    Spawn(String),
    Handshake(String),
    ApiIncompatible { plugin: String, host_range: String },
    Timeout { method: String, ms: u64 },
    Protocol(String),
    Io(std::io::Error),
    Gone,
}

impl std::fmt::Display for PluginHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(s) => write!(f, "插件进程启动失败: {s}"),
            Self::Handshake(s) => write!(f, "握手失败: {s}"),
            Self::ApiIncompatible { host_range, .. } => write!(
                f,
                "{}: 插件 api_version 不在 Host 支持区间 {host_range} 内",
                codes::API_INCOMPATIBLE
            ),
            Self::Timeout { method, ms } => {
                write!(
                    f,
                    "{}: {method} 超时（{ms}ms，子进程已终止）",
                    codes::TIMEOUT
                )
            }
            Self::Protocol(s) => write!(f, "协议错误: {s}"),
            Self::Io(e) => write!(f, "IO: {e}"),
            Self::Gone => write!(f, "子进程已退出"),
        }
    }
}

impl std::error::Error for PluginHostError {}

impl PluginHostError {
    /// 稳定码（与插件域 codes 对齐；供 GUI/报告消费）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::ApiIncompatible { .. } => codes::API_INCOMPATIBLE,
            Self::Timeout { .. } => codes::TIMEOUT,
            Self::Spawn(_) | Self::Gone => codes::FAILED,
            Self::Handshake(_) | Self::Protocol(_) => codes::MANIFEST_INVALID,
            Self::Io(_) => "MF-IO-FAILED",
        }
    }
}

/// 运行中的插件子进程（持久 + 行协议）。
pub struct PluginProcess {
    child: Child,
    stdin: ChildStdin,
    responses: Arc<Mutex<HashMap<String, Response>>>,
    reader: Option<std::thread::JoinHandle<()>>,
    pub manifest: PluginManifest,
    pub program: PathBuf,
}

impl PluginProcess {
    /// spawn + D20 握手（不兼容即 kill 并返回 `ApiIncompatible`）。
    pub fn spawn(program: &Path, host_range: &str) -> Result<Self, PluginHostError> {
        let mut child = Command::new(program)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| PluginHostError::Spawn(e.to_string()))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| PluginHostError::Spawn("stdin 不可用".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| PluginHostError::Spawn("stdout 不可用".into()))?;

        let responses: Arc<Mutex<HashMap<String, Response>>> = Arc::new(Mutex::new(HashMap::new()));
        let reader_map = Arc::clone(&responses);
        let reader = std::thread::spawn(move || {
            let mut lines = BufReader::new(stdout).lines();
            while let Some(Ok(line)) = lines.next() {
                let Ok(resp) = serde_json::from_str::<Response>(&line) else {
                    continue; // 非协议行（如插件误打 stderr 内容）宽容跳过
                };
                reader_map
                    .lock()
                    .map(|mut m| m.insert(resp.id.clone(), resp))
                    .ok();
            }
        });

        let mut proc = Self {
            child,
            stdin,
            responses,
            reader: Some(reader),
            // manifest 先占位，握手后填充
            manifest: PluginManifest {
                name: String::new(),
                api_version: String::new(),
                kind: musicforge_plugin_api::PluginKind::Ai,
                network: false,
                data_sent: vec![],
                data_not_sent: vec![],
                ack_required: false,
            },
            program: program.to_path_buf(),
        };

        // ---- D20 握手 ----
        let resp = proc
            .call("plugin.manifest", serde_json::json!({}), 5_000)
            .inspect_err(|_| proc.kill())?;
        let manifest: PluginManifest = serde_json::from_value(resp)
            .inspect_err(|_| proc.kill())
            .map_err(|e| PluginHostError::Handshake(e.to_string()))?;
        if !api_compatible(&manifest.api_version, host_range) {
            proc.kill();
            return Err(PluginHostError::ApiIncompatible {
                plugin: manifest.api_version.clone(),
                host_range: host_range.to_string(),
            });
        }
        proc.manifest = manifest;
        Ok(proc)
    }

    /// 发起一次调用：写请求行 → 等待同 id 响应 → 超时 kill。
    pub fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
        timeout_ms: u64,
    ) -> Result<serde_json::Value, PluginHostError> {
        let id = format!("req-{}", self.next_id());
        let req = Request {
            id: id.clone(),
            method: method.to_string(),
            params,
        };
        let line =
            serde_json::to_string(&req).map_err(|e| PluginHostError::Protocol(e.to_string()))?;

        {
            let w = &mut self.stdin;
            w.write_all(line.as_bytes())
                .and_then(|_| w.write_all(b"\n"))
                .and_then(|_| w.flush())
                .map_err(PluginHostError::Io)?;
        }

        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            let polled = self.responses.lock().ok().and_then(|mut m| m.remove(&id));
            if let Some(resp) = polled {
                return match resp {
                    Response {
                        ok: true,
                        result: Some(v),
                        ..
                    } => Ok(v),
                    Response {
                        ok: false,
                        error: Some(e),
                        ..
                    } => Err(PluginHostError::Protocol(format!(
                        "{}: {}",
                        e.code, e.message
                    ))),
                    other => Err(PluginHostError::Protocol(format!("畸形响应: {other:?}"))),
                };
            }
            if Instant::now() >= deadline {
                self.kill();
                return Err(PluginHostError::Timeout {
                    method: method.to_string(),
                    ms: timeout_ms,
                });
            }
            // 子进程提前退出（崩溃/被杀）→ 立即失败而非干等超时
            if let Ok(Some(_)) = self.child.try_wait() {
                // 给 reader 线程最后一拍收尾
                std::thread::sleep(Duration::from_millis(50));
                return Err(PluginHostError::Gone);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// [`PluginProcess::call`] 的强类型封装：成功响应直接解码为目标类型。
    ///
    /// 解码失败 → `MF-PLUGIN-FAILED` 域的 `Protocol` 错误（失败显式可见，不伪装）。
    pub fn call_typed<T: serde::de::DeserializeOwned>(
        &mut self,
        method: &str,
        params: serde_json::Value,
        timeout_ms: u64,
    ) -> Result<T, PluginHostError> {
        let v = self.call(method, params, timeout_ms)?;
        serde_json::from_value(v)
            .map_err(|e| PluginHostError::Protocol(format!("响应解码失败: {e}")))
    }

    /// 限制三件套合规的调用入口：超时被夹取到 10–30s 窗口。
    ///
    /// 生产路径（CLI/GUI AI 流程）一律走本方法；裸 [`PluginProcess::call`]
    /// 仅供夹具/测试使用（e2e 需要 300ms 级短窗验证 kill 机制本身）。
    pub fn call_limited(
        &mut self,
        method: &str,
        params: serde_json::Value,
        timeout_ms: u64,
    ) -> Result<serde_json::Value, PluginHostError> {
        self.call(method, params, limits::clamp_timeout(timeout_ms))
    }

    /// 子进程状态探测（kill 隔离的验收接口）。
    pub fn child_try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, PluginHostError> {
        self.child.try_wait().map_err(PluginHostError::Io)
    }

    /// 终止子进程（超时/Drop/握手失败共用；幂等）。
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn next_id(&self) -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(1);
        SEQ.fetch_add(1, Ordering::SeqCst)
    }
}

impl std::fmt::Debug for PluginProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginProcess")
            .field("manifest", &self.manifest)
            .field("program", &self.program)
            .finish_non_exhaustive()
    }
}

impl Drop for PluginProcess {
    fn drop(&mut self) {
        self.kill();
        // reader 线程**分离不 join**：若子进程是 shim/包装器（如 D20 拒绝路径
        // 的 cmd.exe），kill 只终结直接子进程，孙进程仍握有 stdout 写端——
        // join 会永久阻塞（预研实测命中）。线程随管道最终关闭自然退出。
        if let Some(h) = self.reader.take() {
            std::mem::forget(h);
        }
    }
}
