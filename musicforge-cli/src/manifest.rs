//! 操作清单（Manifest）——v0.2.0 安全任务层的可审计留痕。
//!
//! 设计要点（对齐 ROADMAP §5 P2 与治理 §4.2/§4.7）：
//!
//! - **格式**：NDJSON（`.jsonl`）——首行是任务头，其后每行一条 item。
//!   逐行追加既便于崩溃后断点续跑，也便于外部工具流式消费。
//! - **可移植**：manifest 是**纯文件**归档，与状态库（db，可再生缓存）双写；
//!   db 丢失不影响 manifest，manifest 是「发生了什么」的可信留痕。
//! - **版本化**：任务头带 `schema_version`（当前 1），后续演进必须向后兼容
//!   或提供迁移（错误码同理，见 `docs/result-codes.md`）。
//!
//! 说明：本模块刻意**不引入 serde derive**（cli 只依赖 serde_json），
//! 用 `serde_json::json!` 构造，保持依赖面最小（依赖政策 §4.1）。

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::NcmError;

/// 当前 manifest schema 版本。
pub const SCHEMA_VERSION: u32 = 1;

/// 默认 manifest 目录（相对输出根）。
pub const MANIFEST_DIR: &str = ".musicforge/manifests";

/// 一条操作记录。
#[derive(Debug)]
pub struct ManifestItem {
    pub task_id: String,
    pub source: String,
    pub target: Option<String>,
    pub actions: Vec<&'static str>,
    pub source_sha256: Option<String>,
    pub target_sha256: Option<String>,
    pub result: &'static str,
    /// 稳定错误码（`MF-*`；成功为 None）
    pub code: Option<String>,
    /// 是否可回滚；当前未实现 undo，固定 false（P2 回收站/回滚落地后启用）
    pub rollback_available: bool,
    pub adapter: Option<&'static str>,
}

impl ManifestItem {
    fn to_value(&self) -> serde_json::Value {
        serde_json::json!({
            "task_id": self.task_id,
            "source": self.source,
            "target": self.target,
            "actions": self.actions,
            "source_sha256": self.source_sha256,
            "target_sha256": self.target_sha256,
            "result": self.result,
            "code": self.code,
            "rollback_available": self.rollback_available,
            "adapter": self.adapter,
        })
    }
}

/// manifest 写入器（内部加锁，可在并行 worker 中共享）。
pub struct Manifest {
    writer: std::sync::Mutex<BufWriter<File>>,
    task_id: String,
}

impl Manifest {
    /// 创建/追加打开 manifest，并写入任务头。父目录不存在时自动创建。
    pub fn open(path: &Path, task_id: &str, command: &'static str) -> Result<Self, NcmError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let mut writer = BufWriter::new(file);

        let header = serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "app_version": env!("CARGO_PKG_VERSION"),
            "task_id": task_id,
            "created_at": now_rfc3339(),
            "command": command,
        });
        writeln!(writer, "{}", serde_json::to_string(&header)?)?;
        writer.flush()?;

        Ok(Self {
            writer: std::sync::Mutex::new(writer),
            task_id: task_id.to_string(),
        })
    }

    /// 追加一条记录并 flush（保证崩溃时已完成的条目不丢）。
    pub fn append(&self, item: &ManifestItem) -> Result<(), NcmError> {
        let mut writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        writeln!(writer, "{}", serde_json::to_string(&item.to_value())?)?;
        writer.flush()?;
        Ok(())
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }
}

/// 任务 id：`<YYYYMMDD>-<HHMMSS>-<pid>`，便于人类排序与定位。
pub fn new_task_id() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}-{}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60,
        std::process::id()
    )
}

/// 默认 manifest 路径：`<out_dir>/.musicforge/manifests/<task_id>.jsonl`
pub fn default_manifest_path(out_dir: Option<&Path>, task_id: &str) -> PathBuf {
    let base = out_dir.unwrap_or(Path::new("."));
    base.join(MANIFEST_DIR).join(format!("{task_id}.jsonl"))
}

fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant 的 days→civil date 算法（避免引入 chrono）。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
