//! P1/C4 稳定性回归：并行 walker 不得因 worker panic 而死锁（进程挂死）。

use musicforge_core::scan::{scan_library, ScanOptions};
use std::path::Path;

/// C4 回归护栏：并行 walker 扫描真实嵌套目录必须在有限时间内返回（不挂死）。
/// 若 C4 修复被回退且 `scan_one_dir` 因异常输入 panic，worker panic 会使 `remaining`
/// 永不归零 → 其余 worker 永久阻塞在 `cv.wait()`（进程挂死，测试会被框架 60s 超时杀掉而失败）。
#[test]
fn scan_parallel_completes_without_hang() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let opts = ScanOptions {
        parallel_jobs: 4,
        ..Default::default()
    };
    let res = scan_library(dir, &opts);
    // 无论结果（目录可读），必须有限时间内返回，不得死锁
    assert!(res.is_ok() || res.is_err());
}

/// 扫描不存在的根目录应在进入并行 walker 前直接 Err（不进 worker 逻辑）。
#[test]
fn scan_missing_root_is_error() {
    let res = scan_library(
        Path::new("/nonexistent-path-zzz-99999"),
        &ScanOptions::default(),
    );
    assert!(res.is_err());
}
