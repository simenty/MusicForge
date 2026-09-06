//! P1c 绞杀者重构：新旧路径等价性测试。
//!
//! 目的：在 CLI 切换到 `FormatRegistry`（P1d）之前，先用测试证明
//! 「新路径（FormatAdapter/Registry）」与「旧路径（Decoder 直调）」对同一输入
//! 产出**字节级一致的音频负载**与相同的元数据字段。
//!
//! 断言的是外部可观察行为（产物 sha256 / 文件扩展名 / 元数据字段），
//! 不触碰任何内部结构——这样 P1b/P1c 的移动与抽象不会让测试变脆。

use std::fs;
use std::path::{Path, PathBuf};

use musicforge_core::formats::registry::{FormatAdapter, FormatRegistry, NcmAdapter, ProbeInput};
use musicforge_core::{Decoder, MAGIC};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn sha256_file(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let bytes = fs::read(path).expect("读取产物失败");
    let digest = Sha256::digest(&bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn ncm_fixtures() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(fixtures_dir())
        .expect("fixtures 目录可读")
        .map(|e| e.expect("目录项").path())
        .filter(|p| {
            p.extension()
                .map(|e| e.eq_ignore_ascii_case("ncm"))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "至少需要一个 .ncm fixture");
    files
}

/// 核心等价性：对每个 fixture，旧路径（Decoder 直调）与新路径（Registry→Adapter）
/// 产出的音频负载必须字节一致，元数据字段一致，格式扩展名一致。
#[test]
fn adapter_path_matches_legacy_decoder() {
    let tmp = tempfile::tempdir().unwrap();
    let legacy_dir = tmp.path().join("legacy");
    let adapter_dir = tmp.path().join("adapter");
    fs::create_dir_all(&legacy_dir).unwrap();

    let registry = FormatRegistry::with_builtins();

    for fixture in ncm_fixtures() {
        let stem = fixture.file_stem().and_then(|s| s.to_str()).unwrap();

        // ---- 旧路径：Decoder 直调 ----
        let mut legacy_decoder = Decoder::open(&fixture).expect("旧路径 open 成功");
        let legacy_format = legacy_decoder.detect_format().expect("旧路径格式判定成功");
        let legacy_meta = legacy_decoder.metadata().cloned();
        let legacy_target = legacy_dir.join(format!("{stem}.{}", legacy_format.extension()));
        legacy_decoder.dump_to(&legacy_target).expect("旧路径落盘成功");

        // ---- 新路径：Registry → Adapter ----
        let adapter = registry
            .detect_file(&fixture)
            .unwrap_or_else(|| panic!("{stem}: registry 应识别该 fixture"));
        let decoded = adapter
            .decode(&fixture, &adapter_dir)
            .unwrap_or_else(|e| panic!("{stem}: 新路径解码失败: {e}"));

        // ---- 等价性断言 ----
        assert_eq!(
            sha256_file(&legacy_target),
            sha256_file(&decoded.path),
            "{stem}: 新旧路径音频负载必须字节一致"
        );
        assert_eq!(
            legacy_format.extension(),
            decoded.format.extension(),
            "{stem}: 格式扩展名必须一致"
        );

        // 元数据可能整体缺失（如 metadata_len0 fixture，硬约束 11：缺失 → None 而非崩）。
        // 等价性要求的是「两侧一致」，不是「一定有元数据」。
        match (&legacy_meta, &decoded.metadata) {
            (Some(a), Some(b)) => {
                assert_eq!(a.name, b.name, "{stem}: 曲名一致");
                assert_eq!(a.artist, b.artist, "{stem}: 艺术家一致");
                assert_eq!(a.album, b.album, "{stem}: 专辑一致");
                assert_eq!(a.format, b.format, "{stem}: 元数据格式字段一致");
            }
            (None, None) => {}
            _ => panic!("{stem}: 新旧路径元数据存在性不一致"),
        }
    }
}

/// 探测语义：魔数命中=1.0；仅扩展名=0.3（不据此直接解码）；两者都不匹配=None。
#[test]
fn probe_confidence_semantics() {
    let adapter = NcmAdapter;
    assert_eq!(adapter.id(), "ncm");

    let magic_hit = adapter.probe(&ProbeInput {
        extension: Some("ncm"),
        header: &MAGIC,
    });
    assert_eq!(
        magic_hit,
        Some(musicforge_core::formats::registry::ProbeResult {
            format_id: "ncm",
            confidence: 1.0
        })
    );

    // 只有扩展名像 ncm 但魔数不符 → 不认领（避免把损坏文件伪装成可处理）
    let ext_only = adapter.probe(&ProbeInput {
        extension: Some("ncm"),
        header: b"not a real ncm header",
    });
    assert!(
        ext_only.is_none(),
        "仅扩展名匹配不应被认领——交给旧路径给出明确错误"
    );

    let none = adapter.probe(&ProbeInput {
        extension: Some("flac"),
        header: b"fLaC....",
    });
    assert!(none.is_none(), "非 ncm 不应被 NCM 适配器认领");
}

/// 负向：垃圾文件不应被任意内置适配器认领（防止「兜底伪装成功」，G5 教训）。
#[test]
fn garbage_file_is_not_claimed_by_builtins() {
    let tmp = tempfile::tempdir().unwrap();
    let junk = tmp.path().join("junk.ncm");
    fs::write(&junk, b"garbage data here").unwrap();

    let registry = FormatRegistry::with_builtins();
    let claimed = registry.detect_file(&junk);
    assert!(
        claimed.is_none(),
        "垃圾文件不应被认领：魔数不符时只认扩展名会掩盖真实错误"
    );
}

/// 注册表形状：内置含 NCM；空注册表不认领任何东西。
#[test]
fn registry_shape() {
    let full = FormatRegistry::with_builtins();
    assert_eq!(full.len(), 1);
    assert!(!full.is_empty());

    let empty = FormatRegistry::new();
    assert!(empty.is_empty());
    let fixture = ncm_fixtures().remove(0);
    assert!(empty.detect_file(&fixture).is_none());
}
