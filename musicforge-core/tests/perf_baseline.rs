//! 性能专项基准（对照 ROADMAP §4.3 性能预算）。
//!
//! 不计入常规 `cargo test`（会拖慢套件）；仅当设置 `MF_PERF_N` 时运行：
//!
//! ```powershell
//! $env:MF_PERF_N = "10000"
//! cargo test --release --test perf_baseline -- --nocapture
//! ```
//!
//! 设计要点：
//! - 合成 fixture（非真实音频内容，仅体积/扩展名/命名以驱动分类与哈希 IO）；
//! - 首扫 = `scan_library`（纯元数据遍历+分类，§4.3「N 扫描」主体）；
//! - Plan = `build_clean_plan`（§4.3「Plan<5s」）；
//! - 哈希 = `refresh_hash_cache` 冷（全 miss→流式 sha256）/ 热（全命中→零读取，D17 L1）；
//! - 内存 = `PeakWorkingSetSize`（Windows，`windows-sys`），§4.3「哈希内存≤64MB」观测代理。
//!
//! 注：release 档 = `opt-level="z"`（体积最优，非速度最优），即用户实际分发的档位，
//! 故这是「真实可交付」基线，而非速度最优上限。

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::time::Instant;

use musicforge_core::db::Db;
use musicforge_core::scan::{
    build_clean_plan, refresh_hash_cache, scan_library, Category, ScanOptions, RULE_CARDS,
};
use tempfile::TempDir;

/// 采样进程峰值工作集（Windows）。§4.3「哈希内存≤64MB」的观测代理。
fn peak_ws_mb() -> Option<f64> {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    unsafe {
        let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        let ok = GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut pmc,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        );
        if ok != 0 {
            Some(pmc.PeakWorkingSetSize as f64 / (1024.0 * 1024.0))
        } else {
            None
        }
    }
}

/// 生成 N 个文件的合成曲库：2 级树（dirs 个子目录），混合类别，音频带 4KB 内容。
fn gen_fixture(n: usize) -> TempDir {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let dirs = (n / 500).clamp(1, 400) as u64; // 每目录 ~500 文件
    let per_dir = (n as u64).div_ceil(dirs);
    let mut made = 0u64;
    let audio_payload = [0xABu8; 4096]; // 4KB，足以让哈希做真实 IO
    for d in 0..dirs {
        if made >= n as u64 {
            break;
        }
        let sub = root.join(format!("d{d:04}"));
        std::fs::create_dir_all(&sub).unwrap();
        for _i in 0..per_dir {
            if made >= n as u64 {
                break;
            }
            let idx = made;
            // 90% 音频，其余 junk/lyrics/cover/other
            let (name, payload): (String, &[u8]) = match idx % 10 {
                0 => (format!("note_{idx}.txt"), &b""[..]),
                1 => (format!("junk_{idx}.tmp"), &b""[..]),
                2 => (format!("thumbs_{idx}.db"), &b""[..]),
                3 => (format!("track_{idx}.lrc"), &b""[..]),
                4 => (format!("cover_{idx}.jpg"), &[0xFFu8; 256][..]),
                _ => (format!("track_{idx}.flac"), &audio_payload[..]),
            };
            let p = sub.join(name);
            let mut f = std::fs::File::create(&p).unwrap();
            f.write_all(payload).unwrap();
            made += 1;
        }
    }
    tmp
}

#[test]
fn perf_baseline() {
    let n: usize = match std::env::var("MF_PERF_N").ok().and_then(|v| v.parse().ok()) {
        Some(v) if v > 0 => v,
        _ => {
            eprintln!(
                "[perf_baseline] 跳过：未设置 MF_PERF_N（如 10000）。常规测试套件不跑此基准。"
            );
            return;
        }
    };

    println!("\n===== MusicForge 性能基线 (N={n}) =====");
    println!("profile: release (opt-level=\"z\", lto, panic=abort) — 即实际分发档");
    println!(
        "机器: {} 逻辑核",
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0)
    );

    // --- 生成 fixture ---
    let t0 = Instant::now();
    let tmp = gen_fixture(n);
    let gen_ms = t0.elapsed().as_secs_f64() * 1000.0;
    println!(
        "\n[fixture] 生成 {n} 文件耗时 {gen_ms:.1} ms @ {}",
        tmp.path().display()
    );

    let jobs = if ScanOptions::default().parallel_jobs == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .min(8)
    } else {
        ScanOptions::default().parallel_jobs
    };
    let opts = ScanOptions::default();
    println!("[scan] parallel_jobs=auto→{jobs}");

    // --- 首扫（§4.3「N 扫描」）---
    let mem_before = peak_ws_mb();
    let t = Instant::now();
    let report = scan_library(tmp.path(), &opts).expect("scan");
    let scan_s = t.elapsed().as_secs_f64();
    let mem_after_scan = peak_ws_mb();
    let audio = report.audio;
    let junk = report.junk;
    let other = report.other;
    println!("\n[scan] 首扫 {n} 文件 = {scan_s:.3} s  (预算: 10k<10s / 100k<120s)",);
    println!(
        "       分类: audio={audio} junk={junk} other={other} | 扫描目录={} 文件={}",
        report.scanned_dirs, report.scanned_files
    );
    println!(
        "       内存: before={:.1?}MB after_scan={:.1?}MB",
        mem_before, mem_after_scan
    );

    // --- Plan（§4.3「Plan<5s」）---
    let enabled: HashSet<&'static str> = RULE_CARDS.iter().map(|c| c.id).collect();
    let t = Instant::now();
    let plan = build_clean_plan(&report, &enabled, tmp.path(), tmp.path());
    let plan_ms = t.elapsed().as_secs_f64() * 1000.0;
    println!(
        "\n[plan] build_clean_plan 动作数={} = {plan_ms:.1} ms  (预算: <5000 ms)",
        plan.actions.len()
    );

    // --- 哈希：冷（全 miss→流式 sha256）/ 热（全命中，D17 L1）---
    let db = Db::open_in_memory().expect("db");
    let audio_items: usize = report
        .items
        .iter()
        .filter(|i| i.category == Category::Audio)
        .count();
    let bytes_total = audio_items as u64 * 4096; // 合成音频单文件 4KB

    let t = Instant::now();
    let cold = refresh_hash_cache(&db, &report.items);
    let cold_s = t.elapsed().as_secs_f64();
    let cold_mb_s = bytes_total as f64 / (1024.0 * 1024.0) / cold_s.max(1e-9);
    let mem_after_cold = peak_ws_mb();
    println!(
        "\n[hash-cold] 流式 sha256（{audio_items} 音频, {:.1} MB） = {cold_s:.3} s  @ {cold_mb_s:.1} MB/s",
        bytes_total as f64 / (1024.0 * 1024.0)
    );
    println!(
        "             hashed={} cache_hits={} skipped={}",
        cold.hashed, cold.cache_hits, cold.skipped
    );

    let t = Instant::now();
    let warm = refresh_hash_cache(&db, &report.items);
    let warm_ms = t.elapsed().as_secs_f64() * 1000.0;
    let mem_after_warm = peak_ws_mb();
    println!("\n[hash-warm] 增量命中（D17 L1，零文件读取） = {warm_ms:.1} ms",);
    println!(
        "             hashed={} cache_hits={} skipped={}",
        warm.hashed, warm.cache_hits, warm.skipped
    );

    println!(
        "\n[memory] PeakWorkingSet 观测: scan={:.1?}MB cold={:.1?}MB warm={:.1?}MB  (预算: ≤64MB)",
        mem_after_scan, mem_after_cold, mem_after_warm
    );

    // --- 结论摘要 ---
    let scan_budget = if n <= 10_000 { 10.0 } else { 120.0 };
    println!(
        "\n===== 小结 =====\n  首扫: {scan_s:.3}s / 预算 {scan_budget}s -> {}\n  Plan: {plan_ms:.1}ms / 预算 5000ms -> {}\n  哈希内存: {:.1?}MB / 预算 64MB -> {}",
        if scan_s <= scan_budget { "PASS" } else { "FAIL" },
        if plan_ms <= 5000.0 { "PASS" } else { "FAIL" },
        mem_after_warm,
        if mem_after_warm.unwrap_or(999.0) <= 64.0 { "PASS" } else { "FAIL" },
    );
    println!("  (哈希冷跑非预算项；首次全量哈希依赖 D17 缓存，二次扫描方入 §4.3 预算)");

    let _ = Path::new(""); // 抑制未用导入告警
    drop(report);
    drop(plan);
    drop(db);
    drop(tmp);
}
