//! QA(Yan) 第二轮**独立**验证 —— CLI 层：B8 产物可见性 / 2bd4a41 TagRead 降级 /
//! B9 并发上界 / B10 遍历深度 / B11 静默成功 / 第一轮修复回归（B6·G5·K2·G3）。
//!
//! 独立性：文件内自带一套 NCM 编码器（不同的 RC4 密钥、不同的元数据 JSON），
//! **不 import 实现者 `qa_adversarial.rs` 的 `ncm` 模块**。所有断言按 PRD/硬约束
//! 的语义写，不看实现者的自查结论。

use musicforge_cli::{run, run_with_progress_expanded, BatchConfig, Status};
use musicforge_core::Decoder;
use std::path::{Path, PathBuf};
use std::process::Command;

// ==================== 自建 NCM 编码器（不复用实现者的 ncm 模块）====================

use aes::cipher::{generic_array::GenericArray, BlockEncrypt, KeyInit};
use aes::Aes128;

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

fn rc4_ksa(key: &[u8]) -> [u8; 256] {
    let mut s = [0u8; 256];
    for (i, b) in s.iter_mut().enumerate() {
        *b = i as u8;
    }
    let mut j = 0usize;
    for i in 0..256 {
        j = (j + s[i] as usize + key[i % key.len()] as usize) & 0xff;
        s.swap(i, j);
    }
    s
}

/// NCM 音频区加密：全局偏移密钥流（与 RC4 不同，按字节下标取流）。
fn ncm_stream_xor(data: &[u8], box_: &[u8; 256]) -> Vec<u8> {
    let mut out = data.to_vec();
    for (i, b) in out.iter_mut().enumerate() {
        let j = (i + 1) & 0xff;
        *b ^= box_[(box_[j] as usize + box_[(box_[j] as usize + j) & 0xff] as usize) & 0xff];
    }
    out
}

/// 与实现者刻意不同的 RC4 密钥（同长度 112，不同取值序列）。
fn yan_rc4_key() -> Vec<u8> {
    (0..112).map(|i| ((i * 31 + 7) ^ 0x5a) as u8).collect()
}

fn meta_block(music_name: &str, artist: &str, album: &str, fmt: &str) -> Vec<u8> {
    use base64::Engine as _;
    let json = serde_json::json!({
        "musicId": 42,
        "musicName": music_name,
        "artist": [[artist, 0]],
        "albumId": 7,
        "album": album,
        "albumPic": "",
        "bitrate": 320000,
        "duration": 1000,
        "format": fmt,
    })
    .to_string();
    let inner = aes_ecb_encrypt(
        &musicforge_core::META_KEY,
        format!("music:{json}").as_bytes(),
    );
    let full = format!(
        "163 key(Don't modify):{}",
        base64::engine::general_purpose::STANDARD.encode(inner)
    );
    full.bytes().map(|b| b ^ 0x63).collect()
}

/// 构造一个 NCM 文件。`rc4_key` 可传空切片以触发 `EmptyKey`（B6 回归）。
fn encode_ncm(
    audio: &[u8],
    music_name: &str,
    artist: &str,
    album: &str,
    fmt: &str,
    cover: &[u8],
    rc4_key: &[u8],
) -> Vec<u8> {
    let key_plain = [b"neteasecloudmusic".as_slice(), rc4_key].concat();
    let key_data: Vec<u8> = aes_ecb_encrypt(&musicforge_core::CORE_KEY, &key_plain)
        .iter()
        .map(|b| b ^ 0x64)
        .collect();
    let meta_raw = meta_block(music_name, artist, album, fmt);
    let audio_enc = if rc4_key.is_empty() {
        audio.to_vec()
    } else {
        ncm_stream_xor(audio, &rc4_ksa(rc4_key))
    };

    let mut head = Vec::new();
    head.extend_from_slice(&musicforge_core::MAGIC);
    head.extend_from_slice(&[0x01, 0x69]);
    head.extend_from_slice(&(key_data.len() as u32).to_le_bytes());
    head.extend_from_slice(&key_data);
    head.extend_from_slice(&(meta_raw.len() as u32).to_le_bytes());
    head.extend_from_slice(&meta_raw);
    // CRC32 覆盖 [0, 此处偏移)
    let crc = crc32fast::hash(&head);
    head.extend_from_slice(&crc.to_le_bytes());
    head.push(0x01); // 分隔符
    head.extend_from_slice(&(cover.len() as u32).to_le_bytes()); // coverLen1
    head.extend_from_slice(&(cover.len() as u32).to_le_bytes()); // coverLen2
    head.extend_from_slice(cover);
    head.extend_from_slice(&audio_enc);
    head
}

/// 一帧 MPEG-1 Layer III / 128kbps / 44100Hz / 单声道（与实现者的 0x64 立体声区分）。
fn bare_mp3(frames: usize) -> Vec<u8> {
    let mut v = Vec::new();
    for _ in 0..frames {
        v.extend_from_slice(&[0xff, 0xfb, 0x90, 0xc4]);
        v.extend(std::iter::repeat_n(0x5a_u8, 417 - 4));
    }
    v
}

fn cfg(inputs: Vec<PathBuf>, out: &Path, template: &str) -> BatchConfig {
    BatchConfig {
        inputs,
        out_dir: Some(out.to_path_buf()),
        recursive: false,
        skip_existing: false,
        jobs: 2,
        template: template.to_string(),
        dry_run: false,
        manifest: None,
        cancel: None,
    }
}

// ==================== 先验自检 ====================

/// 我手搓的编码器必须能被 musicforge-core 原样解回（否则后续所有用例都无意义）。
#[test]
fn yan_selftest_my_ncm_encoder_roundtrips() {
    let dir = tempfile::tempdir().unwrap();
    let audio = bare_mp3(8);
    let blob = encode_ncm(
        &audio,
        "自检曲",
        "自检歌手",
        "自检专辑",
        "mp3",
        b"",
        &yan_rc4_key(),
    );
    let p = dir.path().join("selftest.ncm");
    std::fs::write(&p, &blob).unwrap();

    // 注意 `dump(out_dir)` 的形参是**输出目录**而非文件路径：
    // 它会 `create_dir_all(dir)` 再落到 `dir/<stem>.<ext>`。
    // 这里传目录，产物名为 `selftest.mp3`（stem 取自源文件名）。
    let out_dir = dir.path().join("dumpdir");
    let meta = {
        let mut dec = Decoder::open(&p).expect("我编码的 NCM 必须能被打开");
        let m = dec.metadata().cloned();
        let produced = dec.dump(Some(&out_dir)).expect("dump 应成功");
        assert_eq!(
            produced,
            out_dir.join("selftest.mp3"),
            "dump 应把产物落在指定目录下"
        );
        m
    };
    assert_eq!(
        std::fs::read(out_dir.join("selftest.mp3")).unwrap(),
        audio,
        "往返后音频字节必须与原文逐字节一致"
    );
    let m = meta.expect("应解析出元数据");
    assert_eq!(m.name.as_deref(), Some("自检曲"));
    assert_eq!(m.artist.as_deref(), Some("自检歌手"));
    assert_eq!(m.album.as_deref(), Some("自检专辑"));
    assert_eq!(m.format.as_deref(), Some("mp3"));
}

/// 打标签**成功**时，产物 = ID3v2 头部 + 原始音频。
/// 因此只能断言「原始音频是产物的**尾部**」，不能断言整体逐字节相等。
/// （反过来：打标签**失败**的降级用例里没有 ID3v2，才可以断言整体相等。）
fn assert_audio_is_suffix_of(path: &Path, audio: &[u8], what: &str) {
    let disk = std::fs::read(path).unwrap();
    assert!(
        disk.len() >= audio.len(),
        "{what}：产物({} 字节) 竟短于源音频({} 字节)",
        disk.len(),
        audio.len()
    );
    let tail = &disk[disk.len() - audio.len()..];
    assert_eq!(tail, audio, "{what}：产物尾部必须等于原始音频（逐字节）");
}

// ==================== B8 / 2bd4a41 正向：TagRead 降级 ====================

/// 元数据声明 `format:"flac"`、负载实为裸 MP3 → dump 成功、lofty 按扩展名解析失败
/// → `NcmError::TagRead`。硬约束 11「绝不因元数据问题失败整个转换」→ 降级为 Ok。
///
/// 断言：产物可见 + 内容逐字节正确 + reason 是告警式（含码/已导出/建议）+ 退出码 0。
#[test]
fn yan_tagread_degrades_to_ok_with_visible_product() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&out_dir).unwrap();

    let audio = bare_mp3(6);
    let blob = encode_ncm(
        &audio,
        "降级曲",
        "QA",
        "QA专辑",
        "flac",
        b"",
        &yan_rc4_key(),
    );
    let p = src.join("mismatch.ncm");
    std::fs::write(&p, &blob).unwrap();

    let summary = run(cfg(vec![p.clone()], &out_dir, "{title}"));
    assert_eq!(summary.results.len(), 1, "结果数必须等于输入数");

    let r = &summary.results[0];
    assert_eq!(r.status, Status::Ok, "TagRead 属元数据层面，必须降级为 Ok");

    // 产物可见性（B8 核心）
    let out_path = r.output.as_ref().expect("必须如实带出输出路径");
    assert!(
        out_path.exists(),
        "报出的输出路径必须真实存在：{out_path:?}"
    );

    // 逐字节相等 —— 证明「降级」只是状态判定，解密本身没被破坏
    assert_eq!(
        std::fs::read(out_path).unwrap(),
        audio,
        "落盘内容必须等于解密后的原始音频（逐字节）"
    );

    // reason 三要素
    let reason = r.reason.as_deref().unwrap_or("");
    assert!(
        reason.contains("NCM-TAG-READ"),
        "reason 必须带错误码：{reason}"
    );
    assert!(
        reason.contains("音频已完整导出"),
        "reason 必须说明音频已导出：{reason}"
    );
    assert!(
        reason.contains("建议"),
        "reason 必须给出可操作建议：{reason}"
    );

    assert_eq!(summary.failed, 0, "降级项不得计入 failed");
    assert_eq!(summary.ok, 1);
    assert_eq!(summary.exit_code(), 0, "元数据问题不得把退出码洗成 1");
}

// ==================== 2bd4a41 反向：非 TagRead 的落盘后失败仍为 Failed ====================

/// 反向约束（本轮最关键的一条）：降级必须**按错误变体精确判定**，
/// 绝不能退化成「产物已落盘就一律判 Ok」。
///
/// 构造：一切正常（mp3 声明 + 裸 MP3 负载），但把 sidecar 路径预先占成**目录**，
/// 使完整性标记写入失败。此时 `produced.is_some()` 同样成立，但错误变体不是 TagRead。
#[test]
fn yan_non_tagread_post_dump_failure_stays_failed() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&out_dir).unwrap();

    let audio = bare_mp3(6);
    let blob = encode_ncm(
        &audio,
        "真失败曲",
        "QA",
        "QA专辑",
        "mp3",
        b"",
        &yan_rc4_key(),
    );
    let p = src.join("boom.ncm");
    std::fs::write(&p, &blob).unwrap();

    // sidecar = <target>.musicforge.json；target = out_dir/真失败曲.mp3
    let sidecar = out_dir.join("真失败曲.mp3.musicforge.json");
    std::fs::create_dir_all(&sidecar).expect("把 sidecar 路径占成目录");

    let summary = run(cfg(vec![p.clone()], &out_dir, "{title}"));
    let r = &summary.results[0];

    assert_eq!(
        r.status,
        Status::Failed,
        "非 TagRead 的落盘后失败必须保持 Failed，不得被降级规则误吞"
    );
    // 产物仍需可见（B8 的另一半：Failed 也不得让产物变成幽灵文件）
    let out_path = r.output.as_ref().expect("失败时也必须如实带出产物路径");
    assert!(out_path.exists(), "失败时产物也应可见：{out_path:?}");
    // 此路径下打标签**成功**（随后才写 sidecar 失败）⇒ 产物 = ID3v2 + 音频
    assert_audio_is_suffix_of(out_path, &audio, "sidecar 写入失败时");

    let reason = r.reason.as_deref().unwrap_or("");
    assert!(
        !reason.contains("NCM-TAG-READ"),
        "非 TagRead 失败的 reason 不得出现 NCM-TAG-READ：{reason}"
    );
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.exit_code(), 1, "真实失败必须让退出码为 1");
}

// ==================== 降级项不得把退出码洗成 0 ====================

/// 混合批次：1 个降级项 + 1 个真实失败项 →
/// 断言降级项仍为 Ok、真实失败项仍为 Failed、**退出码为 1**（降级不得洗码）。
#[test]
fn yan_mixed_degraded_and_failed_batch_exit_code_is_1() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&out_dir).unwrap();

    let audio = bare_mp3(6);

    // A：降级项（声明 flac / 实为 MP3）
    let pa = src.join("degrade.ncm");
    std::fs::write(
        &pa,
        encode_ncm(&audio, "降级项", "QA", "AL", "flac", b"", &yan_rc4_key()),
    )
    .unwrap();

    // B：真实失败项（sidecar 被占成目录）
    let pb = src.join("real_fail.ncm");
    std::fs::write(
        &pb,
        encode_ncm(&audio, "真失败项", "QA", "AL", "mp3", b"", &yan_rc4_key()),
    )
    .unwrap();
    std::fs::create_dir_all(out_dir.join("真失败项.mp3.musicforge.json")).unwrap();

    let summary = run(cfg(vec![pa, pb], &out_dir, "{title}"));
    assert_eq!(summary.results.len(), 2, "结果数必须等于输入数");

    let mut degraded = None;
    let mut real = None;
    for r in &summary.results {
        let n = r.source.file_stem().unwrap().to_string_lossy().to_string();
        match n.as_str() {
            "degrade" => degraded = Some(r),
            "real_fail" => real = Some(r),
            _ => panic!("未知源 {n}"),
        }
    }
    let degraded = degraded.expect("应存在降级项结果");
    let real = real.expect("应存在真实失败项结果");

    assert_eq!(degraded.status, Status::Ok, "降级项应仍为 Ok");
    assert_eq!(real.status, Status::Failed, "真实失败项应仍为 Failed");

    assert_eq!(summary.ok, 1);
    assert_eq!(summary.failed, 1);
    assert_eq!(
        summary.exit_code(),
        1,
        "批中存在 1 个真实失败 ⇒ 退出码必须为 1（降级项不得把退出码洗成 0）"
    );
}

// ==================== 降级项的 CSV 语义（已知行为变化，实测确认）====================

/// 实测确认：`export_failures_csv` 按 `status == Failed` 过滤 ⇒ **降级项不进 CSV**。
///
/// 这条只做「行为实测 + 记录」，不预设对错；结论见 QA 报告 §遗留/存疑。
#[test]
fn yan_degraded_item_is_absent_from_failure_csv() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&out_dir).unwrap();

    let audio = bare_mp3(6);
    let pa = src.join("degrade.ncm");
    std::fs::write(
        &pa,
        encode_ncm(&audio, "降级项", "QA", "AL", "flac", b"", &yan_rc4_key()),
    )
    .unwrap();
    let pb = src.join("real_fail.ncm");
    std::fs::write(
        &pb,
        encode_ncm(&audio, "真失败项", "QA", "AL", "mp3", b"", &yan_rc4_key()),
    )
    .unwrap();
    std::fs::create_dir_all(out_dir.join("真失败项.mp3.musicforge.json")).unwrap();

    let summary = run(cfg(vec![pa, pb], &out_dir, "{title}"));
    let csv_path = dir.path().join("failures.csv");
    summary
        .export_failures_csv(&csv_path)
        .expect("CSV 导出应成功");
    let csv = std::fs::read_to_string(&csv_path).unwrap();

    println!("===== 实测 CSV 全文 =====");
    println!("{csv}");
    println!("========================");

    assert!(csv.contains("真失败项"), "真实失败项必须出现在 CSV 中");
    assert!(
        !csv.contains("降级项"),
        "实测确认：降级项（status=Ok）不出现在 --export-failures 的 CSV 中。\n\
         这是 2bd4a41 引入的行为变化：把 CSV 当「待重跑清单」的用法会漏掉降级项。\n\
         完整 CSV 见上方 stdout。"
    );
    // 降级项**不写完整性标记** ⇒ 下次运行会自动重转（这是它不必进 CSV 的理由）
    assert!(
        !out_dir.join("降级项.flac.musicforge.json").exists(),
        "降级项不应写完整性标记（否则下次不会重转，元数据永久缺失）"
    );
}

// ==================== B9：jobs 输入无界 ====================

/// `jobs = 200_000` 必须被钳制到 MAX_JOBS 且**正常完成**（不崩溃、结果完整）。
#[test]
fn yan_jobs_200k_is_bounded_and_completes() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&out_dir).unwrap();

    let audio = bare_mp3(6);
    let n = 8;
    let mut inputs = Vec::new();
    for i in 0..n {
        let p = src.join(format!("j{i}.ncm"));
        std::fs::write(
            &p,
            encode_ncm(
                &audio,
                &format!("曲{i}"),
                "QA",
                "AL",
                "mp3",
                b"",
                &yan_rc4_key(),
            ),
        )
        .unwrap();
        inputs.push(p);
    }

    let mut c = cfg(inputs, &out_dir, "{title}");
    c.jobs = 200_000;
    let summary = run(c);

    assert_eq!(
        summary.results.len(),
        n,
        "结果数必须等于输入数（不得静默丢结果）"
    );
    assert_eq!(
        summary.ok, n,
        "jobs=200000 被钳制后应全部成功，实际 ok={}",
        summary.ok
    );
    assert_eq!(summary.failed, 0);
    assert_eq!(summary.exit_code(), 0);
}

/// `jobs = 0` 也必须被钳制到 ≥1（否则线程池为空 ⇒ 结果为空的静默失败）。
#[test]
fn yan_jobs_zero_is_bounded_to_at_least_one() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&out_dir).unwrap();

    let audio = bare_mp3(4);
    let p = src.join("z.ncm");
    std::fs::write(
        &p,
        encode_ncm(&audio, "零并发", "QA", "AL", "mp3", b"", &yan_rc4_key()),
    )
    .unwrap();

    let mut c = cfg(vec![p], &out_dir, "{title}");
    c.jobs = 0;
    let summary = run(c);
    assert_eq!(summary.ok, 1, "jobs=0 必须被钳制到 ≥1 并正常转换");
    assert_eq!(summary.results.len(), 1);
}

// ==================== B10：目录遍历深度上界 ====================

/// 深度 60（在上界内）→ 文件**必须**被找到并正常转换。
/// 证明深度上界没有破坏正常使用。
#[test]
fn yan_depth_60_is_still_collected() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    let mut deep = root.clone();
    for _ in 0..60 {
        deep = deep.join("d");
    }
    std::fs::create_dir_all(&deep).unwrap();
    let audio = bare_mp3(4);
    std::fs::write(
        deep.join("deep.ncm"),
        encode_ncm(&audio, "浅深曲", "QA", "AL", "mp3", b"", &yan_rc4_key()),
    )
    .unwrap();

    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&out_dir).unwrap();
    let mut c = cfg(vec![root], &out_dir, "{title}");
    c.recursive = true;
    let summary = run(c);

    assert_eq!(summary.results.len(), 1, "深度 60 在上界内，必须被收集到");
    assert_eq!(summary.ok, 1);
}

/// 深度 200（远超上界 64）→ 必须**不栈溢出**、不崩溃；深层文件被跳过是既定取舍。
#[test]
fn yan_depth_200_does_not_overflow_stack() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    // 浅层放一个能成的，深层放一个必然被跳过的
    std::fs::create_dir_all(&root).unwrap();
    let audio = bare_mp3(4);
    std::fs::write(
        root.join("shallow.ncm"),
        encode_ncm(&audio, "浅层曲", "QA", "AL", "mp3", b"", &yan_rc4_key()),
    )
    .unwrap();

    let mut deep = root.clone();
    for _ in 0..200 {
        deep = deep.join("d");
    }
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(
        deep.join("deep.ncm"),
        encode_ncm(&audio, "深层曲", "QA", "AL", "mp3", b"", &yan_rc4_key()),
    )
    .unwrap();

    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&out_dir).unwrap();
    let mut c = cfg(vec![root], &out_dir, "{title}");
    c.recursive = true;

    // 若能走到这一行就说明没有栈溢出 / abort
    let summary = run(c);

    // 浅层必须正常；深层被深度上界跳过（MAX_WALK_DEPTH=64）——这是**静默**跳过
    assert_eq!(summary.ok, 1, "浅层文件必须被正常转换");
    assert_eq!(
        summary.results.len(),
        1,
        "深度 200 的文件被 MAX_WALK_DEPTH=64 静默跳过（既定取舍，无告警）"
    );
    assert_eq!(summary.exit_code(), 0);
}

/// Windows junction 自引用：不构造 junction 的情形下，用 200 层已覆盖上界。
/// 本用例尝试构造真实 junction（需要 `mklink`），不可用时**显式跳过**并打印说明。
#[test]
fn yan_self_referential_junction_if_constructible() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    let target = dir.path().join("real");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    let audio = bare_mp3(4);
    std::fs::write(
        target.join("t.ncm"),
        encode_ncm(&audio, "链接曲", "QA", "AL", "mp3", b"", &yan_rc4_key()),
    )
    .unwrap();

    let link = root.join("self");
    let out = Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(&root)
        .output();
    let created = match &out {
        Ok(o) => o.status.success(),
        Err(_) => false,
    };
    if !created {
        println!(
            "[SKIP] 当前环境无法创建 junction（mklink 不可用或权限不足）。\n\
             该场景已由 `yan_depth_200_does_not_overflow_stack`（200 层普通目录）等价覆盖。"
        );
        return;
    }
    assert!(link.exists(), "junction 应创建成功");

    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&out_dir).unwrap();
    let mut c = cfg(vec![root], &out_dir, "{title}");
    c.recursive = true;
    let summary = run(c);
    println!(
        "自引用 junction 扫描结果: results={} ok={} failed={}（未崩溃即通过）",
        summary.results.len(),
        summary.ok,
        summary.failed
    );

    // 关键：**先删 junction 再删树**，避免 remove_dir_all 顺着链接走到目标
    let _ = Command::new("cmd")
        .args(["/c", "rmdir"])
        .arg(&link)
        .output();
    assert!(
        !link.exists(),
        "junction 必须被移除，否则 tempdir 清理有风险"
    );
}

// ==================== B11：输入静默成功 ====================

/// 不存在的输入路径 → stderr 必须有告警。
#[test]
fn yan_nonexistent_input_is_surfaced() {
    let exe = env!("CARGO_BIN_EXE_musicforge");
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("绝对不存在的路径.ncm");
    let out = dir.path().join("out");

    let output = Command::new(exe)
        .arg(&missing)
        .arg("-o")
        .arg(&out)
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    println!("===== 不存在输入：stderr =====");
    println!("{stderr}");
    println!("exit code = {:?}", output.status.code());

    assert!(
        stderr.contains("不存在") || stderr.contains("未发现"),
        "不存在的输入必须在 stderr 留痕，实际 stderr={stderr:?}"
    );
    // 退出码语义：此处记录实测值（不存在的路径被 collect_inputs 静默跳过 ⇒ 0 结果）
    println!("[实测] 不存在输入时退出码 = {:?}", output.status.code());
}

/// 存在但无 .ncm 的目录 → stderr 必须有告警。
#[test]
fn yan_directory_without_ncm_is_surfaced() {
    let exe = env!("CARGO_BIN_EXE_musicforge");
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), b"not ncm").unwrap();
    std::fs::write(src.join("b.mp3"), b"not ncm").unwrap();
    let out = dir.path().join("out");

    let output = Command::new(exe)
        .arg(&src)
        .arg("-o")
        .arg(&out)
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    println!("===== 无 ncm 目录：stderr =====");
    println!("{stderr}");
    println!("exit code = {:?}", output.status.code());

    assert!(
        stderr.contains("未发现"),
        "零结果必须在 stderr 告警（否则表现为『成功转换 0 个』的静默失败），stderr={stderr:?}"
    );
    println!("[实测] 无 .ncm 目录时退出码 = {:?}", output.status.code());
}

// ==================== 第一轮修复回归（本轮不得改坏）====================

/// B6：空 RC4 密钥 → `Err(EmptyKey)`，码 `NCM-STRUCT-INVALID`，两条解析路径都不 panic。
#[test]
fn yan_regression_b6_empty_key_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let audio = bare_mp3(4);
    let blob = encode_ncm(&audio, "空密钥", "QA", "AL", "mp3", b"", &[]); // 空 RC4 密钥
    let p = dir.path().join("emptykey.ncm");
    std::fs::write(&p, &blob).unwrap();

    // 路径 1：Decoder::open（流式解析）
    let r1 = std::panic::catch_unwind(|| Decoder::open(&p).map(|_| ()));
    assert!(r1.is_ok(), "Decoder::open 不得 panic");
    let e1 = r1.unwrap().expect_err("空 RC4 密钥必须报错");
    assert_eq!(e1.code(), "NCM-STRUCT-INVALID", "错误码不符：{e1}");

    // 路径 2：批处理落盘（不 panic，且产生 Failed 而非假成功）
    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&out_dir).unwrap();
    let summary = run(cfg(vec![p], &out_dir, "{title}"));
    assert_eq!(summary.results.len(), 1);
    assert_eq!(
        summary.results[0].status,
        Status::Failed,
        "空密钥必须 Failed，不得静默成功"
    );
    assert!(
        summary.results[0]
            .reason
            .as_deref()
            .unwrap_or("")
            .contains("NCM-STRUCT-INVALID"),
        "批处理 reason 必须带错误码"
    );
    assert_eq!(summary.exit_code(), 1);
}

/// K2：未知格式（元数据声明 ogg + 无魔数负载）→ `UnknownFormat`，码 `NCM-FORMAT-UNKNOWN`，零产物。
#[test]
fn yan_regression_k2_unknown_format_no_output() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&out_dir).unwrap();

    // 无任何已知魔数的负载
    let payload: Vec<u8> = std::iter::repeat_n(0x11u8, 512).collect();
    let blob = encode_ncm(&payload, "未知格式", "QA", "AL", "ogg", b"", &yan_rc4_key());
    let p = src.join("unknown.ncm");
    std::fs::write(&p, &blob).unwrap();

    let summary = run(cfg(vec![p], &out_dir, "{title}"));
    assert_eq!(summary.results.len(), 1);
    let r = &summary.results[0];
    assert_eq!(r.status, Status::Failed, "未知格式必须 Failed");
    assert!(
        r.reason
            .as_deref()
            .unwrap_or("")
            .contains("NCM-FORMAT-UNKNOWN"),
        "错误码必须是 NCM-FORMAT-UNKNOWN，实际：{:?}",
        r.reason
    );
    assert!(r.output.is_none(), "未知格式不得产生产物");
    // 零产物
    let produced: Vec<_> = std::fs::read_dir(&out_dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some())
        .collect();
    assert!(
        produced.is_empty(),
        "未知格式必须零产物，实际产生了 {produced:?}"
    );
}

/// G3：`run_with_progress_expanded` 在自定义输出目录下镜像源目录树。
#[test]
fn yan_regression_g3_expanded_mirrors_source_tree() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    let sub = root.join("sub").join("deep");
    std::fs::create_dir_all(&sub).unwrap();
    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&out_dir).unwrap();

    let audio = bare_mp3(4);
    let f = sub.join("nested.ncm");
    std::fs::write(
        &f,
        encode_ncm(&audio, "嵌套曲", "QA", "AL", "mp3", b"", &yan_rc4_key()),
    )
    .unwrap();

    let cfg = BatchConfig {
        inputs: Vec::new(),
        out_dir: Some(out_dir.clone()),
        recursive: true,
        skip_existing: false,
        jobs: 2,
        template: "{title}".to_string(),
        dry_run: false,
        manifest: None,
        cancel: None,
    };
    let summary = run_with_progress_expanded(vec![(f, Some(root.clone()))], cfg, |_| {});

    assert_eq!(summary.ok, 1);
    let expected = out_dir.join("sub").join("deep").join("嵌套曲.mp3");
    assert!(
        expected.exists(),
        "自定义输出目录必须镜像源目录树 out/sub/deep/，期望路径 {expected:?} 不存在。\n\
         实际产物：{:?}",
        std::fs::read_dir(&out_dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect::<Vec<_>>()
    );
}

/// G5：`Decoder::open` 的 stream_offset 从 0 起 ⇒ 端到端魔数嗅探能读到真字节。
/// 构造：元数据**不声明**格式（format=""），靠嗅探 MP3 帧同步字判定。
#[test]
fn yan_regression_g5_magic_sniff_reads_real_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&out_dir).unwrap();

    let audio = bare_mp3(6);
    // 元数据不声明 format ⇒ 必须靠魔数嗅探落到 mp3
    let blob = encode_ncm(&audio, "嗅探曲", "QA", "AL", "", b"", &yan_rc4_key());
    let p = src.join("sniff.ncm");
    std::fs::write(&p, &blob).unwrap();

    let summary = run(cfg(vec![p], &out_dir, "{title}"));
    assert_eq!(summary.results.len(), 1);
    let r = &summary.results[0];
    assert_eq!(
        r.status,
        Status::Ok,
        "无声明格式时必须靠魔数嗅探成功，实际：{:?}",
        r.reason
    );
    let out_path = r.output.as_ref().expect("应有产物");
    assert!(
        out_path.extension().unwrap().eq_ignore_ascii_case("mp3"),
        "嗅探结果必须是 mp3，实际扩展名 {:?}",
        out_path.extension()
    );
    // 嗅探成功 ⇒ 打标签也成功 ⇒ 产物 = ID3v2 + 音频
    assert_audio_is_suffix_of(out_path, &audio, "魔数嗅探路径");
}
