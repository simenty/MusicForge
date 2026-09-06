//! golden 回归测试：5 个自建合规 fixture + metadataLen=0 边界（自建编码器生成，不含任何版权材料）。
//!
//! 断言层（验收三分离之「确定性」层）：
//! 1. 逐字节一致：dump 输出 sha256 == 预期（预期由 Python 参考解码器 dump_expected 生成）
//! 2. 元数据字段全等
//! 3. 格式判定（约束 8 回归：裸 MP3 必须判 mp3）
//! 4. CRC 篡改检出（约束 9 回归：报错且无产物）

use musicforge_core::{Decoder, NcmError};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn sha256_file(p: &Path) -> String {
    let mut f = std::fs::File::open(p).unwrap();
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h).unwrap();
    hex(h.finalize())
}

fn hex(b: impl AsRef<[u8]>) -> String {
    b.as_ref().iter().map(|x| format!("{x:02x}")).collect()
}

fn load_expected(name: &str) -> serde_json::Value {
    let p = fixtures_dir().join(format!("{name}.expected.json"));
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// 断言 1+2+3：每个 fixture 的元数据 / 格式 / 逐字节输出
#[test]
fn golden_all_fixtures_byte_exact() {
    let dir = fixtures_dir();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let p = entry.unwrap().path();
        let is_expected = p
            .file_name()
            .and_then(|f| f.to_str())
            .map(|f| f.ends_with(".expected.json"))
            .unwrap_or(false);
        if !is_expected {
            continue;
        }
        let exp: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let name = exp["ncm"].as_str().unwrap().to_string();
        let ncm_path = dir.join(&name);

        let mut dec = Decoder::open(&ncm_path).unwrap_or_else(|e| panic!("open {name} 失败: {e}"));

        // 元数据断言（None 或逐字段）
        match exp.get("metadata") {
            None | Some(serde_json::Value::Null) => {
                assert!(dec.metadata().is_none(), "{name}: 元数据应为 None");
            }
            Some(m) => {
                let md = dec.metadata().expect("{name}: 元数据不应为 None");
                assert_eq!(
                    md.name.as_deref(),
                    m["musicName"].as_str(),
                    "{name}: musicName"
                );
                assert_eq!(md.artist.as_deref(), m["artist"].as_str(), "{name}: artist");
                assert_eq!(md.album.as_deref(), m["album"].as_str(), "{name}: album");
                assert_eq!(md.format.as_deref(), m["format"].as_str(), "{name}: format");
            }
        }

        // 封面断言
        assert_eq!(
            dec.cover().len(),
            exp["cover_len"].as_u64().unwrap() as usize,
            "{name}: 封面长度"
        );

        // 音频长度 + 格式 + 逐字节 sha256
        assert_eq!(
            dec.audio_len(),
            exp["audio_len"].as_u64().unwrap(),
            "{name}: 音频长度"
        );
        let tmp = tempfile::tempdir().unwrap();
        let out = dec
            .dump(Some(tmp.path()))
            .unwrap_or_else(|e| panic!("dump {name} 失败: {e}"));
        assert_eq!(
            out.extension().and_then(|e| e.to_str()),
            exp["format_ext"].as_str(),
            "{name}: 输出扩展名（格式判定回归）"
        );
        assert_eq!(
            sha256_file(&out),
            exp["audio_sha256"].as_str().unwrap(),
            "{name}: 输出逐字节不一致"
        );
    }
}

/// 断言 4：CRC 篡改检出（硬约束 9）——报 CrcMismatch 且不产出文件
#[test]
fn crc_tamper_detected_and_no_output() {
    let src = fixtures_dir().join("flac_with_cover.ncm");
    let mut blob = std::fs::read(&src).unwrap();
    blob[100] ^= 0xff; // 篡改 keyData 中间字节
    let tampered = fixtures_dir().join("__tampered.ncm");
    std::fs::write(&tampered, &blob).unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let err = Decoder::open(&tampered)
        .and_then(|mut d| d.dump(Some(tmp.path())).map(|_| ()))
        .expect_err("篡改文件必须失败");
    assert!(
        matches!(err, NcmError::CrcMismatch { .. }) || matches!(err, NcmError::BadKeyPrefix),
        "应报 CRC/密钥校验错误，实际: {err}"
    );
    // 无半成品文件
    let leftovers: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().collect();
    assert!(leftovers.is_empty(), "不得产出任何文件");
    let _ = std::fs::remove_file(&tampered);
}

/// 裸 MP3 格式回归（约束 8）：上游 Go 版盲判 flac 的缺陷不得复现
#[test]
fn bare_mp3_format_regression() {
    let mut dec = Decoder::open(fixtures_dir().join("mp3_raw_no_id3.ncm")).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let out = dec.dump(Some(tmp.path())).unwrap();
    assert_eq!(out.extension().and_then(|e| e.to_str()), Some("mp3"));
}

/// 覆盖帧 padding 回归（L1 > L2）：音频起点必须跳过 padding（Go 版 Seek 的必要性实证）
#[test]
fn cover_padding_regression() {
    let mut dec = Decoder::open(fixtures_dir().join("cover_with_padding.ncm")).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let out = dec.dump(Some(tmp.path())).unwrap();
    assert_eq!(
        sha256_file(&out),
        load_expected("cover_with_padding.ncm")["audio_sha256"]
            .as_str()
            .unwrap(),
        "padding 必须被正确跳过"
    );
}

/// metadataLen=0 边界（上游 B-2 触发场景）：metadata None + 封面仍嵌入 + 转换成功
#[test]
fn metadata_len_zero_regression() {
    let mut dec = Decoder::open(fixtures_dir().join("metadata_len0.ncm")).unwrap();
    assert!(dec.metadata().is_none(), "元数据应为 None");
    assert_eq!(dec.cover().len(), 68, "封面应正常提取");
    let tmp = tempfile::tempdir().unwrap();
    let out = dec.dump(Some(tmp.path())).unwrap();
    assert_eq!(
        sha256_file(&out),
        load_expected("metadata_len0.ncm")["audio_sha256"]
            .as_str()
            .unwrap()
    );
}

/// P1a 保护网：解码**确定性**。同一 fixture 两次独立解码，音频负载 sha256 必须一致。
///
/// 这条管住的是「重构引入了隐藏状态 / 非确定初始化」这一类回归：
/// 金标比对（`golden_all_fixtures_byte_exact`）只证明「与 Python 参考解码器一致」，
/// 本条额外证明「自身可重复」。二者合起来才封死「确定性被破坏」的缝隙。
/// 覆盖三类代表样本：flac（带封面）、mp3（带 ID3）、metadataLen=0（无元数据）。
#[test]
fn golden_decode_is_deterministic() {
    for name in [
        "flac_with_cover.ncm",
        "mp3_with_id3.ncm",
        "metadata_len0.ncm",
    ] {
        let mut a = Decoder::open(fixtures_dir().join(name))
            .unwrap_or_else(|e| panic!("{name}: 首次打开失败: {e}"));
        let tmp_a = tempfile::tempdir().unwrap();
        let out_a = a
            .dump(Some(tmp_a.path()))
            .unwrap_or_else(|e| panic!("{name}: 首次解码失败: {e}"));

        let mut b = Decoder::open(fixtures_dir().join(name))
            .unwrap_or_else(|e| panic!("{name}: 二次打开失败: {e}"));
        let tmp_b = tempfile::tempdir().unwrap();
        let out_b = b
            .dump(Some(tmp_b.path()))
            .unwrap_or_else(|e| panic!("{name}: 二次解码失败: {e}"));

        assert_eq!(
            sha256_file(&out_a),
            sha256_file(&out_b),
            "{name}: 两次解码结果不一致 —— 解码过程引入了非确定状态"
        );
    }
}
