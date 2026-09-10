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
    after_help = "法律须知：本项目仅支持处理你已合法获得的文件的个人本地格式转换/备份。",
    subcommand_negates_reqs = true,
    args_conflicts_with_subcommands = true
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

    /// 状态库路径（可再生缓存：文件索引/哈希缓存/任务历史）；必须是本地目录
    #[arg(long)]
    state_db: Option<String>,

    #[command(subcommand)]
    command: Option<Sub>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Sub {
    /// D13 watcher：监听目录新文件（T0 登记 / T1 自动整理 / T2 整理+垃圾进回收站）
    Watch {
        /// 自动化等级：t0=只登记（默认）/ t1=自动整理 / t2=整理+垃圾自动清洗（只进回收站）
        #[arg(long, default_value = "t0")]
        level: String,
        /// 整理目标根目录（t1/t2 必填）
        #[arg(long)]
        target: Option<String>,
        /// 整理命名模板
        #[arg(long, default_value = "{title} - {artist}")]
        template: String,
        /// 防抖窗口毫秒（同路径事件稳定后才处理）
        #[arg(long, default_value_t = 1500)]
        debounce_ms: u64,
        /// T2 自动清洗白名单（可多次；缺省 = 不自动清洗——自动化破坏面需显式授权）
        #[arg(long = "whitelist", value_name = "DIR")]
        whitelist: Vec<String>,
        /// 要监听的目录
        #[arg(value_name = "DIR")]
        dir: String,
    },
    /// 只读扫描：分类文件并报告垃圾/异常项（不改动任何文件）
    Scan {
        /// 要扫描的目录
        #[arg(value_name = "DIR")]
        dir: String,
        /// 递归子目录（默认开）
        #[arg(short = 'r', long, default_value_t = true)]
        recursive: bool,
        /// JSON 输出（机器可读，供 GUI/脚本消费）
        #[arg(long)]
        json: bool,
        /// 状态库：把音频文件索引写入 db（可再生缓存）
        #[arg(long)]
        state_db: Option<String>,
    },
    /// 清洗：把命中的垃圾/异常项移入回收站（默认 dry-run，--apply 才执行）
    Clean {
        /// 要清洗的目录（--restore 模式下不需要）
        #[arg(value_name = "DIR")]
        dir: Option<String>,
        /// 只启用指定规则（逗号分隔，如 MF-CLEAN-001,MF-CLEAN-003）；缺省=全部
        #[arg(long)]
        rules: Option<String>,
        /// 真正执行（缺省=只规划）
        #[arg(long)]
        apply: bool,
        /// 回收站根目录（默认 <DIR>/.musicforge/trash）
        #[arg(long)]
        trash: Option<String>,
        /// 状态库：任务历史与 ack 留痕
        #[arg(long)]
        state_db: Option<String>,
        /// 从回滚清单还原（值 = rollback.jsonl 路径）
        #[arg(long)]
        restore: Option<String>,
    },
    /// 去重：内容重复分组 + 可解释保留评分（默认 dry-run，--apply 移牺牲项入回收站）
    Dedupe {
        /// 要去重的目录
        #[arg(value_name = "DIR")]
        dir: String,
        /// 状态库（D17 哈希缓存：命中免重算；大库强烈建议提供）
        #[arg(long)]
        state_db: Option<String>,
        /// 真正执行（缺省=只规划；牺牲项全部进回收站，可 restore）
        #[arg(long)]
        apply: bool,
        /// 回收站根目录（默认 <DIR>/.musicforge/trash）
        #[arg(long)]
        trash: Option<String>,
        /// 关闭同名候选检测（默认开；同名组默认仅报告，不参与 apply）
        #[arg(long)]
        no_same_name: bool,
        /// 把同名候选组的非保留成员也纳入 apply 范围（同名≠同歌，慎用）
        #[arg(long, requires = "apply")]
        include_same_name: bool,
        /// AI 保留建议（v0.7.0 起提供；当前离线版显式报 MF-PLUGIN-NOT-FOUND）
        #[arg(long, conflicts_with = "apply")]
        suggest: bool,
        /// 附带相似封面分组报告（aHash，仅报告不执行——换封面属 v0.7.0 能力）
        #[arg(long)]
        covers: bool,
        /// JSON 输出（机器可读）
        #[arg(long)]
        json: bool,
    },
    /// 整理：按命名模板把音频文件归位到规范目录结构（默认 dry-run，--apply 才移动）
    Organize {
        /// 要整理的曲库目录
        #[arg(value_name = "DIR")]
        dir: String,
        /// 目标根目录（缺省=原地整理）
        #[arg(long)]
        to: Option<String>,
        /// 命名模板（与 convert 同源渲染语义）
        #[arg(long, default_value = "{artist} - {title}")]
        template: String,
        /// 冲突策略（目标已存在时）：skip=报告跳过 / suffix=追加 (2) / overwrite-never=计失败
        #[arg(long, default_value = "skip")]
        conflict: String,
        /// 真正执行（缺省=只规划；移动可经回滚清单整体还原）
        #[arg(long)]
        apply: bool,
        /// JSON 输出（机器可读）
        #[arg(long)]
        json: bool,
    },
    /// 播放清单：M3U8 按分类导出 / 导入失效路径修复（只写清单文件，不动音乐）
    Playlist {
        #[command(subcommand)]
        cmd: PlaylistCmd,
    },
    /// 转码：无损（WAV↔FLAC，纯 Rust 回读校验）+ 有损导出（MP3 320/AAC 256/Opus 160，FFmpeg sidecar）
    Transcode {
        /// 输入：文件或目录（目录递归收集音频，按魔数/扩展名识别）
        #[arg(value_name = "INPUT")]
        input: Vec<String>,
        /// 输出目录（产物写入这里，绝不修改源）
        #[arg(short = 'o', long)]
        out: String,
        /// 目标格式（mp3=320k / aac=256k / opus=160k）
        #[arg(long, value_parser = ["flac", "wav", "mp3", "aac", "opus"], default_value = "flac")]
        format: String,
        /// ffmpeg 可执行文件路径（五级探测的第 0 级；有损导出必需）
        #[arg(long)]
        ffmpeg_path: Option<String>,
        /// 显式放行有损→无损的伪升级转换（默认拦截）
        #[arg(long)]
        i_know_lossy_to_lossless: bool,
        /// JSON 输出（机器可读）
        #[arg(long)]
        json: bool,
    },
    /// 插件管理：状态 / 启用禁用 / 高风险确认（ACK 闸；config.json 持久化）
    Plugins {
        #[command(subcommand)]
        cmd: PluginsCmd,
    },
    /// 服务端 token 管理（P2-3）：查看 / 轮换访问凭据（轮换后需重启服务生效）
    Token {
        #[command(subcommand)]
        action: TokenAction,
    },
    /// 格式迁移（P6b）：经插件解封装 B/C 级容器（L3 高风险；需 ACK 闸确认）
    FormatMigrate {
        /// 插件名（须已在白名单目录安装并启用；如 kwm-migration）
        #[arg(long)]
        plugin: String,
        /// 源文件（必须位于工作根内）
        #[arg(long, value_name = "FILE")]
        source: String,
        /// 输出目录（必须位于工作根内；绝不覆盖既有产物）
        #[arg(short = 'o', long)]
        output_dir: String,
        /// 授权工作根（路径边界；缺省 = 源文件父目录）
        #[arg(long)]
        work_root: Option<String>,
        /// 预留：QMCv2 类用户自备密钥（本地传递，绝不联网获取）
        #[arg(long)]
        ekey: Option<String>,
    },
    /// 整轨切分：CUE + WAV/FLAC/APE/WV/TAK 镜像 → 按轨道切分为独立文件（无损，源不动）
    Split {
        /// CUE 文件（编码自动检测：UTF-8/GBK/BIG5）
        #[arg(value_name = "CUE_FILE")]
        cue: String,
        /// 输出目录（分轨写入这里，命名 `NN 标题`）
        #[arg(short = 'o', long)]
        out: String,
        /// 输出格式（缺省：WAV/FLAC 源保持原格式；APE/WV/TAK 源 → FLAC）
        #[arg(long, value_parser = ["flac", "wav"])]
        format: Option<String>,
        /// ffmpeg 可执行文件路径（APE/WV/TAK 源切分必需）
        #[arg(long)]
        ffmpeg_path: Option<String>,
        /// JSON 输出（机器可读）
        #[arg(long)]
        json: bool,
    },
    /// genre 写入：文件名风格码 `[Y23-S01-...]` → genre 标签（默认 dry-run）
    Genre {
        /// 曲库目录
        #[arg(value_name = "DIR")]
        dir: String,
        /// codebook JSON（{"S01":"流行",...}；缺省 → genre 用原始码）
        #[arg(long)]
        map: Option<String>,
        /// 覆盖已有 genre（缺省 FillMissingOnly：已有 genre 的文件跳过）
        #[arg(long)]
        replace_all: bool,
        /// 真正写入（缺省=只报告将写什么）
        #[arg(long)]
        apply: bool,
        /// JSON 输出（机器可读）
        #[arg(long)]
        json: bool,
    },
}

/// 服务端 token 管理动作（P2-3）：凭据轮换 / 查看。
#[derive(clap::Subcommand, Debug)]
pub enum TokenAction {
    /// 轮换 token（生成新值并写回；**需重启服务后生效**）
    Rotate {
        /// 数据目录（默认取 MUSICFORGE_DATA_DIR，其次 ./data）
        #[arg(long)]
        data_dir: Option<String>,
        /// 直接指定 token 文件路径（优先于 data_dir）
        #[arg(long)]
        token_file: Option<String>,
    },
    /// 查看当前 token 与其文件路径
    Show {
        /// 数据目录（默认取 MUSICFORGE_DATA_DIR，其次 ./data）
        #[arg(long)]
        data_dir: Option<String>,
        /// 直接指定 token 文件路径（优先于 data_dir）
        #[arg(long)]
        token_file: Option<String>,
    },
}

#[derive(clap::Subcommand, Debug)]
pub enum PluginsCmd {
    /// 列出白名单目录内已安装插件与启用/确认状态
    List,
    /// 启用插件（整表覆盖：`--names` 逗号分隔）
    Enable {
        /// 插件名列表（逗号分隔）
        #[arg(long, value_delimiter = ',')]
        names: Vec<String>,
    },
    /// 禁用全部插件（enabled 置空）
    DisableAll,
    /// 高风险插件确认（ACK 闸；幂等）
    Acknowledge {
        /// 插件名
        #[arg(long)]
        name: String,
    },
}

#[derive(clap::Subcommand, Debug)]
pub enum PlaylistCmd {
    /// 按分类导出曲库为 .m3u8 集合（每组一个文件，UTF-8 + EXTINF）
    Export {
        /// 曲库目录
        #[arg(value_name = "DIR")]
        dir: String,
        /// 输出目录（清单写到这里，必填——避免静默写进任何隐式位置）
        #[arg(long)]
        out: String,
        /// 分类键：artist（默认）/ album / none
        #[arg(long, default_value = "artist")]
        by: String,
    },
    /// 导入播放清单并修复失效路径（同名匹配 + 时长 ±1s 消歧）
    Import {
        /// 播放清单文件（.m3u / .m3u8）
        #[arg(value_name = "FILE")]
        file: String,
        /// 修复搜索根（在此目录内按文件名定位失效条目的新位置）
        #[arg(long)]
        search: String,
        /// 修复后清单写出路径（缺省 = <原名>.fixed.m3u8）
        #[arg(long)]
        out: Option<String>,
        /// JSON 输出（机器可读）
        #[arg(long)]
        json: bool,
    },
}

mod commands;
pub use commands::*;

fn main() {
    let args = Args::parse();

    // v0.3.0 子命令分派：scan / clean（legacy 顶层参数不受影响）
    if let Some(sub) = args.command {
        let code = match sub {
            Sub::Plugins { cmd } => run_plugins_sub(cmd),
        Sub::Token { action } => run_token_sub(&action),
            Sub::FormatMigrate {
                plugin,
                source,
                output_dir,
                work_root,
                ekey,
            } => run_format_migrate_sub(
                &plugin,
                &source,
                &output_dir,
                work_root.as_deref(),
                ekey.as_deref(),
            ),
            Sub::Watch {
                level,
                target,
                template,
                debounce_ms,
                whitelist,
                dir,
            } => run_watch_sub(
                &dir,
                &level,
                target.as_deref(),
                &template,
                debounce_ms,
                &whitelist,
            ),
            Sub::Scan {
                dir,
                recursive,
                json,
                state_db,
            } => run_scan_sub(&dir, recursive, json, state_db.as_deref()),
            Sub::Clean {
                dir,
                rules,
                apply,
                trash,
                state_db,
                restore,
            } => run_clean_sub(
                dir.as_deref(),
                rules.as_deref(),
                apply,
                trash.as_deref(),
                state_db.as_deref(),
                restore.as_deref(),
            ),
            Sub::Dedupe {
                dir,
                state_db,
                apply,
                trash,
                no_same_name,
                include_same_name,
                suggest,
                covers,
                json,
            } => run_dedupe_sub(
                &dir,
                &DedupeArgs {
                    state_db,
                    apply,
                    trash,
                    no_same_name,
                    include_same_name,
                    suggest,
                    covers,
                    json,
                },
            ),
            Sub::Organize {
                dir,
                to,
                template,
                conflict,
                apply,
                json,
            } => run_organize_sub(
                &dir,
                &OrganizeArgs {
                    to,
                    template,
                    conflict,
                    apply,
                    json,
                },
            ),
            Sub::Playlist { cmd } => run_playlist_sub(cmd),
            Sub::Transcode {
                input,
                out,
                format,
                ffmpeg_path,
                i_know_lossy_to_lossless,
                json,
            } => run_transcode_sub(
                &input,
                &TranscodeArgs {
                    out,
                    format,
                    ffmpeg_path,
                    i_know_lossy_to_lossless,
                    json,
                },
            ),
            Sub::Split {
                cue,
                out,
                format,
                ffmpeg_path,
                json,
            } => run_split_sub(&cue, &out, format, ffmpeg_path, json),
            Sub::Genre {
                dir,
                map,
                replace_all,
                apply,
                json,
            } => run_genre_sub(
                &dir,
                &GenreArgs {
                    map,
                    replace_all,
                    apply,
                    json,
                },
            ),
        };
        std::process::exit(code);
    }

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

    // v0.2.0 安全分级：convert 属「只产出新文件」的非破坏类，默认执行；
    // 破坏类命令（P3 的 clean / P4 的 dedupe）届时默认只规划，须 --apply。
    let mode = musicforge_cli::safety::resolve(
        musicforge_cli::safety::OpClass::NonDestructive,
        &musicforge_cli::safety::OpFlags {
            dry_run: args.dry_run,
            apply: false,
            yes: false,
        },
    );
    let mode = match mode {
        Ok(m) => m,
        Err(e) => {
            eprintln!("✗ {e}");
            std::process::exit(2);
        }
    };
    if mode == musicforge_cli::safety::ExecMode::DryRun {
        println!(
            "{}",
            musicforge_cli::safety::mode_note(
                musicforge_cli::safety::OpClass::NonDestructive,
                mode
            )
        );
    }

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
                   // 状态库留痕（可再生缓存：失败只告警，绝不因此让转换失败）
    if let Some(path) = args.state_db.as_ref() {
        record_state(Path::new(path), &summary);
    }

    std::process::exit(summary.exit_code());
}
