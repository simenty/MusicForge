//! P4.2：曲库整理（模板归位 + 冲突策略 + 回滚还原）。
//!
//! 验收口径（对齐 ROADMAP P4）：
//! - organize 模板输出一致（与 convert 同源渲染语义，扩展名保留原格式）；
//! - 冲突 skip 报告 / suffix 续编号 / overwrite-never 计失败（MF-PATH-CONFLICT），
//!   **任何策略绝不覆盖目标**；
//! - apply 后文件位于模板结构；二次规划全为「已在位」（幂等）；
//! - 回滚清单整体还原（复用 clean --restore 机制）。

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use musicforge_core::organize::{
    apply_organize_plan, plan_organize, ConflictStrategy, OrganizeOptions, OrganizeStatus,
};
use musicforge_core::scan::restore_from_trash;

fn uniq_root(tag: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("mf-org-{tag}-{n}-{}-{}", seq, std::process::id()))
}

fn wav_bytes(sample_rate: u32, bits: u16) -> Vec<u8> {
    let data = vec![0u8; 512];
    let byte_rate = sample_rate * bits as u32 / 8;
    let block_align = bits / 8;
    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    v.extend_from_slice(b"WAVE");
    v.extend_from_slice(b"fmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&sample_rate.to_le_bytes());
    v.extend_from_slice(&byte_rate.to_le_bytes());
    v.extend_from_slice(&block_align.to_le_bytes());
    v.extend_from_slice(&bits.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&(data.len() as u32).to_le_bytes());
    v.extend_from_slice(&data);
    v
}

/// 给 WAV 写 RIFF INFO 标签（真实渲染输入：title/artist/album/track）。
fn tag_wav(path: &Path, title: &str, artist: &str, album: &str, track: u32) {
    use lofty::prelude::*;
    use lofty::tag::{Tag, TagType};
    let mut tagged = lofty::read_from_path(path).unwrap();
    let mut tag = Tag::new(TagType::RiffInfo);
    tag.insert_text(lofty::tag::ItemKey::TrackTitle, title.to_string());
    tag.insert_text(lofty::tag::ItemKey::TrackArtist, artist.to_string());
    tag.insert_text(lofty::tag::ItemKey::AlbumTitle, album.to_string());
    tag.set_track(track);
    tagged.insert_tag(tag);
    tagged
        .save_to_path(path, lofty::config::WriteOptions::default())
        .unwrap();
}

const TPL: &str = "{artist}/{album}/{track:02d} {title}";

#[test]
fn plan_renders_from_embedded_tags() {
    let root = uniq_root("render");
    let src = root.join("lib");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.wav"), wav_bytes(44100, 16)).unwrap();
    tag_wav(&src.join("a.wav"), "晴天", "周杰伦", "叶惠美", 1);

    let target = root.join("sorted");
    let plan = plan_organize(
        &src,
        &OrganizeOptions {
            template: TPL,
            target_root: &target,
            conflict: ConflictStrategy::Skip,
        },
    )
    .unwrap();
    assert_eq!(plan.items.len(), 1);
    let item = &plan.items[0];
    assert_eq!(item.status, OrganizeStatus::Planned);
    let t = item.target.to_string_lossy().replace('\\', "/");
    assert!(
        t.ends_with("sorted/周杰伦/叶惠美/01 晴天.wav"),
        "模板渲染必须与 convert 同语义（含目录段+零填充+原扩展名）: {t}"
    );

    // apply：文件落到模板结构，源消失
    let out = apply_organize_plan(&plan, "org-test").unwrap();
    assert_eq!(out.moved, 1);
    assert!(!src.join("a.wav").exists());
    assert!(item.target.exists());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn fallback_render_without_tags() {
    let root = uniq_root("fallback");
    let src = root.join("lib");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("no-tags.flac"), b"fake flac content").unwrap();

    let target = root.join("sorted");
    let plan = plan_organize(
        &src,
        &OrganizeOptions {
            template: TPL,
            target_root: &target,
            conflict: ConflictStrategy::Skip,
        },
    )
    .unwrap();
    let item = &plan.items[0];
    // 无标签 → Fallbacks::default()（未知艺术家/未知专辑/track 0），与 convert
    // 无元数据语义完全一致；仅当所有段都渲染为空才回退源文件名
    let t = item.target.to_string_lossy().replace('\\', "/");
    assert!(
        t.ends_with("sorted/未知艺术家/未知专辑/00 no-tags.flac"),
        "无标签回退语义须与 convert 一致: {t}"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn conflict_strategies_never_overwrite() {
    let root = uniq_root("conflict");
    let src = root.join("lib");
    let target = root.join("sorted");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(src.join("lib"), b"").ok(); // 占位，防根目录语义混乱
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("x.wav"), b"content-A").unwrap();
    // 预创建冲突目标（内容不同 → 真冲突）
    std::fs::write(target.join("x.wav"), b"content-B-ALREADY-HERE").unwrap();

    for (strategy, expect_status, expect_failed) in [
        (
            ConflictStrategy::Skip,
            OrganizeStatus::SkippedConflict,
            0usize,
        ),
        (
            ConflictStrategy::OverwriteNever,
            OrganizeStatus::ConflictNever,
            1usize,
        ),
    ] {
        let plan = plan_organize(
            &src,
            &OrganizeOptions {
                template: "{title}",
                target_root: &target,
                conflict: strategy,
            },
        )
        .unwrap();
        assert_eq!(plan.items[0].status, expect_status, "{strategy:?}");
        let out = apply_organize_plan(&plan, "org-conflict").unwrap();
        assert_eq!(out.moved, 0, "{strategy:?} 绝不移动");
        assert_eq!(out.failed, expect_failed, "{strategy:?}");
        // 双方文件都原封不动
        assert_eq!(std::fs::read(src.join("x.wav")).unwrap(), b"content-A");
        assert_eq!(
            std::fs::read(target.join("x.wav")).unwrap(),
            b"content-B-ALREADY-HERE"
        );
    }

    // suffix：目标改名为 (2)，原目标不被动
    let plan = plan_organize(
        &src,
        &OrganizeOptions {
            template: "{title}",
            target_root: &target,
            conflict: ConflictStrategy::Suffix,
        },
    )
    .unwrap();
    let item = &plan.items[0];
    assert_eq!(item.status, OrganizeStatus::Planned);
    assert!(
        item.target
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with("sorted/x (2).wav"),
        "suffix 应改名为 x (2).wav: {}",
        item.target.display()
    );
    let out = apply_organize_plan(&plan, "org-suffix").unwrap();
    assert_eq!(out.moved, 1);
    assert_eq!(
        std::fs::read(target.join("x (2).wav")).unwrap(),
        b"content-A",
        "原冲突目标内容不变"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn second_run_is_noop_and_rollback_restores() {
    let root = uniq_root("idempotent");
    let src = root.join("lib");
    let target = root.join("sorted");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("m.wav"), wav_bytes(44100, 16)).unwrap();

    fn opts(tr: &Path) -> OrganizeOptions<'_> {
        OrganizeOptions {
            template: "{title}",
            target_root: tr,
            conflict: ConflictStrategy::Skip,
        }
    }
    let plan1 = plan_organize(&src, &opts(&target)).unwrap();
    let out = apply_organize_plan(&plan1, "org-r1").unwrap();
    assert_eq!(out.moved, 1);

    // 幂等：对**目标目录**原地再规划（文件已在规范位置）→ 全部已在位，零移动。
    // （对源目录再规划当然为空——文件已经搬走了，那不是幂等性的正确口径）
    let plan2 = plan_organize(&target, &opts(&target)).unwrap();
    let c2 = plan2.counts();
    assert_eq!(c2.planned, 0, "二次规划不得再有移动项");
    assert_eq!(c2.in_place, 1);
    // 二次 apply 为空操作
    let out2 = apply_organize_plan(&plan2, "org-r2").unwrap();
    assert_eq!(out2.moved, 0);

    // 回滚整体还原（复用 clean --restore 的还原器）
    let rb = out.rollback_manifest.as_ref().unwrap();
    let restored = restore_from_trash(rb).unwrap();
    assert_eq!(restored, 1);
    assert!(src.join("m.wav").exists(), "还原后文件回原位");
    std::fs::remove_dir_all(&root).ok();
}
