//! musicforge CLI 入口（薄壳：参数解析 + 汇总输出 + 退出码；批处理逻辑在 lib.rs）

use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use musicforge_cli::{run, run_resume, BatchConfig};

/// MusicForge — 默认离线、可靠、可观测的本地 ncm 转换器
///
/// 仅处理你已合法获得的文件的个人本地格式转换。零网络、不上传、不收集数据。
#[derive(Parser, Debug)]
#[command(
    name = "musicforge",
    version,
    about,
    after_help = "法律须知：本项目仅支持处理你已合法获得的文件的个人本地格式转换/备份。"
)]
struct Args {
    /// .ncm 文件（可多个）
    files: Vec<String>,

    /// 处理目录（可与 -r 同用）
    #[arg(short = 'd', long)]
    directory: Option<String>,

    /// 递归处理目录（保留目录结构）
    #[arg(short = 'r', long, requires = "directory")]
    recursive: bool,

    /// 输出根目录（默认输出到源文件同目录）
    #[arg(short = 'o', long)]
    output: Option<String>,

    /// 跳过已存在且大小完整的输出文件
    #[arg(long)]
    skip_existing: bool,

    /// 并发数（默认 4）
    #[arg(short = 'j', long, default_value_t = 4)]
    jobs: usize,

    /// 导出失败清单 CSV 到指定路径
    #[arg(long)]
    export_failures: Option<String>,

    /// 命名模板（占位符 {title}/{artist}/{album}/{track}/{track:02d}/{format}；/ 产生子目录）
    #[arg(long, default_value = "{artist} - {title}")]
    template: String,

    /// 只规划不落盘：产出 manifest 计划条目，不写任何音频与侧车文件（v0.2.0）
    #[arg(long)]
    dry_run: bool,

    /// manifest 路径；不指定则不写留痕文件（dry-run 建议指定以便查看计划）
    #[arg(long)]
    manifest: Option<String>,

    /// 断点续跑：跳过该 manifest 中已成功完成的文件（配合 --manifest 使用）
    #[arg(long)]
    resume: Option<String>,
}

fn main() {
    let args = Args::parse();
    let start = Instant::now();

    let mut inputs: Vec<PathBuf> = args.files.iter().map(PathBuf::from).collect();
    if let Some(d) = &args.directory {
        inputs.push(PathBuf::from(d));
    }
    if inputs.is_empty() {
        eprintln!("错误：未提供任何输入文件或目录（-h 查看用法）");
        // Windows 双击闪退修复：非管道模式下等待按键，让用户看清错误信息
        use std::io::IsTerminal;
        if std::io::stdin().is_terminal() {
            println!("\n按 Enter 键退出...");
            let mut buf = String::new();
            let _ = std::io::stdin().read_line(&mut buf);
        }
        std::process::exit(2);
    }

    // `collect_inputs` 对「不存在 / 不可读 / 非 .ncm」的输入是静默跳过的
    // （签名不含错误通道，改动会波及 GUI 的 collect_files 命令，故不在此处改契约）。
    // 后果：路径打错时表现为「汇总：成功 0」+ 退出码 0，用户以为跑完了。
    // 这里补一道可观测性兜底——只加 stderr 提示，不改退出码语义（QA 第二轮）。
    for p in &inputs {
        if !p.exists() {
            eprintln!("警告：输入路径不存在，已跳过：{}", p.display());
        }
    }
    let cfg = BatchConfig {
        inputs,
        out_dir: args.output.as_ref().map(PathBuf::from),
        recursive: args.recursive,
        skip_existing: args.skip_existing,
        jobs: args.jobs,
        template: args.template,
        cancel: None,
        dry_run: args.dry_run,
        // 默认不写 manifest：避免在任何工作目录产生 .musicforge 杂散目录；
        // 需要审计留痕时显式 --manifest <path>
        manifest: args.manifest.as_ref().map(PathBuf::from),
    };

    let summary = match args.resume.as_ref() {
        Some(manifest) => run_resume(cfg, Path::new(manifest), |_| {}),
        None => run(cfg),
    };

    // 同上：零结果 + 非零输入 = 输入没匹配到任何 .ncm。
    // 不报错就会被当成「成功转换 0 个」，是典型的静默失败。
    if summary.results.is_empty() {
        eprintln!(
            "未发现任何 .ncm 文件：请检查输入路径是否正确、目录是否可读、文件扩展名是否为 .ncm"
        );
    }

    // 逐文件结果（repair receipt：失败带错误码 + 建议）
    // 注意：Ok 结果里仅「TagRead 降级」这一项会带 reason（见 lib.rs::execute_one），
    // 其余 Ok 的 reason 恒为 None。故此处打印 reason 等价于打印降级告警，互不干扰。
    let mut degraded = 0usize;
    for r in &summary.results {
        match r.status {
            musicforge_cli::Status::Ok => {
                println!(
                    "✓ {} → {}",
                    r.source.display(),
                    r.output
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                );
                if let Some(reason) = &r.reason {
                    degraded += 1;
                    eprintln!("⚠ {} —— {}", r.source.display(), reason);
                }
            }
            musicforge_cli::Status::Skipped => {
                println!("⏭ {} （已存在，跳过）", r.source.display());
            }
            musicforge_cli::Status::Cancelled => {
                println!("⏸ {} （已取消）", r.source.display());
            }
            musicforge_cli::Status::Failed => {
                if let Some(reason) = &r.reason {
                    eprintln!("✗ {} —— {}", r.source.display(), reason);
                }
            }
        }
    }

    let cancel_note = if summary.is_cancelled() {
        "（已取消）"
    } else {
        ""
    };
    let degraded_note = if degraded > 0 {
        format!("（其中 {degraded} 个音频已完整导出但元数据未写入，下次运行将自动重转）")
    } else {
        String::new()
    };
    println!(
        "\n汇总：成功 {} / 跳过 {} / 失败 {} · 耗时 {} ms{}{}",
        summary.ok,
        summary.skipped,
        summary.failed,
        summary.duration_ms,
        cancel_note,
        degraded_note
    );

    if let Some(csv) = &args.export_failures {
        let csv_path = PathBuf::from(csv);
        // 父目录不存在时自动创建，避免裸 os error 3 —— 用户拿不到有用信息
        if let Some(parent) = csv_path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    eprintln!("失败清单目录创建失败（{}）：{e}", parent.display());
                    std::process::exit(1);
                }
            }
        }
        match summary.export_failures_csv(csv_path.as_path()) {
            Ok(()) => println!("失败清单已导出：{csv}"),
            Err(e) => eprintln!("失败清单导出失败：{e}"),
        }
    }

    let _ = start; // duration 已在 summary 内
    std::process::exit(summary.exit_code());
}
