//! MusicForge CLI 批处理逻辑（lib 化以便集成测试；main.rs 只做参数解析与退出码）
//!
//! 硬约束落点：有界并发（默认 4）、单文件失败不中断、skip-existing 带完整性标记、
//! 结构保留 + 命名模板 + **目标名去重**（同渲染名追加 ` (n)`，修 C-N7 类覆盖 bug）、
//! 正确退出码、失败清单导出、repair receipt 式失败原因。
//!
//! 两阶段设计：
//! - **规划阶段（串行）**：打开每个源 → CRC 校验 → 格式判定 → 渲染目标名 → 去重分配。
//!   失败在此直接记为 Failed（错误码 + 建议）。
//! - **执行阶段（并行）**：按分配好的目标落盘 + 写标签 + 写完整性标记。

pub mod manifest;
pub mod safety;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use musicforge_core::{tagger, Decoder, NcmError};

/// 协作式取消令牌（GUI 取消按钮 / 超时控制用）。 workers 在每个文件开始前检查。
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone)]
pub struct BatchConfig {
    /// 文件与目录混合输入；目录按 `recursive` 决定是否递归
    pub inputs: Vec<PathBuf>,
    /// 输出根目录；None = 输出到源文件同目录
    pub out_dir: Option<PathBuf>,
    /// 目录输入是否递归（递归时**保留目录结构**——修上游 C-N7）
    pub recursive: bool,
    /// 跳过已存在且通过完整性标记的输出
    pub skip_existing: bool,
    /// 有界并发（硬约束 10；默认 4，姿态管理——书面意见 Q7）
    pub jobs: usize,
    /// 命名模板（占位符 {title}/{artist}/{album}/{track}/{track:0Nd}/{format}；`/` 产生子目录；逐段清洗）
    pub template: String,
    /// 只规划不落盘（v0.2.0）：产出 manifest 计划条目，不写任何音频/侧车文件
    pub dry_run: bool,
    /// manifest 路径；None = 不写 manifest（v0.2.0 默认会由 CLI 指定）
    pub manifest: Option<PathBuf>,
    /// 取消令牌（None = 不可取消）
    pub cancel: Option<CancelToken>,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            inputs: Vec::new(),
            out_dir: None,
            recursive: false,
            skip_existing: false,
            jobs: 4,
            template: "{artist} - {title}".to_string(),
            cancel: None,
            dry_run: false,
            manifest: None,
        }
    }
}

/// 并发数硬上界（硬约束 10「有界并发」）。
///
/// `run_inner` 里 `jobs` 直接决定 `thread::scope` 起的 OS 线程数。此前只有 `max(1)`
/// 下界，上界完全交给调用方：`musicforge -j 200000` 会尝试创建 20 万个线程，
/// `ScopedThreadBuilder::spawn` 失败即 panic（release 下 `panic = "abort"` 直接崩进程）。
/// 64 远超任何有意义的吞吐上限（磁盘 IO 早已饱和），不影响正常用法。
const MAX_JOBS: usize = 64;

/// 从被污染的 mutex 中恢复数据。
///
/// 本 crate 的 `queue`/`more` 两把锁**从不跨可失败调用持有**（取任务、存结果都只是
/// 一次 push/pop），锁内不可能 panic，因此中毒只意味着「别的线程在别处 panic 过」，
/// 数据本身依然完好。原先 `Err(_) => break` / `if let Ok(..)` 的写法会在那之后
/// **静默丢弃尚未处理的文件与已算出的结果**，表现为「结果数 < 输入数」且无失败记录
/// ——「空失败清单 ≠ 没丢数据」。统一走 `into_inner()` 恢复，计数恒等于输入数。
fn lock_recover<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Skipped,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone)]
pub struct FileResult {
    pub source: PathBuf,
    pub status: Status,
    pub output: Option<PathBuf>,
    /// repair receipt：`<错误码>: <详情> | 建议: <suggestion>`
    pub reason: Option<String>,
    pub tags_written: usize,
}

/// 计划预览条目（`plan_only` 产出；供 GUI 预览面板与 dry-run 使用）。
#[derive(Debug, Clone)]
pub struct PlannedItem {
    pub source: String,
    /// 目标路径；None = 规划失败（`error` 说明原因，如 NCM-BAD-MAGIC）
    pub target: Option<String>,
    /// 判定的音频格式扩展名（如 "flac"）；失败时为 None
    pub format: Option<String>,
    pub error: Option<String>,
}

/// 只规划不执行：对输入做展开 + 逐文件规划（含目标名去重），返回计划条目。
///
/// 与 `run_inner` 共用同一套 `plan_one` 逻辑——预览与执行**不可能分叉**。
pub fn plan_only(
    inputs: &[PathBuf],
    recursive: bool,
    template: &str,
    out_dir: Option<&Path>,
) -> Vec<PlannedItem> {
    let cfg = BatchConfig {
        out_dir: out_dir.map(|p| p.to_path_buf()),
        template: template.to_string(),
        ..Default::default()
    };
    let sources = collect_inputs(inputs, recursive);
    let mut used: HashSet<String> = HashSet::new();
    let mut items = Vec::new();
    for item in sources {
        match plan_one(item.clone(), &cfg, &mut used) {
            Ok(p) => items.push(PlannedItem {
                source: p.source.display().to_string(),
                target: Some(p.target.display().to_string()),
                format: Some(p.fmt.extension().to_string()),
                error: None,
            }),
            Err(e) => items.push(PlannedItem {
                source: item.0.display().to_string(),
                target: None,
                format: None,
                error: Some(format!("{}: {e}", e.code())),
            }),
        }
    }
    items
}

#[derive(Debug)]
pub struct BatchSummary {
    pub results: Vec<FileResult>,
    /// 仅规划未执行的条目数（dry-run 模式）
    pub planned: usize,
    pub ok: usize,
    pub skipped: usize,
    pub cancelled: usize,
    pub failed: usize,
    pub duration_ms: u128,
}

impl BatchSummary {
    /// 正确退出码：有失败 → 1；全部成功/跳过 → 0
    pub fn exit_code(&self) -> i32 {
        if self.failed > 0 {
            1
        } else {
            0
        }
    }

    /// 被取消而未处理的文件数（GUI 展示用）
    pub fn is_cancelled(&self) -> bool {
        self.cancelled > 0
    }

    /// 导出失败清单 CSV（source,code,reason）
    pub fn export_failures_csv(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(path)?;
        writeln!(f, "source,code,reason")?;
        for r in &self.results {
            if r.status == Status::Failed {
                let (code, reason) = match &r.reason {
                    Some(s) => match s.split_once(':') {
                        Some((c, rest)) => (c.to_string(), rest.trim_start().to_string()),
                        None => (String::new(), s.clone()),
                    },
                    None => (String::new(), String::new()),
                };
                writeln!(
                    f,
                    "{},{},{}",
                    csv_escape(&r.source.to_string_lossy()),
                    code,
                    csv_escape(&reason)
                )?;
            }
        }
        f.flush()?;
        Ok(())
    }
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn is_ncm(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("ncm"))
        .unwrap_or(false)
}

/// 递归深度硬上界。
///
/// `walk_recursive` 是**递归**实现且 `ep.is_dir()` 会跟随目录符号链接/junction。
/// Windows 上 junction 可由普通用户创建，一个自引用 junction（`a\b -> a`）就会让
/// 这里无限递归 → 栈溢出 → 进程 abort（硬约束 1 的边界情形，且 release 下
/// `panic = "abort"` 连回溯都没有）。64 远超任何真实音乐库的目录深度。
const MAX_WALK_DEPTH: usize = 64;

fn walk_recursive(
    root: &Path,
    dir: &Path,
    depth: usize,
    out: &mut Vec<(PathBuf, Option<PathBuf>)>,
) {
    if depth > MAX_WALK_DEPTH {
        return;
    }
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            eprintln!("警告：无法读取目录 {}：{e}", dir.display());
            return;
        }
    };
    for e in rd {
        let e = match e {
            Ok(e) => e,
            Err(err) => {
                eprintln!("警告：跳过 {} 下无法访问的目录项：{err}", dir.display());
                continue;
            }
        };
        let ep = e.path();
        if ep.is_symlink() {
            // F3：不跟随符号链接/junction —— 避免越界遍历 + 自引用 junction 重复计数
            continue;
        } else if ep.is_dir() {
            walk_recursive(root, &ep, depth + 1, out);
        } else if is_ncm(&ep) {
            out.push((ep, Some(root.to_path_buf())));
        }
    }
}

/// 收集待处理文件：`(文件路径, 目录输入的根)`。根用于结构保留计算相对路径。
pub fn collect_inputs(inputs: &[PathBuf], recursive: bool) -> Vec<(PathBuf, Option<PathBuf>)> {
    let mut out = Vec::new();
    for p in inputs {
        if p.is_dir() {
            if recursive {
                walk_recursive(p, p, 1, &mut out);
            } else {
                match std::fs::read_dir(p) {
                    Ok(rd) => {
                        for e in rd {
                            let e = match e {
                                Ok(e) => e,
                                Err(err) => {
                                    eprintln!(
                                        "警告：跳过 {} 下无法访问的目录项：{err}",
                                        p.display()
                                    );
                                    continue;
                                }
                            };
                            let ep = e.path();
                            if !ep.is_symlink() && ep.is_file() && is_ncm(&ep) {
                                out.push((ep, Some(p.clone())));
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("警告：无法读取目录 {}：{e}", p.display());
                    }
                }
            }
        } else if p.is_file() && is_ncm(p) {
            out.push((p.clone(), None));
        }
    }
    out.sort();
    out
}

fn target_dir_for(root: Option<&Path>, source: &Path, out_dir: Option<&Path>) -> PathBuf {
    match out_dir {
        Some(out) => {
            // 结构保留：相对路径的父目录映射到输出目录（修上游 C-N7 平铺覆盖）
            let sub = root
                .and_then(|r| source.strip_prefix(r).ok())
                .and_then(|rel| rel.parent())
                .filter(|p| !p.as_os_str().is_empty());
            match sub {
                Some(sub) => out.join(sub),
                None => out.to_path_buf(),
            }
        }
        None => source.parent().map(|p| p.to_path_buf()).unwrap_or_default(),
    }
}

/// 目标名去重键。Windows 文件系统**大小写不敏感** → 用小写键，避免两个仅大小写不同的目标名
/// 指向同一实际文件导致后者静默覆盖前者（真实数据丢失，且零失败报警——只能靠输出计数发现）。
/// Linux/macOS 大小写敏感 → 保留原样（两个大小写不同的文件本就合法共存）。
fn dedup_key(p: &Path) -> String {
    let s = p.to_string_lossy().into_owned();
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s
    }
}

/// 规划阶段产物：源文件 + 已去重的目标路径 + 格式
struct Plan {
    source: PathBuf,
    target: PathBuf,
    fmt: musicforge_core::Format,
    /// 处理该文件的格式适配器 id（P1d 起由 FormatRegistry 分派得出）
    adapter: &'static str,
}

fn plan_one(
    (source, root): (PathBuf, Option<PathBuf>),
    cfg: &BatchConfig,
    used: &mut HashSet<String>,
) -> Result<Plan, NcmError> {
    // P1d：CLI 内部经 FormatRegistry 分派（外部 API 与退出码不变）。
    // 认领不了的文件直接给明确错误，不再交给 Decoder 兜底猜测（G5 教训）。
    let adapter = musicforge_core::formats::registry::builtin_registry()
        .detect_file(&source)
        .ok_or(NcmError::BadMagic)?;
    let adapter: &'static str = adapter.id();

    let mut dec = Decoder::open(&source)?;
    let fmt = dec.detect_format()?;
    let meta = dec.metadata().cloned();
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output")
        .to_string();

    let rendered = musicforge_core::template::render_filename(&cfg.template, meta.as_ref(), &stem);
    let rel = PathBuf::from(&rendered);
    let dir = target_dir_for(root.as_deref(), &source, cfg.out_dir.as_deref())
        .join(rel.parent().unwrap_or(Path::new("")));
    let file_name = rel
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("output")
        .to_string();
    let file_name = if file_name
        .to_ascii_lowercase()
        .ends_with(&format!(".{}", fmt.extension()))
    {
        file_name
    } else {
        format!("{file_name}.{}", fmt.extension())
    };

    // 目标名去重：同渲染名的后续文件追加 " (n)"（浏览器下载命名惯例；修 C-N7 类覆盖）
    // Windows 大小写不敏感 → 用小写键，仅大小写不同的两个目标名视为碰撞（否则静默覆盖丢数据）
    let ext = fmt.extension().to_string();
    let stem2 = Path::new(&file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("out")
        .to_string();

    let mut n = 1usize;
    let target = loop {
        let candidate = if n == 1 {
            dir.join(&file_name)
        } else {
            dir.join(format!("{stem2} ({n}).{ext}"))
        };
        if used.insert(dedup_key(&candidate)) {
            break candidate;
        }
        n += 1;
    };
    Ok(Plan {
        source,
        target,
        fmt,
        adapter,
    })
}

fn execute_one(plan: &Plan, cfg: &BatchConfig) -> FileResult {
    if cfg.cancel.as_ref().is_some_and(|t| t.is_cancelled()) {
        return FileResult {
            source: plan.source.clone(),
            status: Status::Cancelled,
            output: None,
            reason: Some("用户取消".to_string()),
            tags_written: 0,
        };
    }
    let sidecar = PathBuf::from(format!("{}.musicforge.json", plan.target.display()));

    // 增量跳过（完整性标记：sidecar 记录的最终大小与 sha256 双重一致；无标记 = 残缺半成品，覆盖重转）
    if cfg.skip_existing && integrity_marker_ok(&plan.target, &sidecar) {
        return FileResult {
            source: plan.source.clone(),
            status: Status::Skipped,
            output: Some(plan.target.clone()),
            reason: Some("输出已存在且通过完整性标记校验".to_string()),
            tags_written: 0,
        };
    }

    // 阶段 1：解密落盘（dump_to 自身保证任何失败路径都清理半成品）
    let dumped = (|| -> Result<(Option<musicforge_core::Metadata>, Vec<u8>), NcmError> {
        // #11 修复：输出目录延迟到执行阶段再创建。规划阶段不再 create_dir_all，
        // 避免 Skipped / Cancelled 文件在磁盘留下空的残留目录。
        // create_dir_all 幂等，并发执行各自目标不同，无竞态。
        if let Some(parent) = plan.target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut dec = Decoder::open(&plan.source)?;
        // §4.7 崩溃安全：先写同目录临时文件，成功后 rename 原子就位。
        // 中断时目标路径要么不存在、要么是上一次的完整产物，绝不会是半成品。
        let tmp = temp_path_for(&plan.target);
        dec.dump_to(&tmp)?;
        atomic_rename(&tmp, &plan.target)?;
        Ok((dec.metadata().cloned(), dec.cover().to_vec()))
    })();
    let dumped_ok = dumped.is_ok();

    // 阶段 2：写标签（元数据层面）。产物此时已**完整落盘**。
    let outcome: Result<(usize, bool), NcmError> = match dumped {
        Err(e) => Err(e),
        Ok((meta, cover)) => match meta {
            Some(ref m) => tagger::write_tags(&plan.target, plan.fmt, m, &cover),
            None => Ok((0, false)), // 硬约束 11：元数据缺失 → 跳过打标签
        },
    };

    // 落盘成功但后续阶段失败时，产物是完整可用的：
    // 必须如实带出输出路径，否则「文件已在磁盘上、result.output 却是 None」，
    // 用户既看不到也删不掉，重跑又因无 sidecar 反复重转（QA 第二轮）。
    let produced = if dumped_ok {
        Some(plan.target.clone())
    } else {
        None
    };

    match outcome {
        Ok((tags, _)) => {
            // 写完整性标记（sha256 + 最终大小；打标签后的最终状态）
            let marker = (|| -> Result<serde_json::Value, NcmError> {
                use sha2::{Digest, Sha256};
                let mut f = std::fs::File::open(&plan.target)?;
                let mut h = Sha256::new();
                std::io::copy(&mut f, &mut h)?;
                let size = std::fs::metadata(&plan.target)?.len();
                Ok(serde_json::json!({
                    "sha256": hex(h.finalize()),
                    "size": size,
                    // P1d：记录产出该文件的格式适配器（P6b 起外部格式插件可归因审计）
                    "adapter": plan.adapter,
                }))
            })()
            .and_then(|v| {
                let tmp = temp_path_for(&sidecar);
                std::fs::write(&tmp, serde_json::to_string_pretty(&v)?).map_err(NcmError::from)?;
                atomic_rename(&tmp, &sidecar)
            });
            if let Err(e) = marker {
                return FileResult {
                    source: plan.source.clone(),
                    status: Status::Failed,
                    output: produced,
                    reason: Some(format!(
                        "{}: {e} | 音频已导出但完整性标记写入失败（下次运行会重转）：{} | 建议: {}",
                        e.code(),
                        plan.target.display(),
                        e.suggestion()
                    )),
                    tags_written: tags,
                };
            }
            FileResult {
                source: plan.source.clone(),
                status: Status::Ok,
                output: Some(plan.target.clone()),
                reason: None,
                tags_written: tags,
            }
        }
        Err(e) => {
            // 硬约束 11 落地（主理人拍板）：元数据层面的失败不得拖垮整个转换。
            //
            // · `TagRead` = lofty 无法解析输出容器。产物此刻已**完整落盘**，
            //   失败性质属于「源文件元数据/格式层面」→ 判整文件 Failed 违反硬约束 11
            //   「绝不因元数据问题失败整个转换」，故降级为 Ok + 告警式 reason。
            // · `TagWrite` 与其它 = **输出侧环境故障**（只读 / 被播放器占用 / 不可写），
            //   需要用户干预，保持 Failed。二者失败性质不同，不可混为一谈。
            if let (NcmError::TagRead(_), Some(p)) = (&e, &produced) {
                return FileResult {
                    source: plan.source.clone(),
                    status: Status::Ok,
                    output: Some(p.clone()),
                    reason: Some(format!(
                        "NCM-TAG-READ: 音频已完整导出到 {}，但元数据写入失败：{e} | \
                         建议: 检查输出文件是否被播放器占用；删除该文件后重转，\
                         或关闭「跳过已存在」重跑本文件（本次未写完整性标记，\
                         默认会自动重转）",
                        p.display()
                    )),
                    tags_written: 0,
                };
            }
            let reason = match &produced {
                Some(p) => format!(
                    "{}: {e} | 音频已完整导出到 {}，仅元数据写入失败 | 建议: {}",
                    e.code(),
                    p.display(),
                    e.suggestion()
                ),
                None => format!("{}: {e} | 建议: {}", e.code(), e.suggestion()),
            };
            FileResult {
                source: plan.source.clone(),
                status: Status::Failed,
                output: produced,
                reason: Some(reason),
                tags_written: 0,
            }
        }
    }
}

fn hex(b: impl AsRef<[u8]>) -> String {
    b.as_ref().iter().map(|x| format!("{x:02x}")).collect()
}

/// 完整性标记校验（QA-B3：此前仅比对 size，同尺寸损坏的输出会被静默跳过；
/// sidecar 里本就记录了 sha256，必须一并验证）
fn integrity_marker_ok(target: &Path, sidecar: &Path) -> bool {
    let v = match std::fs::read_to_string(sidecar)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
    {
        Some(v) => v,
        None => return false,
    };
    let (Some(recorded_size), Some(recorded_sha)) = (v["size"].as_u64(), v["sha256"].as_str())
    else {
        return false;
    };
    let Ok(md) = std::fs::metadata(target) else {
        return false;
    };
    if md.len() != recorded_size {
        return false;
    }
    use sha2::{Digest, Sha256};
    let Ok(mut f) = std::fs::File::open(target) else {
        return false;
    };
    let mut h = Sha256::new();
    if std::io::copy(&mut f, &mut h).is_err() {
        return false;
    }
    hex(h.finalize()) == recorded_sha
}

/// 两阶段批处理：串行规划（渲染+去重）→ 有界并行执行（硬约束 10；单文件失败不中断）。
/// `on_result` 在每个文件完成后回调（GUI 进度事件 / 测试断言用）。
pub fn run_with_progress(cfg: BatchConfig, on_result: impl Fn(&FileResult) + Sync) -> BatchSummary {
    let sources = collect_inputs(&cfg.inputs, cfg.recursive);
    run_inner(sources, cfg, &on_result, HashSet::new())
}

/// 已展开输入入口（G3 修复）：调用方直接传入 `(文件路径, 目录根)` 对。
/// 修根因：GUI 经 IPC 把目录展开成散文件时丢失 root，导致自定义输出目录下
/// 源目录树不被镜像（与 CLI 行为不一致）。root 语义与 `collect_inputs` 完全一致：
/// `Some(root)` = 该文件来自目录输入（结构镜像相对 root 的父目录），`None` = 散文件。
pub fn run_with_progress_expanded(
    expanded: Vec<(PathBuf, Option<PathBuf>)>,
    cfg: BatchConfig,
    on_result: impl Fn(&FileResult) + Sync,
) -> BatchSummary {
    run_inner(expanded, cfg, &on_result, HashSet::new())
}

fn run_inner(
    sources: Vec<(PathBuf, Option<PathBuf>)>,
    cfg: BatchConfig,
    on_result: &(impl Fn(&FileResult) + Sync),
    reserved: HashSet<String>,
) -> BatchSummary {
    let start = Instant::now();

    // §4.7：清理上次中断留下的临时文件，避免半成品常驻输出目录
    cleanup_stale_temps(cfg.out_dir.as_deref());

    // ---- 规划阶段（串行，天然无竞态；目标名去重）----
    // 续跑时先占住 manifest 中已记录的目标名，避免新一轮因输入变少而
    // 把「测试曲目.flac」这类名字让给别人 → 覆盖上一次的产物。
    let mut used: HashSet<String> = reserved;
    let mut plans: Vec<Plan> = Vec::new();
    let mut results: Vec<FileResult> = Vec::new();
    for item in sources {
        match plan_one(item.clone(), &cfg, &mut used) {
            Ok(p) => plans.push(p),
            Err(e) => {
                let fr = FileResult {
                    source: item.0,
                    status: Status::Failed,
                    output: None,
                    reason: Some(format!("{}: {e} | 建议: {}", e.code(), e.suggestion())),
                    tags_written: 0,
                };
                on_result(&fr);
                results.push(fr);
            }
        }
    }

    // ---- dry-run：只留痕计划，绝不落盘（v0.2.0 安全任务层）----
    if cfg.dry_run {
        let planned = plans.len();
        if let Some(path) = cfg.manifest.as_deref() {
            let task_id = manifest::new_task_id();
            match manifest::Manifest::open(path, &task_id, "dry-run") {
                Ok(mf) => {
                    for p in &plans {
                        if let Err(e) = mf.append(&manifest::ManifestItem {
                            task_id: task_id.clone(),
                            source: p.source.display().to_string(),
                            target: Some(p.target.display().to_string()),
                            actions: vec!["unpack", "write_tags"],
                            source_sha256: None,
                            target_sha256: None,
                            result: "planned",
                            code: None,
                            rollback_available: false,
                            adapter: Some(p.adapter),
                        }) {
                            eprintln!("⚠ manifest 写入失败（dry-run）: {e}");
                        }
                    }
                }
                Err(e) => eprintln!("⚠ manifest 创建失败，跳过留痕: {e}"),
            }
        }
        return BatchSummary {
            results: Vec::new(),
            planned,
            ok: 0,
            skipped: 0,
            cancelled: 0,
            failed: results
                .iter()
                .filter(|r| r.status == Status::Failed)
                .count(),
            duration_ms: start.elapsed().as_millis(),
        };
    }

    // ---- 执行阶段（有界并行；取消令牌在每个文件开始前检查）----
    let task_id = manifest::new_task_id();
    let mf = match cfg.manifest.as_deref() {
        Some(path) => match manifest::Manifest::open(path, &task_id, "convert") {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("⚠ manifest 创建失败，本次转换无留痕: {e}");
                None
            }
        },
        None => None,
    };
    let mf_ref = mf.as_ref();
    let queue: Mutex<VecDeque<Plan>> = Mutex::new(plans.into());
    let more: Mutex<Vec<FileResult>> = Mutex::new(Vec::new());
    // 硬约束 10：并发数两端都要有界（上界见 MAX_JOBS）
    // clamp 要求 min <= max，此处 1 <= MAX_JOBS(64) 恒成立，不会 panic；
    // 语义与原先的 max(1).min(MAX_JOBS) 完全一致。
    let jobs = cfg.jobs.clamp(1, MAX_JOBS);
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| loop {
                // 取消时不退出：排空队列，剩余文件由 execute_one 标记为 Cancelled（结果计数完整）
                let Some(plan) = lock_recover(&queue).pop_front() else {
                    break;
                };
                let r = execute_one(&plan, &cfg);
                on_result(&r);
                if let Some(mf) = mf_ref {
                    let (result, code) = match r.status {
                        Status::Ok => ("success", None),
                        Status::Skipped => ("skipped", None),
                        Status::Cancelled => ("cancelled", None),
                        Status::Failed => (
                            "failed",
                            r.reason
                                .as_deref()
                                .and_then(|s| s.split(": ").next())
                                .map(|c| c.trim().to_string()),
                        ),
                    };
                    let _ = mf.append(&manifest::ManifestItem {
                        task_id: mf.task_id().to_string(),
                        source: plan.source.display().to_string(),
                        target: Some(plan.target.display().to_string()),
                        actions: vec!["unpack", "write_tags"],
                        source_sha256: None,
                        target_sha256: sha256_of_sidecar(&plan.target),
                        result,
                        code,
                        rollback_available: false,
                        adapter: Some(plan.adapter),
                    });
                }
                lock_recover(&more).push(r);
            });
        }
    });
    results.append(&mut lock_recover(&more));

    results.sort_by(|a, b| a.source.cmp(&b.source));
    let ok = results.iter().filter(|r| r.status == Status::Ok).count();
    let skipped = results
        .iter()
        .filter(|r| r.status == Status::Skipped)
        .count();
    let cancelled = results
        .iter()
        .filter(|r| r.status == Status::Cancelled)
        .count();
    let failed = results
        .iter()
        .filter(|r| r.status == Status::Failed)
        .count();
    BatchSummary {
        results,
        planned: 0,
        ok,
        skipped,
        cancelled,
        failed,
        duration_ms: start.elapsed().as_millis(),
    }
}

/// 同目录临时文件名（rename 才能保持原子性；前缀固定便于清理残留）。
fn temp_path_for(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    target.with_file_name(format!(
        ".musicforge-tmp-{}-{nanos}-{name}",
        std::process::id()
    ))
}

/// 原子就位：Windows 的 rename 不能覆盖已存在目标，先删再 rename；失败清理临时文件。
fn atomic_rename(tmp: &Path, target: &Path) -> Result<(), NcmError> {
    if target.exists() {
        std::fs::remove_file(target)?;
    }
    match std::fs::rename(tmp, target) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(tmp);
            Err(NcmError::from(e))
        }
    }
}

/// 清理上次运行残留的临时文件（§4.7）；返回清理数量。
pub fn cleanup_stale_temps(out_dir: Option<&Path>) -> usize {
    let Some(dir) = out_dir else { return 0 };
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut n = 0;
    for entry in rd.flatten() {
        let p = entry.path();
        let is_tmp = p
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.starts_with(".musicforge-tmp-"))
            .unwrap_or(false);
        if is_tmp && std::fs::remove_file(&p).is_ok() {
            n += 1;
        }
    }
    n
}

/// 从 manifest 读出「已成功完成」的源文件集合（断点续跑用）。
///
/// 只认 `result == "success"` 且**产物与完整性标记都还在**的条目：
/// 标记缺失 = 产物可疑，必须重转（与 skip-existing 的完整性语义一致）。
pub fn completed_sources(manifest: &Path) -> HashSet<PathBuf> {
    let mut set: HashSet<PathBuf> = HashSet::new();
    let Ok(text) = std::fs::read_to_string(manifest) else {
        return set;
    };
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("result").and_then(|r| r.as_str()) != Some("success") {
            continue;
        }
        let Some(source) = v.get("source").and_then(|s| s.as_str()) else {
            continue;
        };
        let target = v
            .get("target")
            .and_then(|t| t.as_str())
            .map(PathBuf::from)
            .unwrap_or_default();
        let sidecar = PathBuf::from(format!("{}.musicforge.json", target.display()));
        if integrity_marker_ok(&target, &sidecar) {
            set.insert(PathBuf::from(source));
        }
    }
    set
}

/// 断点续跑：跳过 manifest 中已完成的文件，只处理剩余部分。
pub fn run_resume(
    cfg: BatchConfig,
    manifest: &Path,
    on_result: impl Fn(&FileResult) + Sync,
) -> BatchSummary {
    let completed = completed_sources(manifest);
    // 保留集：manifest 记录过的所有目标名（含 planned），防止新一轮抢占
    let reserved: HashSet<String> = std::fs::read_to_string(manifest)
        .map(|text| {
            text.lines()
                .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
                .filter_map(|v| {
                    v.get("target")
                        .and_then(|t| t.as_str())
                        .map(|t| dedup_key(Path::new(t)))
                })
                .collect()
        })
        .unwrap_or_default();
    let sources = collect_inputs(&cfg.inputs, cfg.recursive);
    let remaining: Vec<(PathBuf, Option<PathBuf>)> = sources
        .into_iter()
        .filter(|(p, _)| !completed.contains(p))
        .collect();
    run_inner(remaining, cfg, &on_result, reserved)
}

/// 从完整性侧车读取产物 sha256（best effort：缺失/损坏返回 None）。
pub fn sha256_of_sidecar(target: &Path) -> Option<String> {
    let sidecar = PathBuf::from(format!("{}.musicforge.json", target.display()));
    let text = std::fs::read_to_string(&sidecar).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v["sha256"].as_str().map(|s| s.to_string())
}

/// 兼容入口：无进度回调
pub fn run(cfg: BatchConfig) -> BatchSummary {
    run_with_progress(cfg, |_| {})
}
