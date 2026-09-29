//! 运行时验证：i18n 补丁在 Locale::En 下必须产出纯英文、不含中文（编译通过 ≠ 英文生效）。
use musicforge_core::dedupe::*;
use std::path::PathBuf;

fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

fn mk_file(path: &str, sample_rate: u32) -> DedupeFile {
    DedupeFile {
        path: PathBuf::from(path),
        size: 1,
        sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
        score: ScoreBreakdown {
            lossless: false,
            sample_rate,
            bit_depth: 16,
            has_tags: false,
            has_cover: false,
            verified: false,
            notes: vec![],
        },
    }
}

#[test]
fn detail_locale_en_is_english_and_no_cjk() {
    let sb = ScoreBreakdown {
        lossless: true,
        sample_rate: 0,
        bit_depth: 0,
        has_tags: true,
        has_cover: true,
        verified: true,
        notes: vec![],
    };
    let en = sb.detail_with_locale(&PROFILE_FIDELITY, 0, 0, Locale::En);
    assert!(en.contains("Lossless"), "en detail: {en}");
    assert!(en.contains("Sample rate"), "en detail: {en}");
    assert!(en.contains("Bit depth"), "en detail: {en}");
    assert!(en.contains("Tags"), "en detail: {en}");
    assert!(en.contains("Cover"), "en detail: {en}");
    assert!(en.contains("Verified"), "en detail: {en}");
    assert!(!has_cjk(&en), "en detail leaked CJK: {en}");
    // 对照：中文路径应含中文标签
    let zh = sb.detail_with_locale(&PROFILE_FIDELITY, 0, 0, Locale::Zh);
    assert!(has_cjk(&zh), "zh detail should contain CJK: {zh}");
}

#[test]
fn dedupe_note_render_en_is_english() {
    let n = DedupeNote::AttrParseFailed("boom".into());
    let en = n.render(Locale::En);
    assert!(en.contains("Attribute parse failed"), "en note: {en}");
    assert!(!has_cjk(&en), "en note leaked CJK: {en}");
}

#[test]
fn dup_group_sacrifice_reason_en_is_english() {
    let keep = mk_file("keep.flac", 48000);
    let sac = mk_file("sac.flac", 44100);
    let g = DupGroup {
        sha256: keep.sha256.clone(),
        size: 1,
        files: vec![keep, sac],
        keep_index: 0,
    };
    let en = g.sacrifice_reason_with_locale(&PROFILE_FIDELITY, &g.files[1], Locale::En);
    assert!(en.contains("Exact duplicate"), "en sacrifice: {en}");
    assert!(!has_cjk(&en), "en sacrifice leaked CJK: {en}");
}

#[test]
fn same_name_group_candidate_reason_en_is_english() {
    let keep = mk_file("song.flac", 48000);
    let cand = mk_file("song.wav", 44100);
    let g = SameNameGroup {
        stem: "song".into(),
        files: vec![keep, cand],
        keep_index: 0,
    };
    let en = g.candidate_reason_with_locale(&PROFILE_FIDELITY, &g.files[1], Locale::En);
    assert!(
        en.contains("Same name but different content"),
        "en candidate: {en}"
    );
    assert!(!has_cjk(&en), "en candidate leaked CJK: {en}");
}
