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

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use musicforge_plugin_api::{
    api_compatible, codes, methods, HostCapabilities, InitParams, InitResult, PluginManifest,
    Request, Response,
};

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

// ---- 并发槽位（进程级；RAII 释放；try 语义——满即显式拒绝，不排队）----

static SLOT_USED: Mutex<usize> = Mutex::new(0);

/// 一个插件进程占用槽位的 RAII 守卫：Drop 时释放。
#[derive(Debug)]
pub struct PluginSlotGuard;

impl Drop for PluginSlotGuard {
    fn drop(&mut self) {
        let mut used = SLOT_USED.lock().unwrap_or_else(|e| e.into_inner());
        *used = used.saturating_sub(1);
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

/// 协议模式（P6a-R：init 握手成功 = V1；legacy 插件拒 init → 降级基础信封）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolMode {
    /// v0.1：init 已协商（事件/16MB/artifacts 语义可用）
    V1,
    /// v0.7.0–v0.8.0 基础信封（无 init/无事件）
    Legacy,
}

/// 运行中的插件子进程（持久 + 行协议）。
pub struct PluginProcess {
    child: Child,
    stdin: ChildStdin,
    responses: Arc<Mutex<HashMap<String, Response>>>,
    /// X39：插件主动事件（无 id 消息）队列；调用方 `drain_events()` 消费
    events: Arc<Mutex<VecDeque<(String, serde_json::Value)>>>,
    /// 协议违规计数（乱发 event/超 16MB 等——观测用，不中断连接）
    violations: Arc<AtomicU64>,
    /// 是否存在进行中请求（X39：无请求时的 event 属违规）
    inflight: Arc<AtomicBool>,
    /// P1-2：stderr 日志尾部（脱敏后环形缓冲，最近 64 条）
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    last_used: Instant,
    /// P6a-R：协议模式（init 协商结果）
    pub protocol: ProtocolMode,
    pub work_dir: PathBuf,
    reader: Option<std::thread::JoinHandle<()>>,
    pub manifest: PluginManifest,
    pub program: PathBuf,
}

impl PluginProcess {
    /// spawn + 协议握手（P6a-R v0.1，X39）：
    /// 1. 首调 `plugin.init`（协议协商 + work_dir 授权 + Host 能力声明）；
    /// 2. 插件回 `METHOD-UNKNOWN`（legacy）→ 降级基础信封模式（发 plugin.manifest）；
    /// 3. api_version 区间不兼容 → kill 并返回 `ApiIncompatible`。
    ///
    /// `work_dir` = 插件唯一可写目录（X41 出站资源边界；Host 保证存在）。
    ///
    /// 崩溃计数（§4.2）：Handshake/Timeout/Protocol/Gone 记连续崩溃；ApiIncompatible
    /// 属版本问题不计；成功 spawn 清零——`is_disabled()` 供宿主调用方闸门。
    ///
    /// B10（第三轮审计）：并发槽位在**桥接层**接线（try 语义），本函数不管并发。
    pub fn spawn(
        program: &Path,
        host_range: &str,
        work_dir: &Path,
    ) -> Result<Self, PluginHostError> {
        std::fs::create_dir_all(work_dir)
            .map_err(|e| PluginHostError::Spawn(format!("work_dir 创建失败: {e}")))?;
        let mut child = Command::new(program)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
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
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| PluginHostError::Spawn("stderr 不可用".into()))?;

        let responses: Arc<Mutex<HashMap<String, Response>>> = Arc::new(Mutex::new(HashMap::new()));
        let events: Arc<Mutex<VecDeque<(String, serde_json::Value)>>> =
            Arc::new(Mutex::new(VecDeque::new()));
        let violations = Arc::new(AtomicU64::new(0));
        let inflight = Arc::new(AtomicBool::new(false));
        let stderr_tail: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));

        // ---- P1-2：stderr = 日志通道（捕获 + 脱敏 + 环形尾部 64 条）----
        {
            let tail = Arc::clone(&stderr_tail);
            std::thread::spawn(move || {
                let mut lines = BufReader::new(stderr).lines();
                while let Some(Ok(l)) = lines.next() {
                    let mut guard = tail.lock().unwrap_or_else(|e| e.into_inner());
                    if guard.len() >= 64 {
                        guard.pop_front();
                    }
                    guard.push_back(redact_secrets(&l));
                }
            });
        }

        // ---- stdout 协议线程：响应分派 / 事件路由（X39）/ 16MB 违规（P2）----
        let reader = {
            let rm = Arc::clone(&responses);
            let ev = Arc::clone(&events);
            let vio = Arc::clone(&violations);
            let infl = Arc::clone(&inflight);
            std::thread::spawn(move || {
                let mut r = BufReader::new(stdout);
                let mut buf: Vec<u8> = Vec::with_capacity(4096);
                loop {
                    buf.clear();
                    match r.read_until(b'\n', &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    if buf.len() > musicforge_plugin_api::v1::MAX_MESSAGE_BYTES {
                        vio.fetch_add(1, Ordering::Relaxed); // 超限：丢该行（P9 沙箱再做流式强化）
                        continue;
                    }
                    let line = String::from_utf8_lossy(&buf);
                    let line = line.trim_end_matches(['\n', '\r']);
                    if line.is_empty() {
                        continue;
                    }
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                        continue; // 非协议行宽容跳过
                    };
                    if v.get("id").is_some() {
                        let Ok(resp) = serde_json::from_value::<Response>(v) else {
                            continue;
                        };
                        rm.lock().map(|mut m| m.insert(resp.id.clone(), resp)).ok();
                    } else if let Some(m) = v.get("method").and_then(|m| m.as_str()) {
                        // X39：无进行中请求时的 event = 协议违规（丢消息，不断连）
                        if !infl.load(Ordering::Relaxed) {
                            vio.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        let params = v.get("params").cloned().unwrap_or_default();
                        ev.lock()
                            .map(|mut q| q.push_back((m.to_string(), params)))
                            .ok();
                    } else {
                        vio.fetch_add(1, Ordering::Relaxed);
                    }
                }
            })
        };

        let mut proc = Self {
            child,
            stdin,
            responses,
            events,
            violations,
            inflight,
            stderr_tail,
            last_used: Instant::now(),
            protocol: ProtocolMode::V1,
            work_dir: work_dir.to_path_buf(),
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
                extensions: vec![],
                permissions: Default::default(),
            },
            program: program.to_path_buf(),
        };

        // ---- P6a-R v0.1 握手（X39）；legacy 降级见 match 分支 ----
        let init_params = InitParams {
            protocol_version: musicforge_plugin_api::v1::PROTOCOL_VERSION,
            work_dir: work_dir.display().to_string(),
            locale: std::env::var("LANG").unwrap_or_else(|_| "zh-CN".into()),
            host_capabilities: HostCapabilities::default(),
        };
        let manifest: PluginManifest = match proc.call(
            methods::PLUGIN_INIT,
            serde_json::to_value(&init_params)
                .map_err(|e| PluginHostError::Protocol(e.to_string()))?,
            musicforge_plugin_api::v1::INIT_TIMEOUT_MS,
        ) {
            Ok(v) => {
                let ir: InitResult = match serde_json::from_value(v) {
                    Ok(x) => x,
                    Err(e) => {
                        proc.kill();
                        Self::note_crash(program);
                        return Err(PluginHostError::Handshake(format!("init 响应畸形: {e}")));
                    }
                };
                // 稳定审计 C16（第四轮）：init 双源一致性——顶层 api_version 与
                // 随行 manifest.api_version 必须一致（不一致 = 清单不可信，拒绝）
                if ir.api_version.is_empty()
                    || ir
                        .manifest
                        .as_ref()
                        .map(|m| m.api_version != ir.api_version)
                        .unwrap_or(false)
                {
                    proc.kill();
                    Self::note_crash(program);
                    return Err(PluginHostError::Handshake(format!(
                        "init 双源 api_version 不一致: 顶层 {} / manifest {}",
                        ir.api_version,
                        ir.manifest
                            .as_ref()
                            .map(|m| m.api_version.as_str())
                            .unwrap_or("?")
                    )));
                }
                let m = match ir.manifest {
                    Some(m) => m,
                    None => {
                        proc.kill();
                        Self::note_crash(program);
                        return Err(PluginHostError::Handshake(
                            "init 成功但未随行 manifest".into(),
                        ));
                    }
                };
                if !api_compatible(&m.api_version, host_range) {
                    proc.kill();
                    return Err(PluginHostError::ApiIncompatible {
                        plugin: m.api_version.clone(),
                        host_range: host_range.to_string(),
                    });
                }
                proc.protocol = ProtocolMode::V1;
                m
            }
            // legacy 降级（R26 向后兼容）：旧插件不认识 init → 基础信封 manifest 握手
            Err(PluginHostError::Protocol(m)) if m.contains(codes::METHOD_UNKNOWN) => {
                Self::legacy_handshake(&mut proc, host_range)?
            }
            Err(e) => {
                proc.kill();
                Self::note_crash(program);
                return Err(e);
            }
        };
        Self::note_success(program);
        proc.manifest = manifest;
        Ok(proc)
    }

    /// legacy（v0.7.0–v0.8.0 基础信封）握手：发 manifest → D20 校验 → 标记 Legacy。
    fn legacy_handshake(
        proc: &mut PluginProcess,
        host_range: &str,
    ) -> Result<PluginManifest, PluginHostError> {
        proc.protocol = ProtocolMode::Legacy;
        let resp = proc
            .call(
                methods::PLUGIN_MANIFEST,
                serde_json::json!({}),
                musicforge_plugin_api::v1::INIT_TIMEOUT_MS,
            )
            .inspect_err(|_| {
                proc.kill();
                Self::note_crash(&proc.program);
            })?;
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
        Ok(manifest)
    }

    /// 发起一次调用：写请求行 → 等待同 id 响应 → 超时 kill。
    pub fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
        timeout_ms: u64,
    ) -> Result<serde_json::Value, PluginHostError> {
        // X39：in-flight 标记（Drop 兜底复位）——无请求期间的 event 属协议违规
        self.inflight.store(true, Ordering::Relaxed);
        self.last_used = Instant::now();
        struct InflightGuard(Arc<AtomicBool>);
        impl Drop for InflightGuard {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Relaxed);
            }
        }
        let _inflight = InflightGuard(Arc::clone(&self.inflight));
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

    // ------------------------------------------------------------ P6a-R v0.1 --

    /// 取走插件主动事件（X39：progress/log；调用方在长任务轮询间隙消费）。
    pub fn drain_events(&self) -> Vec<(String, serde_json::Value)> {
        self.events
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default()
    }

    /// 协议违规计数（乱发事件/超 16MB/畸形消息；观测用，不中断连接）。
    pub fn violations(&self) -> u64 {
        self.violations.load(Ordering::Relaxed)
    }

    /// stderr 日志尾部（P1-2：已脱敏，最近 ≤64 行）——错误报告随行带出。
    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr_tail
            .lock()
            .map(|q| q.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// 空闲时长（Host 空闲回收策略 [`v1::IDLE_RECYCLE_MS`] 的判定输入）。
    pub fn idle_ms(&self) -> u64 {
        self.last_used.elapsed().as_millis() as u64
    }

    /// X41 出站资源校验：`rel` 必须是 work_dir 内的相对路径
    /// （拒绝对路径/`..` 逃逸/符号链接逃逸）——校验失败 = 协议违规，产物不取用。
    pub fn resolve_artifact(&self, rel: &str) -> Result<PathBuf, PluginHostError> {
        Self::resolve_artifact_in(&self.work_dir, rel)
    }

    /// [`Self::resolve_artifact`] 的可测形态（注入 work_dir）。
    ///
    /// 稳定审计 C14（第四轮，2026-09-09）：原实现只对**已存在**的 resolved 做
    /// canonical 校验——不存在时直接放行 join 结果。攻击：work_dir 内预置
    /// symlink 目录（`sub -> /etc`）+ 请求不存在的 `sub/passwd`（组件检查通过）
    /// → Host 后续读取时经 symlink 解析逃逸（TOCTOU）。修复：对**最近存在的
    /// 祖先** canonical 化后逐级回挂缺失段再校验（与 format-plugins `guard_path`
    /// 同款思路——同类问题举一反三）。
    pub fn resolve_artifact_in(work_dir: &Path, rel: &str) -> Result<PathBuf, PluginHostError> {
        use std::path::Component;
        let p = Path::new(rel);
        if p.is_absolute() {
            return Err(PluginHostError::Protocol(format!(
                "artifacts 逃逸（绝对路径）: {rel}"
            )));
        }
        for comp in p.components() {
            match comp {
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(PluginHostError::Protocol(format!(
                        "artifacts 逃逸（{comp:?} 组件）: {rel}"
                    )));
                }
                _ => {}
            }
        }
        let resolved = work_dir.join(p);
        // 最近存在祖先 canonical 化 + 逐级回挂缺失段（缺段丢弃 = 穿越漏洞）
        let mut missing_rev: Vec<std::ffi::OsString> = Vec::new();
        let mut acc = resolved.clone();
        let existing = loop {
            match acc.canonicalize() {
                Ok(c) => break c,
                Err(_) => match acc.file_name().map(|f| f.to_os_string()) {
                    Some(n) => {
                        missing_rev.push(n);
                        if !acc.pop() {
                            return Err(PluginHostError::Protocol(format!(
                                "artifacts 逃逸（无法定位 work_dir 内路径）: {rel}"
                            )));
                        }
                    }
                    None => {
                        return Err(PluginHostError::Protocol(format!(
                            "artifacts 逃逸（无法定位 work_dir 内路径）: {rel}"
                        )));
                    }
                },
            }
        };
        let mut canon = existing;
        for n in missing_rev.iter().rev() {
            canon.push(n);
        }
        let wd = work_dir
            .canonicalize()
            .unwrap_or_else(|_| work_dir.to_path_buf());
        if !canon.starts_with(&wd) {
            return Err(PluginHostError::Protocol(format!(
                "artifacts 逃逸（符号链接/边界外）: {rel}"
            )));
        }
        Ok(canon)
    }

    /// §4.2：连续崩溃计数（Handshake/Timeout/Protocol/Gone 记一次；成功清零）。
    pub fn crash_count(program: &Path) -> u32 {
        crash_counts()
            .lock()
            .map(|m| *m.get(&crash_key(program)).unwrap_or(&0))
            .unwrap_or(0)
    }

    /// §4.2：达到 [`v1::CRASH_DISABLE_THRESHOLD`] 的插件本会话禁用判定。
    pub fn is_disabled(program: &Path) -> bool {
        Self::crash_count(program) >= musicforge_plugin_api::v1::CRASH_DISABLE_THRESHOLD
    }

    fn note_crash(program: &Path) {
        if let Ok(mut m) = crash_counts().lock() {
            *m.entry(crash_key(program)).or_insert(0) += 1;
        }
    }

    fn note_success(program: &Path) {
        if let Ok(mut m) = crash_counts().lock() {
            m.remove(&crash_key(program));
        }
    }
}

/// P1-2：stderr 脱敏（key/token/secret/password 类赋值掩码为 `***`）。
///
/// 零依赖实现：扫描 `<marker>`（大小写不敏感）后跟 `=` 或 `": "`/`: ` 的位置，
/// 把到值结束（空白/引号/行尾）的内容替换为 `***`。这是**审计级最小约定**
/// （防日志明文泄密），非密码学承诺——真正敏感进程隔离属 P9 沙箱阶段。
fn redact_secrets(line: &str) -> String {
    const MARKERS: [&str; 4] = ["key", "token", "secret", "password"];
    let lower = line.to_lowercase();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for m in MARKERS {
        let mut from = 0usize;
        while let Some(rel) = lower[from..].find(m) {
            let start = from + rel;
            // C15-1 取舍说明：**不加词边界**——真实密钥名多为连写
            // （apikey/access_token），词边界会把它们拒之门外；宁可多杀
            // （"monkey=1" 被误脱敏无功能影响）。仅处理赋值位命中。
            // 仅在 marker 处于赋值位（其后是 = 或 : ）时视为密钥位
            let after = &lower[start + m.len()..];
            let (sep_len, is_sep) = if let Some(r) = after.strip_prefix("=\"") {
                (2 + r.find('"').map(|i| i + 1).unwrap_or(r.len()), true)
            } else if let Some(r) = after.strip_prefix('=').map(|x| {
                x.find(|c: char| c.is_whitespace() || c == '&' || c == ';' || c == ',')
                    .unwrap_or(x.len())
            }) {
                (1 + r, true)
            } else if let Some(r) = after.strip_prefix("\": \"") {
                (4 + r.find('"').map(|i| i + 1).unwrap_or(r.len()), true)
            } else if let Some(r) = after
                .strip_prefix("\":")
                .map(|x| x.find(|c: char| c.is_whitespace()).unwrap_or(x.len()))
            {
                (2 + r, true)
            } else {
                (0, false)
            };
            if is_sep && sep_len > 1 {
                ranges.push((start, start + m.len() + sep_len));
            }
            from = start + m.len();
        }
    }
    if ranges.is_empty() {
        return line.to_string();
    }
    // C15-2：排序 + 合并重叠区间（"apikey=x token=y" 的 key/token 相邻命中）
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    for (s, e) in ranges {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    let mut out = String::with_capacity(line.len());
    let mut pos = 0usize;
    for (s, e) in merged {
        out.push_str(&line[pos..s]);
        out.push_str("***");
        pos = e;
    }
    out.push_str(&line[pos..]);
    out
}

/// 崩溃计数存储（§4.2；进程级会话语义）。
fn crash_counts() -> &'static Mutex<HashMap<String, u32>> {
    use std::sync::OnceLock;
    static M: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

fn crash_key(program: &Path) -> String {
    // 按完整路径区分插件实例（不同 shim 目录 = 不同 key；并行测试互不干扰）。
    // Windows 路径大小写不敏感 → 归一小写。
    program.display().to_string().to_lowercase()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 稳定审计 C15 回归：redact_secrets 的**区间排序合并**（核心修复）+
    /// 保守脱敏语义（无词边界——真实密钥名多为连写，宁多杀不漏杀）。
    /// 断言口径 = **有效性**（密钥值不得泄露 + `***` 标记存在）而非精确
    /// 字符串（掩码覆盖 marker 自身，精确形态随实现演进）。
    #[test]
    fn redact_secrets_word_boundary_and_merged_ranges() {
        // 连写密钥名（apikey=）必须脱敏——真实世界主形态
        let out = redact_secrets("apikey=SECRET123 ok");
        assert!(!out.contains("SECRET123"), "密钥值不得泄露: {out}");
        assert!(out.contains("***"));
        // C15-2 核心：相邻/重叠区间合并（key 子串命中 + token 命中），
        // 旧实现拼接错乱（片段重复/缺失）
        let out = redact_secrets("apikey=x token=y");
        assert!(!out.contains("x token"), "重叠区间必须合并: {out}");
        assert!(out.contains("***"));
        // JSON 形态
        let out = redact_secrets("{\"api_key\": \"s3cret\", \"n\": 1}");
        assert!(!out.contains("s3cret"), "JSON 值不得泄露: {out}");
        // 无密钥行原样
        assert_eq!(redact_secrets("plain log line"), "plain log line");
    }

    /// 稳定审计 C14 回归：不存在路径的**符号链接祖先**逃逸被拒（TOCTOU）。
    /// Unix 直接建 symlink；Windows 建目录 symlink 需特权——创建失败则跳过
    /// （守卫逻辑已由组件级 + canonical 祖先校验覆盖）。
    #[test]
    fn resolve_artifact_rejects_symlinked_missing_ancestor() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = dir.path().join("sub");
        #[cfg(unix)]
        {
            if std::os::unix::fs::symlink(outside.path(), &link).is_err() {
                return; // 环境不支持 → 跳过（守卫逻辑由 canonical 祖先覆盖）
            }
        }
        #[cfg(windows)]
        {
            if std::os::windows::fs::symlink_dir(outside.path(), &link).is_err() {
                return; // 非管理员/开发者模式 → 跳过
            }
        }
        // `sub/secret.txt` 不存在，但祖先 `sub` 是指向外部目录的 symlink
        let r = PluginProcess::resolve_artifact_in(dir.path(), "sub/secret.txt");
        assert!(r.is_err(), "symlink 祖先逃逸必须被拒绝: {r:?}");
        // 边界内不存在路径仍放行（正常功能）
        assert!(PluginProcess::resolve_artifact_in(dir.path(), "ok/new.txt").is_ok());
    }
}
