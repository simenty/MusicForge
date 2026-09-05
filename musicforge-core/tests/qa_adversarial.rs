//! QA 对抗性测试（第一轮，证伪优先）：空音频 / 截断 / 长度越界 / Unicode 边界 / 保留名 / track 宽度炸弹。
//!
//! 全部内容自构造（含内嵌 NCM 编码器，与 scratch/make_ncm.py 对拍），无任何版权材料。
//! 攻击面设计原则：假设代码有 bug，构造最恶意输入去证明它。

use std::path::{Path, PathBuf};

use musicforge_core::template::render_filename;
use musicforge_core::{Decoder, Metadata, NcmError};

// ---------- 自建 NCM 编码器（字段完全可控的对抗性装配入口） ----------

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
        let inner =
            aes_ecb_encrypt(&musicforge_core::META_KEY, format!("music:{meta_json}").as_bytes());
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

    /// 对抗性装配器：所有长度字段可单独篡改
    pub struct Craft {
        pub key_len_override: Option<u32>,
        pub key_data: Vec<u8>,
        pub meta_len_override: Option<u32>,
        pub meta_raw: Vec<u8>,
        pub cover_len1: u32,
        pub cover: Vec<u8>,
        pub audio_enc: Vec<u8>,
    }

    pub fn assemble(c: Craft) -> Vec<u8> {
        let key_len = c.key_len_override.unwrap_or(c.key_data.len() as u32);
        let meta_len = c.meta_len_override.unwrap_or(c.meta_raw.len() as u32);
        let mut head = Vec::new();
        head.extend_from_slice(MAGIC);
        head.extend_from_slice(&[0x01, 0x69]);
        head.extend_from_slice(&key_len.to_le_bytes());
        head.extend_from_slice(&c.key_data);
        head.extend_from_slice(&meta_len.to_le_bytes());
        head.extend_from_slice(&c.meta_raw);
        let crc = crc32fast::hash(&head);
        head.extend_from_slice(&crc.to_le_bytes());
        head.push(0x01);
        head.extend_from_slice(&c.cover_len1.to_le_bytes());
        head.extend_from_slice(&(c.cover.len() as u32).to_le_bytes());
        head.extend_from_slice(&c.cover);
        head.extend_from_slice(&c.audio_enc);
        head
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
        assemble(Craft {
            key_len_override: None,
            key_data,
            meta_len_override: None,
            meta_raw,
            cover_len1: cover.len() as u32,
            cover: cover.to_vec(),
            audio_enc,
        })
    }

    pub fn minimal_flac() -> Vec<u8> {
        // 最小合法 FLAC：fLaC + STREAMINFO（44100Hz/2ch/16bit，last-block）+ 帧区填充
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

fn write_tmp(dir: &Path, name: &str, blob: &[u8]) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, blob).unwrap();
    p
}

fn meta_with(title: &str) -> Metadata {
    Metadata {
        name: Some(title.to_string()),
        artist: Some("测试歌手".to_string()),
        album: Some("测试专辑".to_string()),
        format: Some("flac".to_string()),
        bitrate: None,
        duration: None,
        track: None,
        album_pic_url: None,
    }
}

fn meta_track(t: u64) -> Metadata {
    Metadata { track: Some(t), ..meta_with("t") }
}

// ============ 攻击 2：空音频 —— 必须报 EmptyAudio，不得产出 0 字节文件 ============

#[test]
fn empty_audio_must_error_not_zero_byte_output() {
    let blob = ncm::standard_ncm(b"", "空音频", "专辑", "flac", b"");
    let tmp = tempfile::tempdir().unwrap();
    let p = write_tmp(tmp.path(), "empty.ncm", &blob);
    let out = tempfile::tempdir().unwrap();

    let result = Decoder::open(&p).and_then(|mut d| d.dump(Some(out.path())));
    let err = result.expect_err("空音频必须报错，不得静默成功");
    assert!(matches!(err, NcmError::EmptyAudio), "应报 EmptyAudio，实际: {err}");
    assert!(
        std::fs::read_dir(out.path()).unwrap().next().is_none(),
        "不得产出 0 字节半成品"
    );
}

// ============ 攻击 2b：coverLen1 越界（指向文件外）—— 不得 0 字节假成功 ============

#[test]
fn cover_len1_beyond_file_must_not_produce_zero_byte_output() {
    let audio = ncm::minimal_flac();
    let rc4_key: Vec<u8> = (0..112).map(|i| (i * 7 + 13) as u8).collect();
    let blob = ncm::assemble(ncm::Craft {
        key_len_override: None,
        key_data: ncm::build_key_data(&rc4_key),
        meta_len_override: None,
        meta_raw: ncm::build_meta_raw("t", "a", "flac"),
        cover_len1: 0xFFFF_FFF0, // 恶意：远超文件长度
        cover: vec![0x42u8; 64],
        audio_enc: ncm::ncm_crypt(&audio, &ncm::rc4_ksa(&rc4_key)),
    });
    let tmp = tempfile::tempdir().unwrap();
    let p = write_tmp(tmp.path(), "cov1.ncm", &blob);
    let out = tempfile::tempdir().unwrap();

    let result = Decoder::open(&p).and_then(|mut d| d.dump(Some(out.path())));
    let err = result.expect_err("coverLen1 越界必须报错");
    assert!(
        matches!(err, NcmError::LengthOutOfRange { .. } | NcmError::EmptyAudio),
        "应报 LengthOutOfRange/EmptyAudio，实际: {err}"
    );
    assert!(
        std::fs::read_dir(out.path()).unwrap().next().is_none(),
        "不得产出 0 字节假成功文件"
    );
}

// ============ 攻击 3：截断（只留头部）—— 不得产出任何文件 ============

#[test]
fn header_only_truncation_must_not_produce_output() {
    let full = ncm::standard_ncm(&ncm::minimal_flac(), "t", "a", "flac", b"");
    let tmp = tempfile::tempdir().unwrap();
    let p = write_tmp(tmp.path(), "full.ncm", &full);
    let audio_len = Decoder::open(&p).unwrap().audio_len() as usize;
    let cut = full.len() - audio_len; // 音频起点 = 头部结束处

    let truncated = &full[..cut];
    let p2 = write_tmp(tmp.path(), "cut.ncm", truncated);
    let out = tempfile::tempdir().unwrap();

    let result = Decoder::open(&p2).and_then(|mut d| d.dump(Some(out.path())));
    let err = result.expect_err("仅剩头部的文件必须报错");
    assert!(
        matches!(err, NcmError::EmptyAudio | NcmError::Truncated { .. } | NcmError::Io(_)),
        "应报 EmptyAudio/Truncated/Io，实际: {err}"
    );
    assert!(std::fs::read_dir(out.path()).unwrap().next().is_none());
}

// ============ 攻击 3b：全量截断扫描 —— 零 panic（blob 与 stream 双路径） ============

#[test]
fn truncation_sweep_never_panics() {
    let full = ncm::standard_ncm(&ncm::minimal_flac(), "t", "a", "flac", &[0x42u8; 64]);
    // parse_blob 路径：每隔 11 字节截断一次，只要求不 panic
    for cut in (0..full.len()).step_by(11) {
        let _ = musicforge_core::header::parse_blob(&full[..cut]);
    }
    // parse_stream 路径（Decoder::open）
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("sweep.ncm");
    for cut in (0..full.len()).step_by(37) {
        std::fs::write(&p, &full[..cut]).unwrap();
        let _ = Decoder::open(&p);
    }
}

// ============ 攻击 4：keyLen 非 16 倍数 —— LengthOutOfRange 而非 panic ============

#[test]
fn key_len_not_multiple_of_16_rejected() {
    let blob = ncm::assemble(ncm::Craft {
        key_len_override: Some(18), // 非 16 倍数
        key_data: vec![0x11u8; 18],
        meta_len_override: None,
        meta_raw: vec![],
        cover_len1: 0,
        cover: vec![],
        audio_enc: vec![0xAAu8; 128],
    });
    let tmp = tempfile::tempdir().unwrap();
    let p = write_tmp(tmp.path(), "badkey.ncm", &blob);
    let out = tempfile::tempdir().unwrap();

    let err = Decoder::open(&p)
        .and_then(|mut d| d.dump(Some(out.path())))
        
        .expect_err("必须报错");
    assert!(
        matches!(err, NcmError::LengthOutOfRange { .. } | NcmError::BadKeyPrefix),
        "应报 LengthOutOfRange/BadKeyPrefix，实际: {err}"
    );
    assert!(std::fs::read_dir(out.path()).unwrap().next().is_none());
}

// ============ 攻击 4b：metaLen 超上界 —— 校验先于分配 ============

#[test]
fn meta_len_over_bound_rejected() {
    let rc4_key: Vec<u8> = (0..112).map(|i| (i * 7 + 13) as u8).collect();
    let blob = ncm::assemble(ncm::Craft {
        key_len_override: None,
        key_data: ncm::build_key_data(&rc4_key),
        meta_len_override: Some((1 << 24) + 1), // 超过 16MB 上界
        meta_raw: vec![],
        cover_len1: 0,
        cover: vec![],
        audio_enc: vec![0u8; 64],
    });
    let tmp = tempfile::tempdir().unwrap();
    let p = write_tmp(tmp.path(), "badmeta.ncm", &blob);
    // `expect_err` 要求 Ok 类型实现 Debug，Decoder 没实现 —— 先 map 成 () 再取错误
    let err = Decoder::open(&p).map(|_| ()).expect_err("必须报错");
    assert!(matches!(err, NcmError::LengthOutOfRange { .. }), "实际: {err}");
}

// ============ 攻击 5：Unicode 边界 —— emoji/RTL/零宽/截断不 panic 不切坏 UTF-8 ============

#[test]
fn unicode_adversarial_titles_render_safely() {
    let long = "曲".repeat(250);
    let cases = [
        "🎨🎶专辑".to_string(),
        "\u{202E}gpj.exe".to_string(), // RTL override（视觉欺骗）
        "a\u{200B}b\u{FEFF}c".to_string(), // 零宽字符
        "🎉".repeat(150),              // 截断点落在 4 字节 emoji 内部
        long,
        "\u{0000}控制\u{0007}字符".to_string(),
    ];
    let tmp = tempfile::tempdir().unwrap();
    for title in &cases {
        // Rust String 本身保证 UTF-8 合法；重点是清洗/截断逻辑不 panic、产物可安全写盘
        let out = render_filename("{title}", Some(&meta_with(title)), "src");
        assert!(
            !out.contains(['/', '\\', ':', '<', '>', '"', '|', '?', '*']),
            "清洗失败: {title:?} → {out:?}"
        );
        assert!(out.chars().count() <= 100, "截断失败: {} chars", out.chars().count());
        // 产物必须真实可写盘（Windows 文件系统级验证）
        let p = tmp.path().join(&out);
        std::fs::write(&p, b"x").unwrap();
    }
}

// ============ 攻击 6：保留设备名（大小写变体）============

#[test]
fn reserved_names_case_insensitive_prefixed() {
    for name in ["CON", "con", "Nul", "LpT2", "aux", "COM1.txt", "lpt9.zip", "PrN"] {
        let out = render_filename("{title}", Some(&meta_with(name)), "src");
        assert!(out.starts_with('_'), "{name} 应加前缀，实际 {out:?}");
    }
    // 非保留名（即便以保留名开头）不得误伤
    assert_eq!(render_filename("{title}", Some(&meta_with("const")), "s"), "const");
    assert_eq!(render_filename("{title}", Some(&meta_with("nullify")), "s"), "nullify");
}

// ============ 攻击 B2：track 宽度规格 ============

#[test]
fn track_width_moderate_spec_ok() {
    // 合理宽度（预修语义保留）
    assert_eq!(render_filename("{track:03d}", Some(&meta_track(7)), "s"), "007");
    assert_eq!(render_filename("{track:08d}", Some(&meta_track(1)), "s"), "00000001");
    // 超宽被钳制（QA-B2 修复语义：宽度上界 16）
    let out = render_filename("{track:100000d}", Some(&meta_track(1)), "s");
    assert!(out.chars().count() <= 16, "宽度应被钳制，实际 {} chars", out.chars().count());
}

#[test]
fn track_width_bomb_is_bounded() {
    // 恶意 metadata / 用户模板可注入任意宽度规格：
    // 修复前 format! 对宽度 > u16::MAX 直接 panic（"Formatting argument out of range"）
    let out = render_filename("{track:99999999999999d}", Some(&meta_track(1)), "s");
    assert!(out.chars().count() < 64, "宽度规格必须有上界，实际 {} chars", out.chars().count());
}

// ============ 攻击 B6：RC4 密钥为空 → build_key_box 除零 panic ============

/// 密钥明文恰好等于前缀（17 字节）时，`key_plain[17..]` 为空切片。
/// `build_key_box` 内部 `key_offset % key.len()` → 除零 panic，
/// 违反硬约束 1（零 panic）；release 下 panic=abort 会直接崩掉整个进程。
#[test]
fn empty_rc4_key_does_not_panic() {
    let blob = ncm::assemble(ncm::Craft {
        key_len_override: None,
        // 故意只放前缀，不带任何 RC4 密钥字节
        key_data: ncm::build_key_data(&[]),
        meta_len_override: None,
        meta_raw: ncm::build_meta_raw("空密钥", "QA", "flac"),
        cover_len1: 0,
        cover: vec![],
        audio_enc: vec![1, 2, 3, 4],
    });
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("empty_key.ncm");
    std::fs::write(&p, &blob).unwrap();

    // 解析路径（parse_stream）：不得 panic，必须返回 Err(EmptyKey)
    let r = std::panic::catch_unwind(|| Decoder::open(&p));
    assert!(r.is_ok(), "Decoder::open 不应 panic（索引越界）");
    let err = r.unwrap().map(|_| ()).expect_err("空 RC4 密钥必须返回错误而非 panic");
    assert!(matches!(err, NcmError::EmptyKey), "应报 EmptyKey，实际: {err}");
    assert_eq!(err.code(), "NCM-STRUCT-INVALID", "稳定错误码必须归类结构损坏");

    // parse_blob 路径（整读解析）同样不得 panic、必须 Err——两条解析路径都要钉死
    let r2 = std::panic::catch_unwind(|| musicforge_core::header::parse_blob(&blob));
    assert!(r2.is_ok(), "parse_blob 不应 panic（索引越界）");
    assert!(r2.unwrap().is_err(), "parse_blob 空密钥必须返回 Err");
}

// ============ 攻击 K2：未知格式 → 显式 UnknownFormat，不得兜底产出假 .flac ============

/// 元数据无 format 字段 + 音频区为无魔数字节 → 三级判定全失败。
/// 修复前行为：兜底产出 `.flac` 扩展名的垃圾文件（谎报成功，上游 B-3 同类）。
/// 修复后：Err(UnknownFormat)，稳定码 NCM-FORMAT-UNKNOWN，输出目录零产物。
#[test]
fn unknown_format_rejected_without_output() {
    use base64::Engine as _;
    // 与 build_meta_raw 同构，但**不含 format 字段**（硬约束 11：元数据合法存在）
    let meta_json = serde_json::json!({
        "musicId": 1, "musicName": "未知格式", "album": "QA",
        "artist": [["测试歌手", 0]], "bitrate": 320000, "duration": 1000,
    })
    .to_string();
    let inner = ncm::aes_ecb_encrypt(&musicforge_core::META_KEY, format!("music:{meta_json}").as_bytes());
    let meta_raw: Vec<u8> = format!(
        "163 key(Don't modify):{}",
        base64::engine::general_purpose::STANDARD.encode(inner)
    )
    .bytes()
    .map(|b| b ^ 0x63)
    .collect();

    let rc4_key: Vec<u8> = (0..112).map(|i| (i * 7 + 13) as u8).collect();
    let blob = ncm::assemble(ncm::Craft {
        key_len_override: None,
        key_data: ncm::build_key_data(&rc4_key),
        meta_len_override: None,
        meta_raw,
        cover_len1: 0,
        cover: vec![],
        // 全零音频区：不匹配任何已知魔数（fLaC/ID3/MPEG 帧头…）
        audio_enc: vec![0u8; 512],
    });
    let tmp = tempfile::tempdir().unwrap();
    let p = write_tmp(tmp.path(), "unknown_fmt.ncm", &blob);
    let out = tempfile::tempdir().unwrap();

    let result = Decoder::open(&p).and_then(|mut d| d.dump(Some(out.path())));
    let err = result.map(|_| ()).expect_err("未知格式必须被拒绝");
    assert!(
        matches!(err, NcmError::UnknownFormat),
        "应报 UnknownFormat，实际: {err}"
    );
    assert_eq!(err.code(), "NCM-FORMAT-UNKNOWN", "稳定错误码");
    assert!(
        std::fs::read_dir(out.path()).unwrap().next().is_none(),
        "不得兜底产出假 .flac 产物"
    );
}

// ============ 攻击 B7（第二轮）：tagger 谎报「已写入标签/已嵌入封面」 ============
//
// 背景：tagger 是本库此前**零测试覆盖**的模块（grep 可证：write_tags 只被
// musicforge-cli 调用，无任何测试）。本组测试把「报告值 == 磁盘真值」钉死。

use lofty::prelude::*;
use lofty::tag::ItemKey;

/// 构造 4 帧 MPEG-1 Layer III / 128kbps / 44100Hz 的裸 MP3（无 ID3v2）
fn bare_mp3() -> Vec<u8> {
    let mut v = Vec::new();
    for _ in 0..4 {
        v.extend_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
        v.extend(std::iter::repeat_n(0u8, 417 - 4)); // 帧长 417（含 4 字节头）
    }
    v
}

/// 128 字节 ID3v1 尾标签（字段以空格填充）
fn id3v1(title: &str, artist: &str, album: &str) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"TAG");
    for f in [title, artist, album] {
        let mut buf = vec![b' '; 30];
        let n = f.len().min(30);
        buf[..n].copy_from_slice(&f.as_bytes()[..n]);
        out.extend_from_slice(&buf);
    }
    out.extend_from_slice(b"2024");
    out.extend(std::iter::repeat_n(b' ', 30));
    out.push(12); // genre
    assert_eq!(out.len(), 128);
    out
}

fn sample_meta() -> Metadata {
    Metadata {
        name: Some("新标题".to_string()),
        artist: Some("新歌手".to_string()),
        album: Some("新专辑".to_string()),
        format: Some("mp3".to_string()),
        bitrate: None,
        duration: None,
        track: None,
        album_pic_url: None,
    }
}

const COVER: &[u8] = &[0x89, b'P', b'N', b'G', 1, 2, 3, 4, 5, 6, 7, 8];

/// 只带 ID3v1 的 MP3：`write_tags` 修复前会把写入落到 ID3v1 上。
/// ID3v1 不支持图片、字段上限 30 字符，lofty 写回时**静默丢弃图片**，
/// 本函数却返回 `Ok((1, true))` → 报告「已嵌入封面」，磁盘上图片数为 0。
#[test]
fn id3v1_only_mp3_must_not_claim_embedded_cover() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("id3v1.mp3");
    let mut payload = bare_mp3();
    payload.extend(id3v1("Old Title", "Old Artist", "Old Album"));
    std::fs::write(&p, &payload).unwrap();

    let (written, embedded) =
        musicforge_core::tagger::write_tags(&p, musicforge_core::Format::Mp3, &sample_meta(), COVER)
            .expect("写入应成功");

    // 断言 1：报告嵌入了封面 → 磁盘上必须真的有封面
    let tagged = lofty::read_from_path(&p).unwrap();
    let total_pictures: usize = tagged.tags().iter().map(|t| t.pictures().len()).sum();
    if embedded {
        assert!(total_pictures > 0, "报 embedded=true 却没有任何图片落盘（谎报）");
    }
    // 断言 2：报告写了 N 个字段 → 主标签里必须真有对应文本
    assert!(written > 0, "报 written={written} 但字段数为 0（谎报）");
    let primary = tagged.primary_tag().expect("应存在主标签");
    assert_eq!(
        primary.get_string(ItemKey::TrackTitle),
        Some("新标题"),
        "标题必须真实落盘"
    );
    // 断言 3：写入目标必须是容器主标签类型（ID3v2），不是能力受限的 ID3v1
    assert_eq!(primary.tag_type(), lofty::tag::TagType::Id3v2);
}

/// ID3v1 三个文本字段**全空**（定长标签里极常见：字段存在但内容为空）。
/// 修复前 `get_string()` 返回 `Some("")` → 被判定为「已有值」→ 跳过写入，
/// 最终 written 只剩封面那 1 项且封面被 ID3v1 丢弃 → **元数据 100% 丢失却报成功**。
#[test]
fn empty_string_tag_field_counts_as_missing() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("blank_id3v1.mp3");
    let mut payload = bare_mp3();
    payload.extend(id3v1("", "", ""));
    std::fs::write(&p, &payload).unwrap();

    let (written, _) =
        musicforge_core::tagger::write_tags(&p, musicforge_core::Format::Mp3, &sample_meta(), COVER)
            .expect("写入应成功");
    assert!(written >= 3, "三个文本字段都空 → 都应写入，实际 written={written}");

    let tagged = lofty::read_from_path(&p).unwrap();
    let primary = tagged.primary_tag().expect("应存在主标签");
    for (key, expect) in [
        (ItemKey::TrackTitle, "新标题"),
        (ItemKey::TrackArtist, "新歌手"),
        (ItemKey::AlbumTitle, "新专辑"),
    ] {
        assert_eq!(
            primary.get_string(key),
            Some(expect),
            "{key:?} 必须真实落盘"
        );
    }
}

/// 不变量（举一反三）：对全部 golden fixture，`write_tags` 报告的字段名
/// 必须能在回读时逐一对上——任何「报了但没写」都在此被拦下。
#[test]
fn golden_fixtures_write_tags_reports_match_disk() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let out_dir = tempfile::tempdir().unwrap();
    for name in [
        "cover_with_padding.ncm",
        "flac_with_cover.ncm",
        "illegal_name.ncm",
        "mp3_raw_no_id3.ncm",
        "mp3_with_id3.ncm",
        "no_cover.ncm",
    ] {
        let mut dec = Decoder::open(dir.join(name)).unwrap();
        let fmt = dec.detect_format().unwrap();
        let target = out_dir.path().join(format!("{name}.{}", fmt.extension()));
        dec.dump_to(&target).unwrap();
        let meta = dec.metadata().cloned().unwrap();
        let cover = dec.cover().to_vec();

        let (written, embedded) = musicforge_core::tagger::write_tags(&target, fmt, &meta, &cover)
            .unwrap_or_else(|e| panic!("{name}: write_tags 失败: {e}"));
        assert!(written > 0, "{name}: 应写入元数据");

        let tagged = lofty::read_from_path(&target).unwrap();
        let primary = tagged.primary_tag().unwrap_or_else(|| panic!("{name}: 无主标签"));
        // 声明写入的字段必须真实可读回（值允许不同：已有值按设计不覆盖）
        assert!(
            primary.get_string(ItemKey::TrackTitle).is_some(),
            "{name}: 报 written={written} 但标题未落盘"
        );
        if embedded {
            assert!(
                !primary.pictures().is_empty(),
                "{name}: 报 embedded=true 但主标签无图片（谎报）"
            );
        }
        if !cover.is_empty() {
            assert!(
                !primary.pictures().is_empty(),
                "{name}: 有封面数据却未嵌入"
            );
        }
    }
}

/// 硬约束 1：任何输入下 tagger 都不得 panic。
/// 覆盖「容器与声明格式不符」的组合——此时 lofty 按扩展名解析会失败，
/// 必须返回 Err（如实报错），既不能 panic 也不能谎报成功。
#[test]
fn tagger_never_panics_on_container_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let meta = sample_meta();

    // 声明 flac，实际负载为裸 MP3（扩展名 .flac）
    let p = dir.path().join("mismatch.flac");
    std::fs::write(&p, bare_mp3()).unwrap();
    let r = std::panic::catch_unwind(|| {
        musicforge_core::tagger::write_tags(&p, musicforge_core::Format::Flac, &meta, COVER)
    });
    assert!(r.is_ok(), "不得 panic（违反硬约束 1）");
    assert!(r.unwrap().is_err(), "容器与扩展名不符时必须如实返回 Err");

    // 声明 mp3，实际负载为最小 FLAC（扩展名 .mp3）
    let p2 = dir.path().join("mismatch.mp3");
    std::fs::write(&p2, ncm::minimal_flac()).unwrap();
    let r2 = std::panic::catch_unwind(|| {
        musicforge_core::tagger::write_tags(&p2, musicforge_core::Format::Mp3, &meta, COVER)
    });
    assert!(r2.is_ok(), "不得 panic（违反硬约束 1）");
    assert!(r2.unwrap().is_err(), "容器与扩展名不符时必须如实返回 Err");
}

/// `NcmError::TagWrite` 与 `TagRead` 性质相反，必须保持如实报错：
/// · `TagRead` = 源/输出文件的**元数据层面**问题，音频已完整落盘 → 可降级告警
/// · `TagWrite` = **输出侧环境故障**（只读 / 被播放器占用 / 不可写）→ 需用户干预，必须 Err
///
/// 本用例用**真实 OS 级只读文件**构造该失败，而非 mock：
/// lofty `AudioFile::save_to_path` 以 `OpenOptions::read(true).write(true)` 打开目标，
/// 只读文件在 Windows 与 POSIX 下 open 均失败 → `FileEncodingError`
/// → tagger 必须映射为 `NcmError::TagWrite`（既不得 panic，也不得谎报 Ok）。
#[test]
fn read_only_output_must_fail_with_tag_write() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("readonly.mp3");
    std::fs::write(&p, bare_mp3()).unwrap();

    let set_ro = |ro: bool| {
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        perms.set_readonly(ro);
        std::fs::set_permissions(&p, perms).unwrap();
    };

    set_ro(true);

    // 前置校验：确认本平台确实把只读属性落实为「写打开失败」。
    // 若运行身份不受只读限制（如某些 root 场景），本用例无法构造目标状态，
    // 如实跳过并说明原因，而不是伪装成通过。
    if std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&p)
        .is_ok()
    {
        set_ro(false);
        eprintln!("[skip] 当前平台/运行身份未落实只读属性，无法构造 TagWrite 失败场景");
        return;
    }

    let meta = sample_meta();
    let r = std::panic::catch_unwind(|| {
        musicforge_core::tagger::write_tags(&p, musicforge_core::Format::Mp3, &meta, COVER)
    });
    assert!(r.is_ok(), "只读输出不得 panic（硬约束 1）");

    // 恢复可写，保证 tempdir 能被正常清理
    set_ro(false);

    match r.unwrap() {
        Err(NcmError::TagWrite(msg)) => {
            assert!(!msg.trim().is_empty(), "TagWrite 必须携带可诊断信息");
        }
        other => panic!("只读输出必须映射为 NcmError::TagWrite，实际：{other:?}"),
    }

    // 反向确认：解除只读后同一输入必须成功（证明上一步的失败确实由只读引起，
    // 而不是样本本身不可打标签）
    musicforge_core::tagger::write_tags(&p, musicforge_core::Format::Mp3, &meta, COVER)
        .expect("解除只读后必须能正常打标签");
}
