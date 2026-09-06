//! P1e：错误码双命名空间一致性测试。
//!
//! 契约：`NcmError::code()`（legacy `NCM-*`）保持不变以兼容既有脚本与失败清单；
//! `NcmError::mf_code()`（新 `MF-*`）为跨格式/插件的统一命名空间。两者都必须
//! **稳定且非空**，映射关系见 `docs/result-codes.md`。

use musicforge_core::NcmError;

fn codes(e: &NcmError) -> (&'static str, &'static str) {
    (e.code(), e.mf_code())
}

#[test]
fn every_variant_has_both_codes() {
    let cases: Vec<NcmError> = vec![
        NcmError::BadMagic,
        NcmError::Truncated { at: "key", need: 8, got: 4 },
        NcmError::LengthOutOfRange { at: "meta", value: 1 << 40, max: 1 << 20 },
        NcmError::BadKeyPrefix,
        NcmError::BadMetaPrefix,
        NcmError::BadMusicPrefix,
        NcmError::EmptyKey,
        NcmError::CrcMismatch { expected: 1, computed: 2 },
        NcmError::EmptyAudio,
        NcmError::UnknownFormat,
        NcmError::OutputIntegrity { written: 1, expected: 2 },
        NcmError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "missing")),
        NcmError::TagRead("boom".into()),
        NcmError::TagWrite("boom".into()),
    ];

    for e in &cases {
        let (legacy, mf) = codes(e);
        assert!(!legacy.is_empty(), "{e:?}: legacy 码不得为空");
        assert!(mf.starts_with("MF-"), "{e:?}: 新码必须属 MF-* 命名空间，实际 {mf}");
    }
}

#[test]
fn mapping_is_stable() {
    // 关键映射钉死：改映射等于破坏下游解析，必须同步改 docs/result-codes.md
    assert_eq!(NcmError::BadMagic.code(), "NCM-BAD-MAGIC");
    assert_eq!(NcmError::BadMagic.mf_code(), "MF-FORMAT-UNSUPPORTED");
    assert_eq!(NcmError::CrcMismatch { expected: 1, computed: 2 }.code(), "NCM-CRC-MISMATCH");
    // 旧码 NCM-FORMAT-UNKNOWN 保留（X7：不破坏既有用户脚本）
    assert_eq!(NcmError::UnknownFormat.code(), "NCM-FORMAT-UNKNOWN");
    assert_eq!(NcmError::UnknownFormat.mf_code(), "MF-FORMAT-UNKNOWN");
    assert_eq!(NcmError::EmptyAudio.code(), "NCM-EMPTY-AUDIO");
    assert_eq!(NcmError::TagWrite("x".into()).mf_code(), "MF-TAG-WRITE-FAILED");
}
