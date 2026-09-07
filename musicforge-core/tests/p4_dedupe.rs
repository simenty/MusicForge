//! P4.1：曲库去重（exact 分组 + 同名候选 + 可解释保留评分）。
//!
//! 验收口径（对齐 ROADMAP P4）：
//! - exact 组成员/保留项判定正确；unique 文件不入组；
//! - keep-best reason 可复算（两次运行分数与保留项一致）；
//! - apply 后牺牲项全在回收站、保留项原位未动；restore 整体还原；
//! - 同名候选默认仅报告；--include-same-name 才纳入执行；
//! - 评分权重单元钉死（无损+40/采样率+8/位深+8/标签+10/封面+5/校验+20）。

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use musicforge_core::db::Db;
use musicforge_core::dedupe::{
    build_dedupe_plan, dedupe_scan, DedupeOptions, ScoreBreakdown, W_BIT_DEPTH, W_COVER,
    W_LOSSLESS, W_SAMPLE_RATE, W_TAGS, W_VERIFIED,
};
use musicforge_core::scan::{apply_clean_plan, restore_from_trash};

fn uniq_root(tag: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("mf-p4-{tag}-{n}-{}-{}", seq, std::process::id()))
}

/// 最小合法 PCM WAV（单声道静音）——lofty 可解析出采样率/位深/无损容器。
fn wav_bytes(sample_rate: u32, bits: u16) -> Vec<u8> {
    let data = vec![0u8; 1024];
    let byte_rate = sample_rate * bits as u32 / 8; // 单声道
    let block_align = bits / 8;
    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    v.extend_from_slice(b"WAVE");
    v.extend_from_slice(b"fmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&1u16.to_le_bytes()); // 单声道
    v.extend_from_slice(&sample_rate.to_le_bytes());
    v.extend_from_slice(&byte_rate.to_le_bytes());
    v.extend_from_slice(&block_align.to_le_bytes());
    v.extend_from_slice(&bits.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&(data.len() as u32).to_le_bytes());
    v.extend_from_slice(&data);
    v
}

/// 树结构：
/// - exact 组 A ×3（同内容合法 WAV，跨 dupA/dupB 目录）；
/// - exact 组 B ×2（同内容垃圾 mp3，属性不可解析 → 全 0 分）；
/// - 同名组 "song"（同目录同 stem 不同内容，属性均未知）；
/// - 同名组 "demo"（合法 WAV 48k vs 垃圾 ogg → WAV 应胜出）；
/// - unique.flac（内容/大小/stem 全唯一，不入任何组）。
fn build_tree(root: &Path) {
    let wa = wav_bytes(44100, 16);
    std::fs::create_dir_all(root.join("dupA")).unwrap();
    std::fs::create_dir_all(root.join("dupB")).unwrap();
    std::fs::create_dir_all(root.join("music")).unwrap();
    std::fs::create_dir_all(root.join("covers")).unwrap();
    std::fs::create_dir_all(root.join("live")).unwrap();
    std::fs::write(root.join("dupA/one.wav"), &wa).unwrap();
    std::fs::write(root.join("dupA/one-copy.wav"), &wa).unwrap();
    std::fs::write(root.join("dupB/one.wav"), &wa).unwrap();
    std::fs::write(root.join("music/b1.mp3"), b"garbage-not-a-real-mp3-A").unwrap();
    std::fs::write(root.join("music/b2.mp3"), b"garbage-not-a-real-mp3-A").unwrap();
    std::fs::write(root.join("covers/song.flac"), b"fake-flac-content-v1").unwrap();
    std::fs::write(root.join("covers/song.mp3"), b"fake-mp3-content-v2").unwrap();
    std::fs::write(root.join("live/demo.wav"), wav_bytes(48000, 24)).unwrap();
    std::fs::write(root.join("live/demo.ogg"), b"fake-ogg-content-x").unwrap();
    std::fs::write(root.join("music/unique.flac"), b"one-of-a-kind-content").unwrap();
}

fn ends_with(p: &Path, suffix: &str) -> bool {
    p.to_string_lossy().replace('\\', "/").ends_with(suffix)
}

#[test]
fn exact_groups_and_members_are_correct() {
    let root = uniq_root("groups");
    build_tree(&root);
    let rep = dedupe_scan(&root, &DedupeOptions::default(), None).unwrap();

    assert_eq!(rep.groups.len(), 2, "应恰有 2 个 exact 组");
    let group_sizes: Vec<usize> = rep.groups.iter().map(|g| g.files.len()).collect();
    assert!(group_sizes.contains(&3), "WAV 组应 3 成员: {group_sizes:?}");
    assert!(group_sizes.contains(&2), "mp3 组应 2 成员: {group_sizes:?}");

    // WAV 组：3 成员全同分 → 平分取路径字典序最小
    //（字节序："one-copy.wav" 的 '-'(0x2D) < "one.wav" 的 '.'(0x2E)）
    let wav_group = rep.groups.iter().find(|g| g.files.len() == 3).unwrap();
    assert!(
        ends_with(&wav_group.keep().path, "dupA/one-copy.wav"),
        "平分应保留路径最小者: {}",
        wav_group.keep().path.display()
    );
    assert_eq!(wav_group.sacrifices().len(), 2);

    // unique 不属于任何组
    for g in &rep.groups {
        for f in &g.files {
            assert!(
                !ends_with(&f.path, "music/unique.flac"),
                "unique 文件不得入组"
            );
        }
    }
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn keep_prefers_clean_name_over_artifact_suffix() {
    // 真库形态回归：重复下载残留 "song.flac" + "song (2).flac" 内容完全相同
    // → 全体平分 → 必须保留干净命名的 song.flac，牺牲 (2) 副本（旧行为相反）。
    let root = uniq_root("artifact");
    let lib = root.join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("song.flac"), b"identical-payload").unwrap();
    std::fs::write(lib.join("song (2).flac"), b"identical-payload").unwrap();

    let rep = dedupe_scan(&lib, &DedupeOptions::default(), None).unwrap();
    assert_eq!(rep.groups.len(), 1);
    let g = &rep.groups[0];
    let keep_name = g.keep().path.file_name().unwrap().to_str().unwrap();
    assert_eq!(
        keep_name,
        "song.flac",
        "平分必须保留干净命名: {}",
        g.keep().path.display()
    );
    assert_eq!(g.sacrifices().len(), 1);
    assert!(g.sacrifices()[0]
        .path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("song ("));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn keep_best_reason_is_recomputable() {
    let root = uniq_root("recomp");
    build_tree(&root);
    let r1 = dedupe_scan(&root, &DedupeOptions::default(), None).unwrap();
    let r2 = dedupe_scan(&root, &DedupeOptions::default(), None).unwrap();

    assert_eq!(r1.groups.len(), r2.groups.len());
    for (g1, g2) in r1.groups.iter().zip(r2.groups.iter()) {
        assert_eq!(g1.keep().path, g2.keep().path, "两次运行保留项必须一致");
        assert_eq!(
            g1.score_of(g1.keep()),
            g2.score_of(g2.keep()),
            "两次运行分数必须一致"
        );
        for (f1, f2) in g1.files.iter().zip(g2.files.iter()) {
            assert_eq!(g1.sacrifice_reason(f1), g2.sacrifice_reason(f2));
        }
    }
    for (s1, s2) in r1.same_name.iter().zip(r2.same_name.iter()) {
        assert_eq!(s1.keep().path, s2.keep().path);
        assert_eq!(s1.score_of(s1.keep()), s2.score_of(s2.keep()));
    }
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn same_name_prefers_lossless_and_reports_only() {
    let root = uniq_root("sname");
    build_tree(&root);
    let rep = dedupe_scan(&root, &DedupeOptions::default(), None).unwrap();

    assert_eq!(rep.same_name.len(), 2, "song/demo 两组同名候选");
    let demo = rep
        .same_name
        .iter()
        .find(|g| g.stem == "demo")
        .expect("demo 组应存在");
    assert!(
        ends_with(&demo.keep().path, "live/demo.wav"),
        "合法 WAV（无损+40 采样率+8 位深+8=56）应胜过未知 ogg: {}",
        demo.keep().path.display()
    );
    assert_eq!(demo.score_of(demo.keep()), 56);
    assert_eq!(demo.score_of(&demo.files[demo.keep_index ^ 1]), 0);
    let reason = demo.candidate_reason(&demo.files[demo.keep_index ^ 1]);
    assert!(reason.contains("无损+0"), "候选 reason 应含明细: {reason}");

    // 同名组默认不进计划（exact 牺牲项 3 个：WAV 组 2 + mp3 组 1）
    let plan = build_dedupe_plan(&rep, &root.join(".musicforge/trash"), &root, false);
    assert_eq!(plan.actions.len(), 3, "同名候选默认仅报告");
    assert!(
        plan.actions.iter().all(|a| a.rule_id == "MF-DUP-EXACT"),
        "默认计划只含 exact 牺牲项"
    );

    // include_same_name=true 时纳入 2 个同名候选
    let plan2 = build_dedupe_plan(&rep, &root.join(".musicforge/trash"), &root, true);
    assert_eq!(plan2.actions.len(), 5);
    assert!(plan2
        .actions
        .iter()
        .any(|a| a.rule_id == "MF-DUP-SAME-NAME"));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn apply_moves_sacrifices_to_trash_and_restores() {
    let root = uniq_root("apply");
    build_tree(&root);
    let rep = dedupe_scan(&root, &DedupeOptions::default(), None).unwrap();
    let trash = root.join(".musicforge/trash");
    let plan = build_dedupe_plan(&rep, &trash, &root, false);

    // 记录保留项原内容
    let keep_paths: Vec<PathBuf> = rep.groups.iter().map(|g| g.keep().path.clone()).collect();
    let keep_contents: Vec<Vec<u8>> = keep_paths
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect();

    let outcome = apply_clean_plan(&plan, "task-p4-test").unwrap();
    assert_eq!(outcome.moved, 3, "牺牲项 3 个应全部移入回收站");

    // 牺牲项原位置已空、保留项原位未动
    for g in &rep.groups {
        for f in g.sacrifices() {
            assert!(!f.path.exists(), "牺牲项应已离开原位: {}", f.path.display());
        }
    }
    for (p, c) in keep_paths.iter().zip(keep_contents.iter()) {
        assert!(p.exists(), "保留项必须在原位: {}", p.display());
        assert_eq!(std::fs::read(p).unwrap(), *c, "保留项字节级未动");
    }

    // 整体还原（复用 clean --restore 机制）
    let rb = outcome.rollback_manifest.as_ref().unwrap();
    let restored = restore_from_trash(rb).unwrap();
    assert_eq!(restored, 3);
    for g in &rep.groups {
        for f in g.sacrifices() {
            assert!(
                f.path.exists(),
                "还原后牺牲项应回原位: {}",
                f.path.display()
            );
        }
    }
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn score_weights_are_pinned() {
    let base = ScoreBreakdown::default();
    assert_eq!(base.total(44100, 16), 0, "全未知=0 分");

    let lossless_only = ScoreBreakdown {
        lossless: true,
        ..Default::default()
    };
    assert_eq!(lossless_only.total(0, 0), W_LOSSLESS);

    let full = ScoreBreakdown {
        lossless: true,
        sample_rate: 48000,
        bit_depth: 24,
        has_tags: true,
        has_cover: true,
        verified: true,
        ..Default::default()
    };
    assert_eq!(
        full.total(48000, 24),
        W_LOSSLESS + W_SAMPLE_RATE + W_BIT_DEPTH + W_TAGS + W_COVER + W_VERIFIED,
        "满分 = 40+8+8+10+5+20 = 91"
    );
    // 组内最高才得分：采样率低于组内最高 → +0
    assert_eq!(full.total(96000, 24), 91 - W_SAMPLE_RATE);

    let d = full.detail(48000, 24);
    assert!(d.contains("无损+40") && d.contains("校验+20"), "{d}");
}

#[test]
fn hash_cache_is_reused_across_runs() {
    let root = uniq_root("cache");
    build_tree(&root);
    let db = Db::open_in_memory().unwrap();

    let r1 = dedupe_scan(&root, &DedupeOptions::default(), Some(&db)).unwrap();
    assert!(r1.hashed_now > 0, "首扫应实算哈希");
    assert_eq!(r1.cache_hits, 0);

    let r2 = dedupe_scan(&root, &DedupeOptions::default(), Some(&db)).unwrap();
    assert_eq!(r2.hashed_now, 0, "二扫应全命中（D17 缓存）");
    assert_eq!(
        r2.cache_hits, r1.hashed_now,
        "命中数 = 首扫实算数（同一参与集合）"
    );
    // 分组结论与无缓存路径完全一致
    assert_eq!(r1.groups.len(), r2.groups.len());
    std::fs::remove_dir_all(&root).ok();
}
