// 拆分自 main.rs（P9 可维护性治理：巨型入口文件拆解）——**逐字迁移，行为零变更**。
use crate::*;

/// organize 子命令参数包（与 DedupeArgs 同理由：避免 too_many_arguments）。
/// transcode 子命令参数包。
pub struct TranscodeArgs {
    pub out: String,
    pub format: String,
    pub ffmpeg_path: Option<String>,
    pub i_know_lossy_to_lossless: bool,
    pub json: bool,
}

/// 转码子命令：无损走纯 Rust 管线（回读校验），有损走 ffmpeg sidecar（D1 永不捆绑）。
pub fn run_transcode_sub(inputs: &[String], a: &TranscodeArgs) -> i32 {
    use musicforge_core::ffmpeg::{classify_source, Ffmpeg, LossyPreset, SourceClass};
    use musicforge_core::lossless::{self, LosslessFormat};

    enum Target {
        Lossless(LosslessFormat),
        Lossy(LossyPreset),
    }
    let target = match a.format.as_str() {
        "flac" => Target::Lossless(LosslessFormat::Flac),
        "wav" => Target::Lossless(LosslessFormat::Wav),
        "mp3" => Target::Lossy(LossyPreset::Mp3),
        "aac" => Target::Lossy(LossyPreset::Aac),
        "opus" => Target::Lossy(LossyPreset::Opus),
        other => {
            eprintln!("✗ 未知目标格式 {other}（可选 flac|wav|mp3|aac|opus）");
            return 2;
        }
    };

    // 收集输入：文件/目录递归，认领无损（魔数）与有损（扩展名）源
    let mut files: Vec<PathBuf> = Vec::new();
    let mut skipped_inputs = 0usize;
    for raw in inputs {
        let p = Path::new(raw);
        if p.is_dir() {
            let mut stack = vec![p.to_path_buf()];
            while let Some(d) = stack.pop() {
                let Ok(rd) = std::fs::read_dir(&d) else {
                    continue;
                };
                for e in rd.flatten() {
                    let ep = e.path();
                    if ep.is_dir() {
                        if ep.file_name().and_then(|n| n.to_str()) == Some(".musicforge") {
                            continue;
                        }
                        stack.push(ep);
                    } else if classify_source(&ep).is_some() {
                        files.push(ep);
                    }
                }
            }
        } else if classify_source(p).is_some() {
            files.push(p.to_path_buf());
        } else {
            skipped_inputs += 1;
        }
    }
    files.sort();

    let out_dir = PathBuf::from(&a.out);
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!("✗ 输出目录创建失败: {e}");
        return 1;
    }

    #[derive(Debug)]
    struct Item {
        src: PathBuf,
        dst: PathBuf,
        bytes: u64,
    }
    struct Fail {
        src: String,
        reason: String,
    }
    let mut done: Vec<Item> = Vec::new();
    let mut same_format = 0usize;
    let mut failures: Vec<Fail> = Vec::new();
    let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    // ffmpeg 惰性解析（五级探测，只做一次；Err 复用给所有需要它的文件）
    let mut ff_resolved: Option<Result<Ffmpeg, musicforge_core::NcmError>> = None;
    let mut get_ff = |a: &TranscodeArgs, failures: &mut Vec<Fail>| -> Option<Ffmpeg> {
        let resolved = ff_resolved
            .get_or_insert_with(|| Ffmpeg::find(a.ffmpeg_path.as_deref().map(Path::new)));
        match resolved {
            Ok(ff) => Some(ff.clone()),
            Err(e) => {
                failures.push(Fail {
                    src: "(ffmpeg)".to_string(),
                    reason: format!("{}: {e}", e.mf_code()),
                });
                None
            }
        }
    };

    for src_path in &files {
        let Some(class) = classify_source(src_path) else {
            skipped_inputs += 1;
            continue;
        };
        let stem = src_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output")
            .to_string();

        let ext: String = match &target {
            Target::Lossless(f) => f.extension().to_string(),
            Target::Lossy(p) => p.extension().to_string(),
        };
        let mut n = 1usize;
        let mut dst = out_dir.join(format!("{stem}.{ext}"));
        loop {
            if used_names.insert(dst.to_string_lossy().into_owned()) && !dst.exists() {
                break;
            }
            n += 1;
            dst = out_dir.join(format!("{stem} ({n}).{ext}"));
        }

        match &target {
            Target::Lossless(tl) => match class {
                SourceClass::Lossless => {
                    if lossless::probe_lossless_file(src_path) == Some(*tl) {
                        same_format += 1;
                        continue;
                    }
                    match lossless::transcode(src_path, &dst, *tl) {
                        Ok(o) => done.push(Item {
                            src: src_path.clone(),
                            dst: o.dst,
                            bytes: o.bytes_written,
                        }),
                        Err(e) => failures.push(Fail {
                            src: src_path.to_string_lossy().into_owned(),
                            reason: format!("{}: {e}", e.mf_code()),
                        }),
                    }
                }
                SourceClass::Lossy => {
                    if !a.i_know_lossy_to_lossless {
                        failures.push(Fail {
                            src: src_path.to_string_lossy().into_owned(),
                            reason: "MF-LOSSY-TO-LOSSLESS: 有损→无损升级被拦截（如确需加 --i-know-lossy-to-lossless）".to_string(),
                        });
                        continue;
                    }
                    let Some(ff) = get_ff(a, &mut failures) else {
                        continue;
                    };
                    let args: &[&str] = match tl {
                        LosslessFormat::Flac => &["-codec:a", "flac"],
                        // 24-bit 容器承载 16/24 位源（值空间无损；16le 会静默降位深）
                        LosslessFormat::Wav => &["-codec:a", "pcm_s24le"],
                    };
                    match ff.export_custom(src_path, &dst, args) {
                        Ok(bytes) => done.push(Item {
                            src: src_path.clone(),
                            dst,
                            bytes,
                        }),
                        Err(e) => failures.push(Fail {
                            src: src_path.to_string_lossy().into_owned(),
                            reason: format!("{}: {e}", e.mf_code()),
                        }),
                    }
                }
            },
            Target::Lossy(preset) => {
                if src_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case(preset.extension()))
                    .unwrap_or(false)
                {
                    same_format += 1;
                    continue;
                }
                let Some(ff) = get_ff(a, &mut failures) else {
                    continue;
                };
                match ff.export_lossy(src_path, &dst, *preset) {
                    Ok(bytes) => done.push(Item {
                        src: src_path.clone(),
                        dst,
                        bytes,
                    }),
                    Err(e) => failures.push(Fail {
                        src: src_path.to_string_lossy().into_owned(),
                        reason: format!("{}: {e}", e.mf_code()),
                    }),
                }
            }
        }
    }

    let total_bytes: u64 = done.iter().map(|i| i.bytes).sum();
    if a.json {
        let out = serde_json::json!({
            "inputs": inputs,
            "target": a.format,
            "found": files.len(),
            "skipped_same_format": same_format,
            "skipped_inputs": skipped_inputs,
            "ok": done.len(),
            "failed": failures.len(),
            "bytes_written": total_bytes,
            "items": done.iter().map(|i| serde_json::json!({
                "source": i.src.display().to_string(),
                "target": i.dst.display().to_string(),
                "bytes": i.bytes,
            })).collect::<Vec<_>>(),
            "failures": failures.iter().map(|f| serde_json::json!({
                "source": f.src, "reason": f.reason,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        return if failures.is_empty() { 0 } else { 1 };
    }

    println!(
        "输入 {} · 认领 {} · 同格式跳过 {} · 成功 {} · 失败 {} · 写出 {:.1} MB",
        files.len() + skipped_inputs,
        files.len(),
        same_format,
        done.len(),
        failures.len(),
        total_bytes as f64 / 1024.0 / 1024.0
    );
    for i in &done {
        println!("  {} → {}", i.src.display(), i.dst.display());
    }
    for f in &failures {
        println!("  ✕ {} — {}", f.src, f.reason);
    }
    println!("校验承诺：无损产物逐样本回读一致；有损产物容器魔数+时长差<1s 回读校验。");
    if failures.is_empty() {
        0
    } else {
        1
    }
}

/// 整轨切分子命令：CUE → 独立音轨（校验在写盘前完成，失败轨不落盘）。
pub fn run_split_sub(
    cue: &str,
    out: &str,
    format: Option<String>,
    ffmpeg_path: Option<String>,
    json: bool,
) -> i32 {
    // APE/WV/TAK 源需要 ffmpeg sidecar（D21）；WAV/FLAC 源不需要
    let needs_ff = Path::new(cue)
        .parent()
        .and_then(|dir| {
            let sheet_text = std::fs::read_to_string(cue).ok()?;
            let file_line = sheet_text
                .lines()
                .find(|l| l.trim_start().to_uppercase().starts_with("FILE"))?;
            let name = file_line
                .split('"')
                .nth(1)
                .map(|s| s.to_string())
                .or_else(|| file_line.split_whitespace().nth(1).map(|s| s.to_string()))?;
            Some(
                dir.join(name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_lowercase()),
            )
        })
        .flatten()
        .map(|ext| matches!(ext.as_str(), "ape" | "wv" | "tak"))
        .unwrap_or(false);
    let ff = if needs_ff {
        match musicforge_core::ffmpeg::Ffmpeg::find(ffmpeg_path.as_deref().map(Path::new)) {
            Ok(ff) => Some(ff),
            Err(e) => {
                eprintln!("✗ {e}");
                eprintln!(
                    "  {}",
                    musicforge_core::NcmError::FfmpegMissing { searched: 0 }.suggestion()
                );
                return 1;
            }
        }
    } else {
        None
    };
    let target = format
        .as_deref()
        .and_then(musicforge_core::lossless::LosslessFormat::parse);

    match musicforge_core::cue::split_cue_ex(
        Path::new(cue),
        Path::new(out),
        target,
        ff.as_ref(),
        |n, t| {
            // 清洗由 split_cue_ex 统一执行（template::sanitize）；此处只做命名
            let title = t.title.as_deref().unwrap_or("Unknown Track");
            format!("{n:02} {title}")
        },
    ) {
        Ok(report) => {
            if json {
                let o = serde_json::json!({
                    "cue": cue,
                    "source": report.source.display().to_string(),
                    "album": report.sheet.title,
                    "tracks": report.tracks.iter().map(|t| serde_json::json!({
                        "index": t.index,
                        "title": t.title,
                        "path": t.dst.display().to_string(),
                        "durationSecs": format!("{:.2}", t.duration_secs),
                    })).collect::<Vec<_>>(),
                    "failed": report.failed.iter().map(|(no, r)| serde_json::json!({
                        "track": no, "reason": r,
                    })).collect::<Vec<_>>(),
                });
                println!("{}", serde_json::to_string_pretty(&o).unwrap_or_default());
            } else {
                println!(
                    "切分完成 {} 轨 · 失败 {} 轨 · 专辑「{}」",
                    report.tracks.len(),
                    report.failed.len(),
                    report.sheet.title.as_deref().unwrap_or("—")
                );
                for t in &report.tracks {
                    println!(
                        "  {:02} {} ({:.2}s) → {}",
                        t.index,
                        t.title.as_deref().unwrap_or("Unknown"),
                        t.duration_secs,
                        t.dst.display()
                    );
                }
                for (no, r) in &report.failed {
                    println!("  ✕ 轨 {no}: {r}");
                }
                if !report.failed.is_empty() {
                    println!("失败轨未写盘（校验在写盘前完成）；请检查 CUE 与音频是否匹配。");
                }
            }
            if report.failed.is_empty() {
                0
            } else {
                1
            }
        }
        Err(e) => {
            eprintln!("✗ 切分失败: {e}");
            1
        }
    }
}
