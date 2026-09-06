//! v0.2.0 安全任务层（切片二）：崩溃安全与断点续跑（§4.7）。
//!
//! - 产物与完整性标记均**原子就位**（临时文件 → rename），输出目录不留半成品；
//! - 上次中断残留的临时文件在下次运行开始时被清理；
//! - `run_resume` 依据 manifest 跳过已成功且完整性标记仍在的文件，只补做剩余部分。

use std::path::{Path, PathBuf};

use musicforge_cli::{run, run_resume, BatchConfig};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../musicforge-core/tests/fixtures")
}

fn ncm_files() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(fixtures())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().map(|e| e == "ncm").unwrap_or(false))
        .collect();
    v.sort();
    v
}

fn cfg(inputs: Vec<PathBuf>, out: &Path, manifest: Option<PathBuf>) -> BatchConfig {
    BatchConfig {
        inputs,
        out_dir: Some(out.to_path_buf()),
        recursive: false,
        skip_existing: false,
        jobs: 2,
        template: "{title}".to_string(),
        cancel: None,
        dry_run: false,
        manifest,
    }
}

fn count_tmp(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with(".musicforge-tmp-")
                })
                .count()
        })
        .unwrap_or(0)
}

/// 原子落盘：正常转换不留下任何临时文件；上次中断的残留也会被清理。
#[test]
fn atomic_write_leaves_no_temp_files() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    // 伪造上次中断留下的半成品
    let stale = out.join(".musicforge-tmp-1234-half.flac");
    std::fs::write(&stale, b"half written").unwrap();

    let summary = run(cfg(ncm_files(), &out, None));
    assert_eq!(summary.ok, 7, "7 个 fixture 全部成功");

    assert!(!stale.exists(), "上次中断的临时文件必须被清理");
    assert_eq!(count_tmp(&out), 0, "输出目录不得残留任何临时文件");
}

/// 断点续跑：已完成的 2 个不再重做，产物保持原样；只补做剩余 5 个。
#[test]
fn resume_only_processes_remaining() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let manifest = tmp.path().join("run.jsonl");
    let all = ncm_files();

    // 第一次只做前 2 个
    let first = run(cfg(all[..2].to_vec(), &out, Some(manifest.clone())));
    assert_eq!(first.ok, 2);

    let done: Vec<PathBuf> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| !p.to_string_lossy().contains(".musicforge.json"))
        .collect();
    assert_eq!(done.len(), 2);
    let before: Vec<(PathBuf, u64)> = done
        .iter()
        .map(|p| (p.clone(), std::fs::metadata(p).unwrap().len()))
        .collect();

    // 断点续跑：输入仍给全量，应只处理剩余 5 个
    let resumed = run_resume(
        cfg(all.clone(), &out, Some(manifest.clone())),
        &manifest,
        |_| {},
    );
    assert_eq!(resumed.ok, 5, "只应补做剩余 5 个");
    assert_eq!(resumed.failed, 0);

    // 已完成的产物未被改动（大小一致）
    for (p, size) in before {
        assert!(
            std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) == size,
            "已完成产物不应被覆盖重转: {p:?}"
        );
    }
}

/// 全部完成时续跑是空操作：不报错、退出码 0。
#[test]
fn resume_when_all_completed_is_noop() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let manifest = tmp.path().join("run.jsonl");
    let all = ncm_files();

    run(cfg(all.clone(), &out, Some(manifest.clone())));
    let resumed = run_resume(cfg(all, &out, Some(manifest.clone())), &manifest, |_| {});
    assert_eq!(resumed.ok, 0, "全部完成后续跑不应再产出");
    assert_eq!(resumed.exit_code(), 0);
}
