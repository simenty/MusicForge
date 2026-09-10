//! X25：并行 walker 等价性与确定性测试。
//!
//! 核心不变量：**任意并行度下，扫描结果（items 集合/计数/规则命中/空目录/
//! 未授权清单）与单线程逐字节等价**；且并行输出顺序确定性（items 按路径排序）。

use musicforge_core::scan::{scan_library, ScanOptions};

/// 构造多目录多文件测试树：3 层 × 每层多目录 × 混合文件（音频/歌词/垃圾/其他）。
fn build_tree(root: &std::path::Path, dirs: usize, files_per_dir: usize) {
    for d in 0..dirs {
        let dir = root.join(format!("lv1_{d:02}")).join("nested");
        std::fs::create_dir_all(&dir).unwrap();
        for f in 0..files_per_dir {
            std::fs::write(dir.join(format!("song_{f:02}.flac")), b"fLaC-stub").unwrap();
            std::fs::write(dir.join(format!("song_{f:02}.lrc")), b"[00:00] x").unwrap();
            std::fs::write(dir.join(format!("junk_{f}.tmp")), b"").unwrap();
            std::fs::write(dir.join(format!("note_{f}.txt")), b"other").unwrap();
        }
        // 一层散目录（空目录与孤立歌词用例）
        std::fs::create_dir_all(root.join(format!("scattered_{d:02}"))).unwrap();
        std::fs::write(
            root.join(format!("scattered_{d:02}").to_string() + "/orphan.lrc"),
            b"",
        )
        .unwrap();
    }
    // 空目录
    std::fs::create_dir_all(root.join("empty-dir")).unwrap();
    // 垃圾文件（根层）
    std::fs::write(root.join("Thumbs.db"), b"").unwrap();
}

fn opts(jobs: usize) -> ScanOptions {
    ScanOptions {
        parallel_jobs: jobs,
        ..ScanOptions::default()
    }
}

/// 等价性：并行度 1 / 4 / 8 / 自动(0) 的扫描结果全等（items 集合/计数/规则命中/空目录）。
#[test]
fn x25_parallel_results_equal_serial() {
    let root = tempfile::tempdir().unwrap();
    build_tree(root.path(), 6, 4);

    let serial = scan_library(root.path(), &opts(1)).unwrap();
    for jobs in [4usize, 8, 0] {
        let par = scan_library(root.path(), &opts(jobs)).unwrap();
        // items 集合（排序后逐项等价）
        assert_eq!(
            serial.items.len(),
            par.items.len(),
            "jobs={jobs}: items 数量不一致"
        );
        let s: Vec<_> = serial.items.iter().map(|i| i.path.clone()).collect();
        let mut p: Vec<_> = par.items.iter().map(|i| i.path.clone()).collect();
        p.sort();
        let mut s_sorted = s.clone();
        s_sorted.sort();
        assert_eq!(s_sorted, p, "jobs={jobs}: items 路径集合不一致");
        for (a, b) in serial.items.iter().zip(
            {
                let mut v = par.items.clone();
                v.sort_by(|x, y| x.path.cmp(&y.path));
                v
            }
            .iter(),
        ) {
            assert_eq!(a.path, b.path);
            assert_eq!(a.category, b.category, "分类漂移: {}", a.path.display());
            assert_eq!(a.rule_id, b.rule_id, "规则命中漂移: {}", a.path.display());
        }
        // 计数
        assert_eq!(serial.audio, par.audio, "jobs={jobs}: audio 计数");
        assert_eq!(serial.junk, par.junk, "jobs={jobs}: junk 计数");
        assert_eq!(
            serial.scanned_files, par.scanned_files,
            "jobs={jobs}: files"
        );
        assert_eq!(serial.scanned_dirs, par.scanned_dirs, "jobs={jobs}: dirs");
        // 规则命中
        assert_eq!(serial.rule_hits, par.rule_hits, "jobs={jobs}: rule_hits");
        // 空目录集合
        assert_eq!(
            serial.empty_dirs.len(),
            par.empty_dirs.len(),
            "jobs={jobs}: 空目录数"
        );
        assert_eq!(
            serial.unauthorized_dirs.len(),
            par.unauthorized_dirs.len(),
            "jobs={jobs}: 未授权数"
        );
    }
}

/// 确定性：同一并行度连续两次扫描，items 顺序逐项一致（排序保证跨运行稳定）。
#[test]
fn x25_parallel_output_is_deterministic() {
    let root = tempfile::tempdir().unwrap();
    build_tree(root.path(), 8, 5);
    let a = scan_library(root.path(), &opts(8)).unwrap();
    let b = scan_library(root.path(), &opts(8)).unwrap();
    let pa: Vec<_> = a.items.iter().map(|i| i.path.clone()).collect();
    let pb: Vec<_> = b.items.iter().map(|i| i.path.clone()).collect();
    assert_eq!(pa, pb, "并行扫描输出顺序必须确定性");
}

/// 语义保持：约定目录 `.musicforge` 剪枝在并行下同样生效。
#[test]
fn x25_musicforge_dir_pruned_under_parallelism() {
    let root = tempfile::tempdir().unwrap();
    build_tree(root.path(), 2, 2);
    let inner = root.path().join("lv1_00/.musicforge");
    std::fs::create_dir_all(&inner).unwrap();
    std::fs::write(inner.join("recycled.flac"), b"fLaC-stub").unwrap();
    let r = scan_library(root.path(), &opts(8)).unwrap();
    assert!(
        !r.items.iter().any(|i| i.path.starts_with(&inner)),
        "并行下 .musicforge 必须剪枝"
    );
}

/// 语义保持：max_depth 截断在并行下同样生效（深度 0 = 只扫根层）。
#[test]
fn x25_max_depth_respected_under_parallelism() {
    let root = tempfile::tempdir().unwrap();
    build_tree(root.path(), 3, 2);
    let o = ScanOptions {
        max_depth: 0,
        parallel_jobs: 8,
        ..ScanOptions::default()
    };
    let r = scan_library(root.path(), &o).unwrap();
    assert!(
        !r.items
            .iter()
            .any(|i| i.path.components().count() > root.path().components().count() + 1),
        "max_depth=0 时不得进入子目录"
    );
}
