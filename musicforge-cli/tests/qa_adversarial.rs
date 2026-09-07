//! musicforge-cli QA 对抗性测试（第一轮，证伪优先）：
//! 模板路径逃逸 / 空音频端到端 / 取消计数完整性 / 并发同目标完整性 / Unicode 端到端 / 真实样本。
//!
//! 全部内容自构造（内嵌 NCM 编码器与 make_ncm.py 对拍），无版权材料。
//! 真实样本测试标注 #[ignore]：仅本地运行验证，输出留在临时目录，不入 git。

use std::path::Path;
use std::process::Command;

use musicforge_cli::{
    run, run_with_progress, run_with_progress_expanded, BatchConfig, CancelToken,
};
use musicforge_core::Decoder;

// ---------- 自建 NCM 编码器（与 musicforge-core/tests/qa_adversarial.rs 同源） ----------

mod ncm {
    use aes::cipher::{generic_array::GenericArray, BlockEncrypt, KeyInit};
    use aes::Aes128;

    pub const MAGIC: &[u8; 8] = b"CTENFDAM";

    pub fn aes_ecb_encrypt(key: &[u8; 16], data: &[u8]) -> Vec<u8> {
        let cipher = Aes128::new(GenericArray::from_slice(key));
        let pad = 16 - data.len() % 16;
        let mut out = data.to_vec();
        out.extend(std::iter::repeat_n(pad as u8, pad));
        for chunk in out.as_chunks_mut::<16>().0 {
            cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
        }
        out
    }

    pub fn rc4_ksa(key: &[u8]) -> [u8; 256] {
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

    pub fn ncm_crypt(data: &[u8], box_: &[u8; 256]) -> Vec<u8> {
        let mut out = data.to_vec();
        for (i, b) in out.iter_mut().enumerate() {
            let j = (i + 1) & 0xff;
            *b ^= box_[(box_[j] as usize + box_[(box_[j] as usize + j) & 0xff] as usize) & 0xff];
        }
        out
    }

    pub fn build_meta_raw(music_name: &str, album: &str, fmt: &str) -> Vec<u8> {
        use base64::Engine as _;
        let meta_json = serde_json::json!({
            "musicId": 1, "musicName": music_name,
            "artist": [["测试歌手", 0]],
            "albumId": 1, "album": album,
            "albumPic": "", "bitrate": 320000, "duration": 1000,
            "format": fmt,
        })
        .to_string();
        let inner = aes_ecb_encrypt(
            &musicforge_core::META_KEY,
            format!("music:{meta_json}").as_bytes(),
        );
        let full = format!(
            "163 key(Don't modify):{}",
            base64::engine::general_purpose::STANDARD.encode(inner)
        );
        full.bytes().map(|b| b ^ 0x63).collect()
    }

    pub fn build_key_data(rc4_key: &[u8]) -> Vec<u8> {
        let plain = [b"neteasecloudmusic".as_slice(), rc4_key].concat();
        aes_ecb_encrypt(&musicforge_core::CORE_KEY, &plain)
            .iter()
            .map(|b| b ^ 0x64)
            .collect()
    }

    pub fn standard_ncm(
        audio: &[u8],
        music_name: &str,
        album: &str,
        fmt: &str,
        cover: &[u8],
    ) -> Vec<u8> {
        let rc4_key: Vec<u8> = (0..112).map(|i| (i * 7 + 13) as u8).collect();
        let key_data = build_key_data(&rc4_key);
        let meta_raw = build_meta_raw(music_name, album, fmt);
        let audio_enc = ncm_crypt(audio, &rc4_ksa(&rc4_key));
        let mut head = Vec::new();
        head.extend_from_slice(MAGIC);
        head.extend_from_slice(&[0x01, 0x69]);
        head.extend_from_slice(&(key_data.len() as u32).to_le_bytes());
        head.extend_from_slice(&key_data);
        head.extend_from_slice(&(meta_raw.len() as u32).to_le_bytes());
        head.extend_from_slice(&meta_raw);
        let crc = crc32fast::hash(&head);
        head.extend_from_slice(&crc.to_le_bytes());
        head.push(0x01);
        head.extend_from_slice(&(cover.len() as u32).to_le_bytes());
        head.extend_from_slice(&(cover.len() as u32).to_le_bytes());
        head.extend_from_slice(cover);
        head.extend_from_slice(&audio_enc);
        head
    }

    pub fn minimal_flac() -> Vec<u8> {
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
        out.extend_from_slice(&(info.len() as u32).to_be_bytes()[1..]);
        out.extend_from_slice(&info);
        out.extend_from_slice(&[0u8; 512]);
        out
    }
}

fn sha256_hex(p: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(p).unwrap();
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h).unwrap();
    h.finalize().iter().map(|x| format!("{x:02x}")).collect()
}

fn cfg(inputs: Vec<std::path::PathBuf>, out: &Path, template: &str) -> BatchConfig {
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

// ============ G3 回归：expanded 输入保留源目录树（CLI/GUI 一致性）============

/// GUI 经 IPC 传 (path, root) 对 → 自定义输出目录下源目录树必须被镜像，
/// 与 CLI `-r` 目录输入行为一致（修 collect_files 丢 root 的不一致）。
#[test]
fn expanded_inputs_mirror_source_tree() {
    let src = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let root = src.path();
    std::fs::create_dir_all(root.join("sub")).unwrap();
    let a = root.join("song_a.ncm");
    let b = root.join("sub").join("song_b.ncm");
    std::fs::write(
        &a,
        ncm::standard_ncm(&ncm::minimal_flac(), "曲A", "专辑A", "flac", b""),
    )
    .unwrap();
    std::fs::write(
        &b,
        ncm::standard_ncm(&ncm::minimal_flac(), "曲B", "专辑B", "flac", b""),
    )
    .unwrap();

    let pairs = vec![(a, Some(root.to_path_buf())), (b, Some(root.to_path_buf()))];
    let summary = run_with_progress_expanded(pairs, cfg(vec![], out.path(), "{title}"), |_| {});
    assert_eq!(
        summary.ok,
        2,
        "两文件都应成功: {:?}",
        summary
            .results
            .iter()
            .map(|r| (r.status, r.reason.clone()))
            .collect::<Vec<_>>()
    );
    assert!(
        out.path().join("曲A.flac").exists(),
        "根下文件 → 输出根（无相对父目录）"
    );
    assert!(
        out.path().join("sub").join("曲B.flac").exists(),
        "子目录文件 → 镜像 out/sub/（G3 核心断言）"
    );
}

// ============ 攻击 1：模板路径逃逸（值注入 + 模板注入）============

#[test]
fn metadata_path_escape_is_contained() {
    let src = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let out_root = out.path().canonicalize().unwrap();

    for (i, title) in [
        "../../evil",
        "..",
        "...",
        "a/../../b",
        "\\\\server\\share\\x",
        "C:\\Windows\\evil",
        "..\\..\\..\\system32",
    ]
    .iter()
    .enumerate()
    {
        let blob = ncm::standard_ncm(&ncm::minimal_flac(), title, "专辑", "flac", b"");
        std::fs::write(src.path().join(format!("s{i}.ncm")), &blob).unwrap();
    }

    let summary = run(cfg(vec![src.path().to_path_buf()], out.path(), "{title}"));
    assert_eq!(
        summary.ok, 7,
        "全部应成功（逃逸被清洗，不是失败）: {:?}",
        summary.results
    );
    for r in &summary.results {
        let o = r.output.as_ref().unwrap().canonicalize().unwrap();
        assert!(
            o.starts_with(&out_root),
            "输出逃逸出输出目录: {:?}",
            r.output
        );
        let has_parent_dir = r
            .output
            .as_ref()
            .unwrap()
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir));
        assert!(!has_parent_dir, "输出路径含 .. 段: {:?}", r.output);
    }
}

#[test]
fn template_itself_with_dotdot_segments_is_contained() {
    let src = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let out_root = out.path().canonicalize().unwrap();
    let blob = ncm::standard_ncm(&ncm::minimal_flac(), "测试曲目", "专辑", "flac", b"");
    std::fs::write(src.path().join("a.ncm"), &blob).unwrap();

    for template in [
        "../{title}",
        "{title}/../x",
        "..\\{title}",
        "{artist}/{title}/..",
    ] {
        let summary = run(cfg(vec![src.path().to_path_buf()], out.path(), template));
        assert_eq!(summary.ok, 1, "模板 {template:?} 应成功");
        let o = summary.results[0]
            .output
            .as_ref()
            .unwrap()
            .canonicalize()
            .unwrap();
        assert!(
            o.starts_with(&out_root),
            "模板 {template:?} 导致逃逸: {:?}",
            summary.results[0].output
        );
    }
    // 清理本轮产物，避免影响断言计数
    for e in std::fs::read_dir(out.path()).unwrap().flatten() {
        let _ = std::fs::remove_file(e.path());
    }
}

// ============ 攻击 2（CLI 端到端）：空音频 —— Failed + NCM-EMPTY-AUDIO，无产物 ============

#[test]
fn empty_audio_cli_fails_with_error_code() {
    let src = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let blob = ncm::standard_ncm(b"", "空音频", "专辑", "flac", b"");
    std::fs::write(src.path().join("empty.ncm"), &blob).unwrap();

    let summary = run(cfg(vec![src.path().to_path_buf()], out.path(), "{title}"));
    assert_eq!(summary.failed, 1, "空音频必须失败");
    assert_eq!(summary.ok, 0);
    let r = &summary.results[0];
    assert!(
        r.reason
            .as_deref()
            .unwrap_or("")
            .contains("NCM-EMPTY-AUDIO"),
        "失败原因应含错误码 NCM-EMPTY-AUDIO: {:?}",
        r.reason
    );
    assert!(
        std::fs::read_dir(out.path()).unwrap().next().is_none(),
        "不得产出 0 字节文件"
    );
}

// ============ 攻击 7：取消后结果计数完整性 ============

#[test]
fn cancel_accounting_is_complete() {
    let src = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let blob = ncm::standard_ncm(&ncm::minimal_flac(), "取消计数", "专辑", "flac", b"");
    for i in 0..20 {
        std::fs::write(src.path().join(format!("t{i:02}.ncm")), &blob).unwrap();
    }
    let token = CancelToken::new();
    let cfg = BatchConfig {
        inputs: vec![src.path().to_path_buf()],
        out_dir: Some(out.path().to_path_buf()),
        recursive: false,
        skip_existing: false,
        jobs: 1,
        template: "{title}".to_string(),
        dry_run: false,
        manifest: None,
        cancel: Some(token.clone()),
    };
    let seen = std::sync::atomic::AtomicUsize::new(0);
    let summary = run_with_progress(cfg, |_r| {
        let n = seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n >= 1 {
            token.cancel();
        }
    });
    assert_eq!(summary.ok, 2, "取消前已开工 2 个完成");
    assert_eq!(summary.cancelled, 18, "其余全部标记取消");
    // 硬断言：四类计数之和 == 总输入数，无静默丢失
    assert_eq!(
        summary.ok + summary.skipped + summary.cancelled + summary.failed,
        20,
        "结果计数必须完整覆盖全部输入"
    );
}

// ============ 攻击 8：并发同目标 —— 去重后无覆盖、内容与单线程一致 ============

#[test]
fn concurrent_same_target_dedup_and_integrity() {
    let src = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let ref_dir = tempfile::tempdir().unwrap();
    let blob = ncm::standard_ncm(&ncm::minimal_flac(), "并发目标", "专辑", "flac", b"");
    for name in ["a.ncm", "b.ncm", "c.ncm", "d.ncm"] {
        std::fs::write(src.path().join(name), &blob).unwrap();
    }

    // 单线程参考转换（内容基准）
    let ref_summary = run(cfg(
        vec![src.path().join("a.ncm")],
        ref_dir.path(),
        "{title}",
    ));
    assert_eq!(ref_summary.ok, 1);
    let ref_sha = sha256_hex(ref_summary.results[0].output.as_ref().unwrap());

    // 4 线程并发同渲染名
    let mut cfg4 = cfg(vec![src.path().to_path_buf()], out.path(), "{title}");
    cfg4.jobs = 4;
    let summary = run(cfg4);
    assert_eq!(summary.ok, 4, "4 个都成功: {:?}", summary.results);

    let mut outputs: Vec<_> = summary
        .results
        .iter()
        .filter_map(|r| r.output.clone())
        .collect();
    outputs.sort();
    assert_eq!(outputs.len(), 4, "4 个互不相同的目标");
    let names: Vec<_> = outputs
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert!(
        names.contains(&"并发目标.flac".to_string()),
        "渲染名: {names:?}"
    );
    assert!(
        names.contains(&"并发目标 (2).flac".to_string()),
        "去重后缀: {names:?}"
    );
    assert!(
        names.contains(&"并发目标 (3).flac".to_string()),
        "去重后缀: {names:?}"
    );
    assert!(
        names.contains(&"并发目标 (4).flac".to_string()),
        "去重后缀: {names:?}"
    );
    for p in &outputs {
        assert_eq!(
            sha256_hex(p),
            ref_sha,
            "并发输出必须与单线程逐字节一致: {p:?}"
        );
    }
}

// ============ 攻击 5（CLI 端到端）：Unicode 恶意曲名 ============

#[test]
fn unicode_title_end_to_end() {
    let src = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let blob = ncm::standard_ncm(
        &ncm::minimal_flac(),
        "🎨🎸\u{202E}exe\u{200B}曲",
        "专辑",
        "flac",
        b"",
    );
    std::fs::write(src.path().join("u.ncm"), &blob).unwrap();

    let summary = run(cfg(vec![src.path().to_path_buf()], out.path(), "{title}"));
    assert_eq!(summary.ok, 1, "Unicode 曲名应成功: {:?}", summary.results);
    let name = summary.results[0]
        .output
        .as_ref()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert!(!name.contains(['<', '>', ':', '"', '/', '\\', '|', '?', '*']));
}

// ============ 攻击（第二轮回归预置）：skip_existing 完整性标记 ============

#[test]
fn skip_existing_detects_corrupted_output() {
    let src = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let blob = ncm::standard_ncm(&ncm::minimal_flac(), "损坏检测", "专辑", "flac", b"");
    std::fs::write(src.path().join("c.ncm"), &blob).unwrap();

    let summary = run(cfg(vec![src.path().to_path_buf()], out.path(), "{title}"));
    assert_eq!(summary.ok, 1);
    let target = summary.results[0].output.as_ref().unwrap().clone();

    // 同尺寸篡改输出文件一个字节（保持大小不变）
    let mut data = std::fs::read(&target).unwrap();
    let mid = data.len() / 2;
    data[mid] ^= 0xff;
    std::fs::write(&target, &data).unwrap();
    assert_eq!(std::fs::metadata(&target).unwrap().len(), data.len() as u64);

    let mut second = cfg(vec![src.path().to_path_buf()], out.path(), "{title}");
    second.skip_existing = true;
    let summary2 = run(second);
    assert_eq!(
        summary2.ok, 1,
        "输出已损坏（同尺寸）必须重转，不得凭 size 一致就 Skipped: {:?}",
        summary2.results
    );
    assert_eq!(summary2.skipped, 0);
}

// ============ 攻击 9：真实样本端到端（仅本地，#[ignore]）============

#[test]
#[ignore = "真实样本仅本地验证；输出留在临时目录，不入 git、不入 CI"]
fn real_sample_end_to_end() {
    let src = Path::new(r"C:\WorkBuddy\NCM\ncmdump-cpp\test\test.ncm");
    assert!(src.exists(), "真实样本不存在: {}", src.display());

    // 1) Decoder 直检：audio_len 与元数据
    let dec = Decoder::open(src).unwrap();
    assert_eq!(dec.audio_len(), 155_727, "真实样本 audio_len");
    let md = dec.metadata().expect("真实样本应有元数据");
    assert_eq!(md.name.as_deref(), Some("贝贝"));
    assert_eq!(md.artist.as_deref(), Some("李荣浩"));
    assert_eq!(md.album.as_deref(), Some("耳朵"));

    // 2) CLI 端到端
    let out = tempfile::tempdir().unwrap();
    let summary = run(cfg(
        vec![src.to_path_buf()],
        out.path(),
        "{artist} - {title}",
    ));
    assert_eq!(summary.ok, 1, "CLI 处理真实样本: {:?}", summary.results);
    let target = summary.results[0].output.as_ref().unwrap().clone();
    assert_eq!(target.extension().and_then(|e| e.to_str()), Some("flac"));
    // 输出尺寸 ≥ 裸音频长度（写标签后有开销），且非 0 字节
    let sz = std::fs::metadata(&target).unwrap().len();
    assert!(sz >= 155_727, "输出应包含完整音频（{sz} B）");

    // 3) 标签读回验证（lofty）
    use lofty::prelude::*;
    let tagged = lofty::read_from_path(&target).unwrap();
    let tag = tagged
        .primary_tag()
        .or(tagged.first_tag())
        .expect("应存在标签");
    assert_eq!(
        tag.get_string(lofty::tag::ItemKey::TrackTitle),
        Some("贝贝")
    );
    assert_eq!(
        tag.get_string(lofty::tag::ItemKey::TrackArtist),
        Some("李荣浩")
    );
    assert_eq!(
        tag.get_string(lofty::tag::ItemKey::AlbumTitle),
        Some("耳朵")
    );
}

// ============ 附：CLI 二进制冒烟（退出码语义）============

#[test]
fn cli_binary_exit_code_on_bad_input() {
    let exe = env!("CARGO_BIN_EXE_musicforge");
    let src = tempfile::tempdir().unwrap();
    std::fs::write(src.path().join("not_ncm.ncm"), b"garbage data here").unwrap();
    let out = tempfile::tempdir().unwrap();
    let output = Command::new(exe)
        .arg(src.path().join("not_ncm.ncm"))
        .arg("-o")
        .arg(out.path())
        .output()
        .unwrap();
    // 非 ncm 文件在规划阶段失败 → 退出码 1（collect_inputs 会收集 .ncm 后缀）
    assert_eq!(output.status.code(), Some(1), "损坏输入退出码应为 1");
}

/// Windows 大小写不敏感回归测试：两个仅**曲名大小写不同**的文件 → 必须产生 2 个输出，不得静默覆盖。
///
/// 该缺陷在 5701 文件真实库抽样时发现：输入 500 / 输出 499 = 静默数据丢失，且零失败报警
/// （只能通过「输入数 vs 输出数」发现，失败清单是空的）。根因：Windows 文件系统大小写不敏感，
/// 两个仅大小写不同的目标名指向同一实际文件 → 后者覆盖前者。修复：dedup 键在 Windows 上小写化，
/// 使大小写不同的目标名被视为碰撞 → 追加 " (2)" 后缀而非覆盖。
///
/// 注：需要 venv python 构造 fixture（make_ncm 编码器）。若环境无 python，测试会失败并提示。
#[test]
fn case_only_collision_no_silent_overwrite() {
    // 环境守卫：本测试依赖本机 venv python 与仓外 scratch 脚本构造 fixture。
    // CI / 无该环境的机器上跳过（回归语义由本地全量跑覆盖）。
    let py = std::path::Path::new(
        r"C:/Users/Vincent/.workbuddy/binaries/python/envs/default/Scripts/python.exe",
    );
    let script = std::path::Path::new(r"C:/WorkBuddy/NCM/scratch/build_case_fixtures.py");
    if !py.exists() || !script.exists() {
        eprintln!("skip: 缺少本机 venv python 或 scratch/build_case_fixtures.py（CI 环境）");
        return;
    }

    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = std::env::temp_dir().join(format!(
        "musicforge-case-{nanos}-{seq}-{}",
        std::process::id()
    ));
    let src_dir = base.join("src");
    let out_dir = base.join("out");
    std::fs::create_dir_all(&src_dir).expect("创建源目录");

    // 1) 构造两个仅大小写不同的 ncm（音频负载不同，便于检测覆盖）
    let status = std::process::Command::new(py)
        .arg(script)
        .arg(&src_dir)
        .status()
        .expect("venv python 可执行");
    assert!(status.success(), "fixture 构造失败");

    let inputs: Vec<std::path::PathBuf> = std::fs::read_dir(&src_dir)
        .expect("读取源目录")
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension()
                .map(|e| e.eq_ignore_ascii_case("ncm"))
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(inputs.len(), 2, "应有 2 个输入");

    // 2) 批量转换（模板 {title} → 渲染出仅大小写不同的两个目标名）
    let cfg = BatchConfig {
        inputs,
        out_dir: Some(out_dir.clone()),
        recursive: false,
        skip_existing: false,
        jobs: 2,
        template: "{title}".to_string(),
        dry_run: false,
        manifest: None,
        cancel: None,
    };
    let summary = run_with_progress(cfg, |_r| {});
    assert_eq!(summary.failed, 0, "不应有失败项");
    assert_eq!(summary.ok, 2, "两个文件都应成功");

    // 3) 关键断言：磁盘上必须存在 2 个**不同**的输出文件（若只 1 个 = 被静默覆盖）
    let mut outputs: Vec<std::path::PathBuf> = std::fs::read_dir(&out_dir)
        .expect("读取输出目录")
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension()
                .map(|e| e.eq_ignore_ascii_case("flac"))
                .unwrap_or(false)
        })
        .collect();
    outputs.sort();
    assert_eq!(
        outputs.len(),
        2,
        "磁盘上必须有 2 个输出文件（若只 1 个 = 被静默覆盖）"
    );

    let a = std::fs::read(&outputs[0]).expect("读取输出 A");
    let b = std::fs::read(&outputs[1]).expect("读取输出 B");
    assert_ne!(a, b, "两个输出内容应不同（若相同 = 覆盖）");

    let _ = std::fs::remove_dir_all(&base);
}

// ============ 第二轮：落盘成功但标签失败 → 产物必须可见（不得 output=None）============

/// 4 帧 MPEG-1 Layer III / 128kbps / 44100Hz 裸 MP3（无 ID3v2）
fn bare_mp3() -> Vec<u8> {
    let mut v = Vec::new();
    for _ in 0..4 {
        v.extend_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
        v.extend(std::iter::repeat_n(0u8, 417 - 4));
    }
    v
}

/// 元数据声明 `format: "flac"`，但实际负载是裸 MP3。
/// `detect_format` 按硬约束 8 采信声明格式 → 扩展名落为 `.flac` →
/// lofty 按**扩展名**解析失败 → `write_tags` 返回 `NcmError::TagRead`。
///
/// 该错误属**源文件元数据/格式层面**问题，音频已完整落盘 → 按硬约束 11
/// 「绝不因元数据问题失败整个转换」降级为 `Ok` + 告警（主理人拍板）。
/// 修复前不仅判 Failed，还把 output 置 None，产物成了「看不见删不掉的幽灵文件」。
#[test]
fn tag_read_failure_degrades_to_ok_not_failed() {
    let base = std::env::temp_dir().join(format!("musicforge-untaggable-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let src = base.join("src");
    let out_dir = base.join("out");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&out_dir).unwrap();

    let audio = bare_mp3();
    let blob = ncm::standard_ncm(&audio, "格式不符", "QA", "flac", b"");
    let p = src.join("mismatch.ncm");
    std::fs::write(&p, &blob).unwrap();

    let summary = run(cfg(vec![p.clone()], &out_dir, "{title}"));
    assert_eq!(summary.results.len(), 1, "结果数必须等于输入数");

    let r = &summary.results[0];
    // 断言 1：硬约束 11 —— 元数据层面失败不得拖垮转换
    assert_eq!(
        r.status,
        musicforge_cli::Status::Ok,
        "TagRead（元数据层面）必须降级为 Ok，不得判整文件 Failed"
    );
    // 断言 2：产物必须可见
    let out_path = r.output.as_ref().expect("必须如实带出输出路径");
    assert!(
        out_path.exists(),
        "报出的输出路径必须真实存在：{out_path:?}"
    );
    // 断言 3：产物必须与源音频**逐字节一致**（解密本身是对的）
    assert_eq!(
        std::fs::read(out_path).unwrap(),
        audio,
        "落盘内容必须等于解密后的原始音频"
    );
    // 断言 4：reason 必须带稳定错误码与「音频已完整导出」语义
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
    // 断言 5：降级后不计入 failed，退出码不受影响
    assert_eq!(summary.failed, 0, "TagRead 降级后不得计入 failed");
    assert_eq!(summary.ok, 1);
    assert_eq!(summary.exit_code(), 0, "元数据问题不得让退出码变成 1");

    let _ = std::fs::remove_dir_all(&base);
}

/// 拍板 1 的**反向约束**：降级必须是**按错误变体精确判定**的，
/// 绝不能退化成「只要产物已落盘就一律判 Ok」的懒规则。
///
/// 构造：落盘成功、打标签也成功，但随后写完整性标记（sidecar）失败
/// （把 sidecar 路径预先占成一个目录 → `std::fs::write` 必然失败）。
/// 此时 `produced.is_some()` 同样成立，但错误变体不是 `TagRead` → 必须仍为 `Failed`。
///
/// 这条与 `tag_read_failure_degrades_to_ok_not_failed` 成对：
/// 两条用例的**唯一自变量是错误变体**，因变量（Ok / Failed）必须相反，
/// 从而把「变体判定」钉死，防止后人把降级改宽。
#[test]
fn non_tagread_post_dump_failure_stays_failed_even_when_product_exists() {
    let dir = tempfile::tempdir().unwrap();
    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&out_dir).unwrap();

    // 预先把 sidecar 路径占成目录，使「写完整性标记」这一步必然失败。
    // 目标名由模板 {title} + 扩展名确定，可精确预测。
    std::fs::create_dir_all(out_dir.join("标记失败.mp3.musicforge.json")).unwrap();

    let blob = ncm::standard_ncm(&bare_mp3(), "标记失败", "QA", "mp3", b"");
    let src = dir.path().join("s.ncm");
    std::fs::write(&src, &blob).unwrap();

    let summary = run(cfg(vec![src], &out_dir, "{title}"));
    assert_eq!(summary.results.len(), 1);

    let r = &summary.results[0];
    // 断言 1：非 TagRead 的落盘后失败 → 仍是 Failed（降级未被改宽）
    assert_eq!(
        r.status,
        musicforge_cli::Status::Failed,
        "非 TagRead 错误不得被降级为 Ok"
    );
    // 断言 2：产物仍然如实可见（B8 的语义与本条正交，不因改回 Failed 而回退）
    let out_path = r.output.as_ref().expect("产物已落盘就必须带出路径");
    assert!(out_path.exists(), "报出的输出路径必须真实存在");
    // 断言 3：失败原因必须区分于 TagRead 降级文案，二者不可混淆
    let reason = r.reason.as_deref().unwrap_or("");
    assert!(
        reason.contains("完整性标记写入失败"),
        "reason 必须说明是哪一步失败，实际：{reason}"
    );
    assert!(
        !reason.contains("NCM-TAG-READ"),
        "非 TagRead 错误不得复用 TagRead 的降级文案：{reason}"
    );
    // 断言 4：计入 failed 且影响退出码（与降级路径形成对照）
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.ok, 0);
    assert_eq!(summary.exit_code(), 1);
}

// ============ 第二轮：并发数上界（硬约束 10「有界」）============

/// `-j` 直接决定 `thread::scope` 起的 OS 线程数。此前只有下界 `max(1)`：
/// `-j 200000` 会尝试创建 20 万个线程，`spawn` 失败即 panic
/// （release 下 `panic = "abort"` 直接崩进程）。
#[test]
fn absurd_jobs_value_is_bounded_not_fatal() {
    let base = std::env::temp_dir().join(format!("musicforge-jobs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let src = base.join("src");
    let out_dir = base.join("out");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&out_dir).unwrap();

    for i in 0..3 {
        let blob = ncm::standard_ncm(&ncm::minimal_flac(), &format!("曲{i}"), "QA", "flac", b"");
        std::fs::write(src.join(format!("f{i}.ncm")), &blob).unwrap();
    }

    let mut c = cfg(
        (0..3).map(|i| src.join(format!("f{i}.ncm"))).collect(),
        &out_dir,
        "{title}",
    );
    c.jobs = 200_000; // 荒谬值：必须被钳制而不是去创建 20 万个线程
    let summary = run(c);
    assert_eq!(
        summary.ok, 3,
        "荒谬并发值不得影响正确性（应被钳制后正常完成）"
    );

    let _ = std::fs::remove_dir_all(&base);
}

// ============ 第二轮：输入目录不可读/无 ncm → 不得「静默成功 0」============

/// `collect_inputs` 对不存在/不可读的路径、以及非 .ncm 文件是**静默跳过**的
/// （`if let Ok(rd)` 无 else 分支）。命令行传错路径时表现为
/// 「汇总：成功 0 / 跳过 0 / 失败 0」+ 退出码 0 —— 用户以为跑完了。
/// 本测试钉住：CLI 必须在 stderr 给出可操作提示（退出码语义不变）。
#[test]
fn nonexistent_input_is_surfaced_not_silently_noop() {
    let exe = env!("CARGO_BIN_EXE_musicforge");
    let missing =
        std::env::temp_dir().join(format!("musicforge-does-not-exist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&missing);

    let out = std::process::Command::new(exe)
        .arg(&missing)
        .output()
        .expect("运行 musicforge");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("不存在") || stderr.contains("未发现"),
        "不存在的输入路径必须给出可操作提示，实际 stderr：{stderr}"
    );
}
