//! QA（严过关 Yan#2）第二轮**独立验证** —— CLI 层。
//!
//! ⚠ 文件名隔离：本文件为 `qa_yan2_round2.rs`，与另一位 QA 的 `qa_yan_round2.rs`
//! 互不干扰（曾发生同名覆盖事故，故改用 `qa_yan2_` 前缀）。
//!
//! 覆盖：B8 产物可见性 / 2bd4a41 TagRead 降级**未被改宽**（正向 + 反向 + 混合批次）/
//! CSV 语义实测 / B9 并发上界 / B10 目录深度（既有测试**未覆盖**）/ B11 静默成功 /
//! C1 结果计数 / G3 结构镜像。
//!
//! 独立性：NCM 编码器与容器载荷**逐字节自建**（依据 `musicforge-core/src/header.rs`
//! 的布局契约重写），不 import 实现者 `qa_adversarial.rs` 的 `ncm` 模块。

use aes::cipher::{generic_array::GenericArray, BlockEncrypt, KeyInit};
use aes::Aes128;
use musicforge_cli::{BatchConfig, BatchSummary, Status};
use std::path::{Path, PathBuf};

// ==================== 自建 NCM 编码器（依据 header.rs 布局契约重写） ====================

const MAGIC: &[u8; 8] = b"CTENFDAM";

fn aes_ecb_encrypt(key: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let cipher = Aes128::new(GenericArray::from_slice(key));
    let pad = 16 - data.len() % 16;
    let mut out = data.to_vec();
    out.extend(std::iter::repeat_n(pad as u8, pad));
    for chunk in out.as_chunks_mut::<16>().0 {
        cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
    }
    out
}

/// NCM RC4 变体 KSA（与 crypto::build_key_box 等价，独立实现）
fn key_box(rc4_key: &[u8]) -> [u8; 256] {
    assert!(!rc4_key.is_empty());
    let mut s = [0u8; 256];
    for (i, b) in s.iter_mut().enumerate() {
        *b = i as u8;
    }
    let (mut last, mut koff) = (0usize, 0usize);
    for i in 0..256 {
        let swap = s[i];
        let c = (swap as usize + last + rc4_key[koff] as usize) & 0xff;
        koff = (koff + 1) % rc4_key.len();
        s[i] = s[c];
        s[c] = swap;
        last = c;
    }
    s
}

/// 音频区加密（与 crypto::keystream_byte 互逆，独立实现）
fn ncm_crypt(data: &[u8], s: &[u8; 256]) -> Vec<u8> {
    let mut out = data.to_vec();
    let mut ks = Vec::with_capacity(out.len());
    for i in 0..out.len() {
        let j = (i + 1) & 0xff;
        let a = s[j] as usize;
        let b = s[(a + j) & 0xff] as usize;
        ks.push(s[(a + b) & 0xff]);
    }
    for (o, k) in out.iter_mut().zip(ks) {
        *o ^= k;
    }
    out
}

fn meta_raw(music_name: &str, album: &str, fmt: Option<&str>) -> Vec<u8> {
    use base64::Engine as _;
    let mut j = serde_json::json!({
        "musicId": 1,
        "musicName": music_name,
        "artist": [["独立验证歌手", 0]],
        "albumId": 1,
        "album": album,
        "albumPic": "",
        "bitrate": 320000,
        "duration": 1000,
    });
    if let Some(f) = fmt {
        j["format"] = serde_json::json!(f);
    }
    let inner = aes_ecb_encrypt(&musicforge_core::META_KEY, format!("music:{}", j).as_bytes());
    let full = format!(
        "163 key(Don't modify):{}",
        base64::engine::general_purpose::STANDARD.encode(inner)
    );
    full.bytes().map(|b| b ^ 0x63).collect()
}

/// 装配一个 `.ncm`。`declared_format` = 元数据里声明的 format（None = 不声明）。
fn craft_ncm(audio: &[u8], name: &str, album: &str, declared_format: Option<&str>) -> Vec<u8> {
    let rc4_key: Vec<u8> = (0..112u32).map(|i| (i * 5 + 11) as u8).collect();
    let key_plain = [b"neteasecloudmusic".as_slice(), rc4_key.as_slice()].concat();
    let key_data: Vec<u8> = aes_ecb_encrypt(&musicforge_core::CORE_KEY, &key_plain)
        .iter()
        .map(|b| b ^ 0x64)
        .collect();
    let meta = meta_raw(name, album, declared_format);

    let mut head = Vec::new();
    head.extend_from_slice(MAGIC);
    head.extend_from_slice(&[0x01, 0x69]);
    head.extend_from_slice(&(key_data.len() as u32).to_le_bytes());
    head.extend_from_slice(&key_data);
    head.extend_from_slice(&(meta.len() as u32).to_le_bytes());
    head.extend_from_slice(&meta);
    head.extend_from_slice(&crc32fast::hash(&head).to_le_bytes());
    head.push(0x01);
    head.extend_from_slice(&0u32.to_le_bytes()); // coverLen1
    head.extend_from_slice(&0u32.to_le_bytes()); // coverLen2
    head.extend_from_slice(&ncm_crypt(audio, &key_box(&rc4_key)));
    head
}

// ==================== 自建音频载荷 ====================

/// MPEG-1 Layer III / 128kbps / 44100Hz 帧（帧长 417）
fn mp3_frame() -> Vec<u8> {
    let mut f = vec![0xFFu8, 0xFB, 0x90, 0x00];
    f.extend(std::iter::repeat_n(0u8, 413));
    f
}

fn bare_mp3(frames: usize) -> Vec<u8> {
    let mut v = Vec::new();
    for _ in 0..frames {
        v.extend(mp3_frame());
    }
    v
}

fn minimal_flac() -> Vec<u8> {
    let mut info = Vec::new();
    info.extend_from_slice(&4096u16.to_be_bytes());
    info.extend_from_slice(&4096u16.to_be_bytes());
    info.extend_from_slice(&[0u8; 3]);
    info.extend_from_slice(&[0u8; 3]);
    let packed: u64 = (44100u64 << 44) | (1u64 << 41) | (15u64 << 36);
    info.extend_from_slice(&packed.to_be_bytes());
    info.extend_from_slice(&[0u8; 16]);
    let mut out = b"fLaC".to_vec();
    out.push(0x80);
    out.extend_from_slice(&[0u8, 0u8, 34u8]);
    out.extend_from_slice(&info);
    out.extend_from_slice(&[0u8; 512]);
    out
}

/// 自检：我这套自建编码器必须能被 musicforge-core 正确解析，否则后面所有用例都无意义。
#[test]
fn yan2_selftest_my_ncm_encoder_roundtrips() {
    let dir = tempfile::tempdir().unwrap();
    let audio = minimal_flac();
    let p = dir.path().join("rt.ncm");
    std::fs::write(&p, craft_ncm(&audio, "往返", "QA", Some("flac"))).unwrap();

    let mut dec = musicforge_core::Decoder::open(&p).expect("自建 NCM 必须能打开");
    let fmt = dec.detect_format().expect("格式应判为 flac");
    assert_eq!(fmt.extension(), "flac");
    let out = dir.path().join("rt.flac");
    dec.dump_to(&out).expect("dump 应成功");
    assert_eq!(
        std::fs::read(&out).unwrap(),
        audio,
        "自建编码器往返必须逐字节一致"
    );
}

/// 允许自定义 RC4 密钥的装配入口（B6 空密钥用例用）
fn craft_ncm_with_key(
    audio: &[u8],
    rc4_key: &[u8],
    name: &str,
    declared_format: Option<&str>,
) -> Vec<u8> {
    let key_plain = [b"neteasecloudmusic".as_slice(), rc4_key].concat();
    let key_data: Vec<u8> = aes_ecb_encrypt(&musicforge_core::CORE_KEY, &key_plain)
        .iter()
        .map(|b| b ^ 0x64)
        .collect();
    let meta = meta_raw(name, "QA", declared_format);
    let mut head = Vec::new();
    head.extend_from_slice(MAGIC);
    head.extend_from_slice(&[0x01, 0x69]);
    head.extend_from_slice(&(key_data.len() as u32).to_le_bytes());
    head.extend_from_slice(&key_data);
    head.extend_from_slice(&(meta.len() as u32).to_le_bytes());
    head.extend_from_slice(&meta);
    head.extend_from_slice(&crc32fast::hash(&head).to_le_bytes());
    head.push(0x01);
    head.extend_from_slice(&0u32.to_le_bytes());
    head.extend_from_slice(&0u32.to_le_bytes());
    head.extend_from_slice(audio); // 空密钥时无需加密（解析必在密钥阶段失败）
    head
}

// ==================== 第一轮修复回归（自建样本，验证本轮没改坏） ====================

/// B6：空 RC4 密钥 → `Err(EmptyKey)`（码 `NCM-STRUCT-INVALID`），
/// 两条解析路径（parse_stream / parse_blob）都不 panic。
#[test]
fn yan2_regression_b6_empty_key_rejected() {
    use musicforge_core::NcmError;

    let blob = craft_ncm_with_key(&minimal_flac(), &[], "空密钥", Some("flac"));
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("empty_key.ncm");
    std::fs::write(&p, &blob).unwrap();

    // 路径 1：Decoder::open（parse_stream）
    let r = std::panic::catch_unwind(|| musicforge_core::Decoder::open(&p));
    assert!(r.is_ok(), "Decoder::open 不得 panic（索引越界）");
    let err = r.unwrap().map(|_| ()).expect_err("空 RC4 密钥必须返回错误");
    assert!(matches!(err, NcmError::EmptyKey), "应报 EmptyKey，实际: {err}");
    assert_eq!(err.code(), "NCM-STRUCT-INVALID", "稳定错误码必须归类结构损坏");

    // 路径 2：parse_blob（整读解析）
    let r2 = std::panic::catch_unwind(|| musicforge_core::header::parse_blob(&blob));
    assert!(r2.is_ok(), "parse_blob 不得 panic");
    assert!(r2.unwrap().is_err(), "parse_blob 空密钥必须返回 Err");
}

/// K2：三级判定皆失败 → `Err(UnknownFormat)`（码 `NCM-FORMAT-UNKNOWN`）且**零产物**。
#[test]
fn yan2_regression_k2_unknown_format_no_output() {
    use musicforge_core::NcmError;

    // 不声明 format + 全零音频（不匹配任何魔数）
    let blob = craft_ncm(&vec![0u8; 512], "未知格式", "QA", None);
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("unknown.ncm");
    std::fs::write(&p, &blob).unwrap();
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).unwrap();

    let err = musicforge_core::Decoder::open(&p)
        .and_then(|mut d| d.dump(Some(out.as_path())))
        .expect_err("未知格式必须显式报错，不得兜底 flac 产出垃圾文件");
    assert!(matches!(err, NcmError::UnknownFormat), "应报 UnknownFormat，实际: {err}");
    assert_eq!(err.code(), "NCM-FORMAT-UNKNOWN");
    assert!(
        std::fs::read_dir(&out).unwrap().next().is_none(),
        "未知格式必须零产物（不得产出假 .flac 垃圾文件）"
    );
}

/// G5：`Decoder::open` 的 `stream_offset` 从 0 起 → `detect_format` 能读到**真字节**。
/// 构造：不声明 format，音频是裸 MP3（帧同步字 ff fb）→ 魔数兜底必须判 Mp3，
/// 且 dump 输出与源音频逐字节相等（若 stream_offset 错位，两者都会错）。
#[test]
fn yan2_regression_g5_magic_sniff_reads_real_bytes() {
    let audio = bare_mp3(4);
    let blob = craft_ncm(&audio, "裸MP3", "QA", None); // 不声明 → 走魔数兜底
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("bare.ncm");
    std::fs::write(&p, &blob).unwrap();

    let mut dec = musicforge_core::Decoder::open(&p).unwrap();
    let fmt = dec.detect_format().expect("魔数兜底应识别裸 MP3");
    assert_eq!(fmt, musicforge_core::Format::Mp3, "魔数嗅探必须读到真字节（stream_offset 应为 0）");

    let target = dir.path().join("bare.mp3");
    dec.dump_to(&target).unwrap();
    assert_eq!(
        std::fs::read(&target).unwrap(),
        audio,
        "dump 输出必须与源音频逐字节相等（stream_offset 归零后才成立）"
    );
}

// ==================== 工具 ====================

fn cfg(inputs: Vec<PathBuf>, out: &Path, template: &str) -> BatchConfig {
    BatchConfig {
        inputs,
        out_dir: Some(out.to_path_buf()),
        recursive: true,
        skip_existing: false,
        jobs: 4,
        template: template.to_string(),
        cancel: None,
    }
}

fn scratch(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("musicforge-yan2-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

fn find<'a>(s: &'a BatchSummary, stem: &str) -> &'a musicforge_cli::FileResult {
    s.results
        .iter()
        .find(|r| r.source.file_stem().and_then(|x| x.to_str()) == Some(stem))
        .unwrap_or_else(|| panic!("未找到源 {stem} 的结果（结果数 {}）", s.results.len()))
}

// ==================== B8 / 2bd4a41 正向：落盘成功 + TagRead 降级 ====================

/// metadata 声明 flac、负载实为裸 MP3 → dump 成功、lofty 按扩展名解析失败。
/// 产物必须可见、内容逐字节正确，且降级为 Ok 而非 Failed。
#[test]
fn yan2_b8_tagread_product_visible_and_byte_exact() {
    let base = scratch("b8");
    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();

    let audio = bare_mp3(4);
    let src = base.join("mismatch.ncm");
    std::fs::write(&src, craft_ncm(&audio, "格式不符", "QA", Some("flac"))).unwrap();

    let s = musicforge_cli::run(cfg(vec![src], &out, "{title}"));
    assert_eq!(s.results.len(), 1, "结果数必须等于输入数");

    let r = find(&s, "mismatch");
    assert_eq!(r.status, Status::Ok, "TagRead 必须降级为 Ok，实际 reason={:?}", r.reason);

    let p = r.output.as_ref().expect("产物已落盘就必须带出路径");
    assert!(p.exists(), "报出的输出路径必须真实存在：{p:?}");
    assert_eq!(std::fs::read(p).unwrap(), audio, "落盘内容必须与解密后音频逐字节相等");

    let reason = r.reason.as_deref().unwrap_or("");
    assert!(reason.contains("NCM-TAG-READ"), "需带稳定错误码：{reason}");
    assert!(reason.contains("音频已完整导出"), "需说明音频已导出：{reason}");
    assert!(reason.contains("建议"), "需给出可操作建议：{reason}");

    assert_eq!(s.failed, 0);
    assert_eq!(s.ok, 1);
    assert_eq!(s.exit_code(), 0, "元数据问题不得把退出码变成 1");

    let _ = std::fs::remove_dir_all(&base);
}

/// 2bd4a41 反向：非 TagRead 的落盘后失败（sidecar 路径被占成目录）→ 必须仍为 Failed。
/// 与正向用例**唯一自变量是错误变体**，因变量必须相反 → 把「降级未被改宽」钉死。
#[test]
fn yan2_reverse_marker_failure_stays_failed() {
    let base = scratch("marker");
    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::create_dir_all(out.join("标记失败.mp3.musicforge.json")).unwrap();

    let src = base.join("s.ncm");
    std::fs::write(&src, craft_ncm(&bare_mp3(4), "标记失败", "QA", Some("mp3"))).unwrap();

    let s = musicforge_cli::run(cfg(vec![src], &out, "{title}"));
    let r = find(&s, "s");

    assert_eq!(r.status, Status::Failed, "非 TagRead 错误不得被降级为 Ok");
    let p = r.output.as_ref().expect("产物已落盘就必须带出路径");
    assert!(p.exists(), "产物必须可见（B8 语义与本条正交，不得回退）");
    assert!(!std::fs::read(p).unwrap().is_empty());

    let reason = r.reason.as_deref().unwrap_or("");
    assert!(!reason.contains("NCM-TAG-READ"), "不得复用降级文案：{reason}");
    assert!(reason.contains("完整性标记写入失败"), "需说明具体哪一步失败：{reason}");
    assert_eq!(s.failed, 1);
    assert_eq!(s.exit_code(), 1);

    let _ = std::fs::remove_dir_all(&base);
}

/// **既有测试未覆盖**：一个降级项 + 一个真实失败项的混合批次。
/// 降级项不得把整批退出码洗成 0（否则 CI/脚本无法发现真实失败）。
#[test]
fn yan2_mixed_degraded_and_real_failure_exit_code_is_one() {
    let base = scratch("mixed");
    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::create_dir_all(out.join("真失败.mp3.musicforge.json")).unwrap();

    std::fs::write(
        base.join("degraded.ncm"),
        craft_ncm(&bare_mp3(4), "降级项", "QA", Some("flac")), // 声明 flac / 实为 mp3 → TagRead
    )
    .unwrap();
    std::fs::write(
        base.join("failed.ncm"),
        craft_ncm(&bare_mp3(4), "真失败", "QA", Some("mp3")), // 正常音频，但标记写入失败
    )
    .unwrap();

    let s = musicforge_cli::run(cfg(
        vec![base.join("degraded.ncm"), base.join("failed.ncm")],
        &out,
        "{title}",
    ));
    assert_eq!(s.results.len(), 2, "结果数必须等于输入数");

    assert_eq!(find(&s, "degraded").status, Status::Ok, "TagRead 项应降级为 Ok");
    assert_eq!(find(&s, "failed").status, Status::Failed, "标记失败项应判 Failed");

    assert_eq!(s.ok, 1, "降级项计入 ok");
    assert_eq!(s.failed, 1, "真实失败项计入 failed");
    assert_eq!(
        s.exit_code(),
        1,
        "混合批次中只要有一个真实失败，退出码就必须为 1（降级项不得洗码）"
    );

    let _ = std::fs::remove_dir_all(&base);
}

// ==================== 降级项在失败清单 CSV 中的表现（实测 + 钉死行为） ====================

/// **已知语义变化，实测确认**：降级项 `status == Ok`，而 `export_failures_csv`
/// 按 `status == Failed` 过滤 → 降级项**不会**出现在 `--export-failures` 的 CSV 里。
#[test]
fn yan2_degraded_item_is_absent_from_failures_csv() {
    let base = scratch("csv");
    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();

    std::fs::write(
        base.join("degraded.ncm"),
        craft_ncm(&bare_mp3(4), "降级项", "QA", Some("flac")),
    )
    .unwrap();
    std::fs::write(
        base.join("good.ncm"),
        craft_ncm(&minimal_flac(), "正常项", "QA", Some("flac")),
    )
    .unwrap();

    let s = musicforge_cli::run(cfg(
        vec![base.join("degraded.ncm"), base.join("good.ncm")],
        &out,
        "{title}",
    ));
    assert_eq!(find(&s, "degraded").status, Status::Ok, "降级项应为 Ok");
    assert_eq!(s.failed, 0);

    let csv = base.join("failures.csv");
    s.export_failures_csv(&csv).unwrap();
    let text = std::fs::read_to_string(&csv).unwrap();
    eprintln!("[CSV 实测内容]\n{text}");
    assert!(!text.contains("降级项"), "实测：降级项不应出现在失败清单 CSV 中，实际 CSV=\n{text}");
    assert_eq!(text.lines().count(), 1, "CSV 应只剩表头一行，实际 {text:?}");

    let _ = std::fs::remove_dir_all(&base);
}

/// 降级告警在**命令行输出**里的可观测性实测（观察而非预设结论）。
#[test]
fn yan2_degraded_warning_visibility_in_cli_output() {
    let base = scratch("vis");
    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(
        base.join("degraded.ncm"),
        craft_ncm(&bare_mp3(4), "降级项", "QA", Some("flac")),
    )
    .unwrap();

    let o = std::process::Command::new(env!("CARGO_BIN_EXE_musicforge"))
        .args([
            base.join("degraded.ncm").to_string_lossy().to_string(),
            "-o".to_string(),
            out.to_string_lossy().to_string(),
        ])
        .output()
        .expect("CLI 应可执行");

    let stdout = String::from_utf8_lossy(&o.stdout);
    let stderr = String::from_utf8_lossy(&o.stderr);
    eprintln!("[stdout]\n{stdout}\n[stderr]\n{stderr}\n[exit] {:?}", o.status.code());

    let mentioned = stdout.contains("NCM-TAG-READ")
        || stderr.contains("NCM-TAG-READ")
        || stdout.contains("元数据写入失败")
        || stderr.contains("元数据写入失败");
    // 观察项：不做正向断言，只把事实打出来供人工判读。
    eprintln!("[观察] 降级告警是否在 CLI 输出中可见: {mentioned}");
    assert_eq!(o.status.code(), Some(0), "降级项不得改变退出码");

    let _ = std::fs::remove_dir_all(&base);
}

// ==================== B9：并发数上界 ====================

#[test]
fn yan2_b9_absurd_jobs_bounded_and_completes() {
    let base = scratch("jobs");
    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();

    let n = 6;
    let inputs: Vec<PathBuf> = (0..n)
        .map(|i| {
            let p = base.join(format!("f{i}.ncm"));
            std::fs::write(&p, craft_ncm(&minimal_flac(), &format!("曲{i}"), "QA", Some("flac")))
                .unwrap();
            p
        })
        .collect();

    let mut c = cfg(inputs, &out, "{title}");
    c.jobs = 200_000; // 荒谬值：必须被钳制，不得尝试创建 20 万个线程
    let s = musicforge_cli::run(c);

    assert_eq!(s.results.len(), n, "结果数必须等于输入数");
    assert_eq!(s.ok, n, "荒谬并发值不得影响正确性，实际 ok={} failed={}", s.ok, s.failed);
    assert_eq!(s.failed, 0);
    assert_eq!(s.exit_code(), 0);

    let _ = std::fs::remove_dir_all(&base);
}

/// 下界同样要有：`jobs = 0` 必须钳制到至少 1，否则 `thread::scope` 起 0 个 worker
/// → 队列永远不排空 → 结果数 < 输入数（C1 类静默丢结果）。
#[test]
fn yan2_b9_jobs_zero_is_bounded_to_at_least_one() {
    let base = scratch("jobs0");
    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();

    let inputs: Vec<PathBuf> = (0..3)
        .map(|i| {
            let p = base.join(format!("z{i}.ncm"));
            std::fs::write(&p, craft_ncm(&minimal_flac(), &format!("零{i}"), "QA", Some("flac")))
                .unwrap();
            p
        })
        .collect();

    let mut c = cfg(inputs, &out, "{title}");
    c.jobs = 0;
    let s = musicforge_cli::run(c);
    assert_eq!(s.results.len(), 3, "jobs=0 也必须处理完全部输入（结果数恒等于输入数）");
    assert_eq!(s.ok, 3, "jobs=0 应被钳制到 >=1，实际 ok={}", s.ok);

    let _ = std::fs::remove_dir_all(&base);
}

// ==================== B10：目录递归深度上界（既有测试未覆盖） ====================

/// 用 70 层普通目录树把深度上界钉在 ~64：
/// · 第 63 层（walk 深度 64）的文件**必须**被收集（上界不得误伤正常深度）
/// · 第 66 层（walk 深度 67 > 64）的文件**必须不**被收集（上界确实生效）
/// · 全程不得栈溢出 / abort
#[test]
fn yan2_b10_depth_cap_is_at_64_and_no_overflow() {
    let base = scratch("depth");
    let root = base.join("root");

    let audio = minimal_flac();
    let blob = craft_ncm(&audio, "深层", "QA", Some("flac"));
    let mut cur = root.clone();
    for level in 1..=70 {
        cur = cur.join(format!("d{level}"));
        std::fs::create_dir_all(&cur).unwrap();
        if level == 63 || level == 66 {
            std::fs::write(cur.join(format!("lv{level}.ncm")), &blob).unwrap();
        }
    }
    std::fs::write(root.join("shallow.ncm"), &blob).unwrap();

    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let s = musicforge_cli::run(cfg(vec![root], &out, "{title}"));

    let mut found: Vec<String> = s
        .results
        .iter()
        .map(|r| r.source.file_stem().and_then(|x| x.to_str()).unwrap_or("").to_string())
        .collect();
    found.sort();

    assert!(found.contains(&"shallow".to_string()), "根层文件必须被收集，实际 {found:?}");
    assert!(found.contains(&"lv63".to_string()), "第 63 层属正常深度，不得被上界误伤，实际 {found:?}");
    assert!(!found.contains(&"lv66".to_string()), "第 66 层必须被深度上界截断（深度 > 64），实际 {found:?}");
    assert_eq!(s.results.len(), 2, "应恰好收集 2 个，实际 {found:?}");

    let _ = std::fs::remove_dir_all(&base);
}

/// 自引用 junction（`base/loop -> root`）：无深度上界时 `is_dir()` 会无限跟随。
/// 断言：进程不崩、不栈溢出，且仍能正常返回。
///
/// 注意：Windows MAX_PATH 会让超长路径的 `read_dir` 自然失败，因此本用例
/// 只能证明「不崩」，深度上界的**精确位置**由上一条用例（70 层）证明。
#[test]
fn yan2_b10_self_referencing_junction_does_not_overflow() {
    let base = scratch("junction");
    let root = base.join("root");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("ok.ncm"),
        craft_ncm(&minimal_flac(), "正常", "QA", Some("flac")),
    )
    .unwrap();

    let st = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J", "loop", "root"])
        .current_dir(&base)
        .output();
    let created = matches!(&st, Ok(o) if o.status.success());
    if !created {
        eprintln!("[跳过] 无法创建 junction（无权限或不支持），本用例仅在支持时生效");
        let _ = std::fs::remove_dir_all(&base);
        return;
    }
    assert!(base.join("loop").exists(), "junction 应创建成功");

    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();
    // 若深度上界失效，这里会无限递归 → 栈溢出 → 进程 abort（测试进程直接死掉）
    let s = musicforge_cli::run(cfg(vec![root], &out, "{title}"));

    assert!(
        s.results
            .iter()
            .any(|r| r.source.file_stem().and_then(|x| x.to_str()) == Some("ok")),
        "自引用 junction 不得影响正常文件的收集，实际结果数 {}",
        s.results.len()
    );
    assert!(s.results.len() <= 64, "结果数应在上界内，实际 {}", s.results.len());

    let _ = std::process::Command::new("cmd")
        .args(["/C", "rmdir", "loop"])
        .current_dir(&base)
        .output();
    let _ = std::fs::remove_dir_all(&base);
}

// ==================== B11：输入无匹配时的静默成功 ====================

/// 不存在的路径 + 可读但无 .ncm 的目录：CLI 必须在 stderr 给出告警，
/// 且**退出码语义不被破坏**（本用例记录观察到的退出码）。
#[test]
fn yan2_b11_nonexistent_path_warns_on_stderr() {
    let base = scratch("b11");
    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();

    let exe = env!("CARGO_BIN_EXE_musicforge");
    let missing = base.join("i_do_not_exist_at_all");
    // 输入是**位置参数**（`-d/--directory` 才是目录）
    let o = std::process::Command::new(exe)
        .args([
            missing.to_string_lossy().to_string(),
            "-o".to_string(),
            out.to_string_lossy().to_string(),
        ])
        .output()
        .expect("CLI 应可执行");

    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(
        stderr.contains("不存在") || stderr.contains("未发现任何"),
        "不存在的输入必须在 stderr 告警，实际 stderr={stderr:?}"
    );
    eprintln!("[B11-不存在] exit={:?} stderr={}", o.status.code(), stderr.trim());

    let empty = base.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    std::fs::write(empty.join("not_music.txt"), b"hi").unwrap();
    let o2 = std::process::Command::new(exe)
        .args([
            "-d".to_string(),
            empty.to_string_lossy().to_string(),
            "-r".to_string(),
            "-o".to_string(),
            out.to_string_lossy().to_string(),
        ])
        .output()
        .expect("CLI 应可执行");
    let stderr2 = String::from_utf8_lossy(&o2.stderr);
    assert!(stderr2.contains("未发现任何"), "无 .ncm 的目录必须给出告警，实际 stderr={stderr2:?}");
    eprintln!("[B11-无ncm] exit={:?} stderr={}", o2.status.code(), stderr2.trim());
    eprintln!(
        "[B11 退出码语义] 不存在={:?} / 无ncm={:?}（观测值，用于确认告警未破坏退出码语义）",
        o.status.code(),
        o2.status.code()
    );

    let _ = std::fs::remove_dir_all(&base);
}

// ==================== C1：结果计数恒等于输入数 ====================

#[test]
fn yan2_c1_result_count_equals_input_count() {
    let base = scratch("c1");
    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();

    let inputs: Vec<PathBuf> = (0..12)
        .map(|i| {
            let p = base.join(format!("c{i}.ncm"));
            let audio: Vec<u8> = if i % 6 == 5 { vec![0u8; 256] } else { minimal_flac() };
            // 全零音频**不声明** format → 三级判定皆失败 → 规划阶段 UnknownFormat
            let declared = if i % 6 == 5 { None } else { Some("flac") };
            std::fs::write(&p, craft_ncm(&audio, &format!("曲{i}"), "QA", declared)).unwrap();
            p
        })
        .collect();

    let s = musicforge_cli::run(cfg(inputs.clone(), &out, "{title}"));
    assert_eq!(
        s.results.len(),
        inputs.len(),
        "结果数必须恒等于输入数（锁中毒 / 静默丢弃会破坏本式）"
    );
    assert_eq!(
        s.ok + s.failed + s.skipped + s.cancelled,
        inputs.len(),
        "四类计数之和必须等于输入数"
    );
    assert!(s.failed >= 1, "全零音频应判 Failed，实际 failed={}", s.failed);

    let _ = std::fs::remove_dir_all(&base);
}

// ==================== G3：结构镜像回归 ====================

#[test]
fn yan2_g3_expanded_inputs_mirror_source_tree() {
    let base = scratch("g3");
    let src = base.join("src");
    let out = base.join("out");
    let sub = src.join("子目录").join("更深");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::create_dir_all(src.join("顶层")).unwrap();

    let blob = craft_ncm(&minimal_flac(), "镜像", "QA", Some("flac"));
    std::fs::write(src.join("a.ncm"), &blob).unwrap();
    std::fs::write(src.join("顶层").join("b.ncm"), &blob).unwrap();
    std::fs::write(sub.join("c.ncm"), &blob).unwrap();

    let expanded = vec![
        (src.join("a.ncm"), Some(src.clone())),
        (src.join("顶层").join("b.ncm"), Some(src.clone())),
        (sub.join("c.ncm"), Some(src.clone())),
    ];
    let s = musicforge_cli::run_with_progress_expanded(expanded, cfg(vec![], &out, "{title}"), |_| {});
    assert_eq!(s.results.len(), 3);
    assert_eq!(s.failed, 0, "不应有失败：{:?}", s.results.iter().map(|r| &r.reason).collect::<Vec<_>>());

    let mut got: Vec<String> = s
        .results
        .iter()
        .filter_map(|r| r.output.as_ref())
        .map(|p| p.strip_prefix(&out).unwrap().to_string_lossy().replace('\\', "/"))
        .collect();
    got.sort();

    // 三个源文件渲染出的文件名相同（"镜像.flac"），但落在不同子目录 →
    // 去重键是**完整路径**，故不追加 " (n)"，而是各自镜像到源目录结构下。
    let mut expected = vec![
        "镜像.flac".to_string(),
        "顶层/镜像.flac".to_string(),
        "子目录/更深/镜像.flac".to_string(),
    ];
    expected.sort();
    assert_eq!(got, expected, "输出文件必须按源目录树分镜像，而非平铺去重");

    // 结构镜像：逐条核对「源相对路径的父目录」映射到同名输出子目录
    let mut mirrored: Vec<String> = s
        .results
        .iter()
        .map(|r| {
            let rel_src = r.source.strip_prefix(&src).unwrap();
            let rel_out = r.output.as_ref().unwrap().strip_prefix(&out).unwrap();
            format!(
                "{} -> {}",
                rel_src.parent().unwrap().display().to_string().replace('\\', "/"),
                rel_out.parent().unwrap().display().to_string().replace('\\', "/")
            )
        })
        .collect();
    mirrored.sort();
    let mut expect_mirror = vec![
        " -> ".to_string(),
        "顶层 -> 顶层".to_string(),
        "子目录/更深 -> 子目录/更深".to_string(),
    ];
    expect_mirror.sort();
    assert_eq!(mirrored, expect_mirror, "输出目录结构必须逐层镜像源目录树");
}
