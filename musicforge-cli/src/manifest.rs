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

use crate::{NcmError, Suggestion};

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
    /// X13/X35：用户确认后并入本条操作的建议；None = 无建议（行内**不出现**该键，
    /// 与 v0.2.0–v0.6.0 历史行字节兼容；schema_version 不 bump）
    pub suggestion: Option<Suggestion>,
}

impl ManifestItem {
    fn to_value(&self) -> serde_json::Value {
        let mut v = serde_json::json!({
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
        });
        // X35：NDJSON 行级可选键——仅在携带建议时出现（旧读取方忽略未知键）
        if let Some(s) = &self.suggestion {
            v["suggestion"] = s.to_value();
        }
        v
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

/// 默认 manifest 路径：`<out_dir>/.musicforge/manifests/<task_id>.jsonl`。
/// `out_dir` 为 None 时落到**本地配置目录**（绝不落到进程 CWD，避免杂散目录）。
pub fn default_manifest_path(out_dir: Option<&Path>, task_id: &str) -> PathBuf {
    let base = match out_dir {
        Some(d) => d.to_path_buf(),
        None => musicforge_core::db::local_config_dir(),
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_item(suggestion: Option<Suggestion>) -> ManifestItem {
        ManifestItem {
            task_id: "t-1".into(),
            source: "src/a.ncm".into(),
            target: Some("out/a.flac".into()),
            actions: vec!["unpack", "write_tags"],
            source_sha256: None,
            target_sha256: None,
            result: "ok",
            code: None,
            rollback_available: false,
            adapter: Some("ncm"),
            suggestion,
        }
    }

    fn sample_suggestion() -> Suggestion {
        Suggestion {
            provider: "ai-openai-compatible".into(),
            method: "ai.identify_track".into(),
            confidence: 0.93,
            fields: serde_json::json!({"title": "借墨", "artists": ["王铮亮", "风华音纪"]}),
            reason: "文件名与内嵌标签一致".into(),
        }
    }

    /// X35 兼容规则 1：无建议的行**不出现** `suggestion` 键（与 v0.2.0–v0.6.0 字节兼容）。
    #[test]
    fn row_without_suggestion_has_no_key() {
        let line = serde_json::to_string(&sample_item(None).to_value()).unwrap();
        assert!(
            !line.contains("suggestion"),
            "无建议行不得含 suggestion 键: {line}"
        );
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert!(v.get("suggestion").is_none());
        // 旧读取方路径：既有键全部可读
        assert_eq!(v["result"], "ok");
        assert_eq!(v["adapter"], "ncm");
    }

    /// X35 兼容规则 2：携带建议的行新增 `suggestion` 可选键，schema_version 不 bump。
    #[test]
    fn row_with_suggestion_is_backward_compatible_extension() {
        let line =
            serde_json::to_string(&sample_item(Some(sample_suggestion())).to_value()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        let s = &v["suggestion"];
        assert_eq!(s["provider"], "ai-openai-compatible");
        assert_eq!(s["method"], "ai.identify_track");
        assert!((s["confidence"].as_f64().unwrap() - 0.93).abs() < 1e-6);
        assert_eq!(s["fields"]["title"], "借墨");
        assert_eq!(s["fields"]["artists"].as_array().unwrap().len(), 2);
        // 旧读取方忽略未知键：既有键不受影响
        assert_eq!(v["result"], "ok");
        assert_eq!(v["task_id"], "t-1");
    }

    /// manifest 全链路：写盘 → 逐行读回，头部 schema_version 恒为 1（X35 不 bump）。
    #[test]
    fn manifest_roundtrip_keeps_schema_version_1() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.jsonl");
        let mf = Manifest::open(&path, "task-x", "convert").unwrap();
        mf.append(&sample_item(None)).unwrap();
        mf.append(&sample_item(Some(sample_suggestion()))).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let mut lines = text.lines();
        let header: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!(header["schema_version"], 1);
        assert_eq!(header["command"], "convert");

        let row1: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert!(
            row1.get("suggestion").is_none(),
            "历史形状行不得出现 suggestion 键"
        );
        let row2: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!(row2["suggestion"]["provider"], "ai-openai-compatible");
        assert!(lines.next().is_none(), "恰为 1 头 + 2 行");
    }
}
