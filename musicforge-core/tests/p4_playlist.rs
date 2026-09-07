//! P4.3：播放清单（按分类导出 + 导入失效路径修复）。
//!
//! 验收口径（对齐 v2.4 §P4 能力 #12）：
//! - 按分类导出：每组一个 .m3u8（UTF-8 + #EXTM3U + #EXTINF），条目存在；
//! - 导入：有效条目直接命中；失效条目按「同名 → 时长 ±1s 消歧」修复；
//! - 修复不了的条目以 # FAIL 注释保留（审计不丢行），绝不静默丢弃；
//! - 全程不改动任何音乐文件。

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use musicforge_core::playlist::{export_playlists, import_and_repair, GroupBy};

fn uniq_root(tag: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("mf-pl-{tag}-{n}-{}-{}", seq, std::process::id()))
}

/// 指定数据字节数的 WAV（时长 = bytes/2/44100 秒）。
fn wav_bytes_with_len(sample_rate: u32, bits: u16, data_len: usize) -> Vec<u8> {
    let data = vec![0u8; data_len];
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

fn tag_wav(path: &Path, title: &str, artist: &str) {
    use lofty::prelude::*;
    use lofty::tag::{Tag, TagType};
    let mut tagged = lofty::read_from_path(path).unwrap();
    let mut tag = Tag::new(TagType::RiffInfo);
    tag.insert_text(lofty::tag::ItemKey::TrackTitle, title.to_string());
    tag.insert_text(lofty::tag::ItemKey::TrackArtist, artist.to_string());
    tagged.insert_tag(tag);
    tagged
        .save_to_path(path, lofty::config::WriteOptions::default())
        .unwrap();
}

#[test]
fn export_by_artist_writes_groups_with_valid_entries() {
    let root = uniq_root("exp");
    let lib = root.join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("a.wav"), wav_bytes_with_len(44100, 16, 88200)).unwrap();
    tag_wav(&lib.join("a.wav"), "晴天", "周杰伦");
    std::fs::write(lib.join("b.wav"), wav_bytes_with_len(44100, 16, 88200)).unwrap();
    tag_wav(&lib.join("b.wav"), "七里香", "周杰伦");
    std::fs::write(lib.join("c.wav"), wav_bytes_with_len(44100, 16, 88200)).unwrap();
    tag_wav(&lib.join("c.wav"), "绿光", "孙燕姿");

    let out = root.join("pls");
    let rep = export_playlists(&lib, &out, GroupBy::Artist).unwrap();
    assert_eq!(rep.files_seen, 3);
    assert_eq!(rep.playlists.len(), 2, "两位艺术家 → 两个清单");

    let zjl = out.join("周杰伦.m3u8");
    assert!(zjl.exists(), "清单应存在: {}", zjl.display());
    let text = std::fs::read_to_string(&zjl).unwrap();
    assert!(text.starts_with("#EXTM3U\n"));
    assert_eq!(
        text.matches("#EXTINF:1,").count(),
        2,
        "两个 1s WAV（88200B→44100 样本→1s）"
    );
    assert!(text.contains("#EXTINF:1,晴天"));
    // 条目路径必须在磁盘上存在
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let p = out.join(line);
        assert!(p.exists(), "相对条目应存在: {line}");
    }
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn export_none_writes_single_library_playlist() {
    let root = uniq_root("none");
    let lib = root.join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("x.wav"), wav_bytes_with_len(44100, 16, 88200)).unwrap();
    let out = root.join("pls");
    let rep = export_playlists(&lib, &out, GroupBy::None).unwrap();
    assert_eq!(rep.playlists.len(), 1);
    assert!(out.join("library.m3u8").exists());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn import_roundtrip_all_ok() {
    let root = uniq_root("round");
    let lib = root.join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("a.wav"), wav_bytes_with_len(44100, 16, 88200)).unwrap();
    std::fs::write(lib.join("b.wav"), wav_bytes_with_len(44100, 16, 176400)).unwrap();

    let out = root.join("pls");
    export_playlists(&lib, &out, GroupBy::None).unwrap();
    let list = out.join("library.m3u8");

    let rep = import_and_repair(&list, &lib, None).unwrap();
    assert_eq!(rep.total_entries, 2);
    assert_eq!(rep.ok, 2, "导出后立即导入应全部命中");
    assert!(rep.repaired.is_empty() && rep.unresolved.is_empty());
    assert!(rep.written.unwrap().exists());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn import_repairs_broken_paths_with_duration_disambiguation() {
    let root = uniq_root("repair");
    // 两个同名 song.wav，时长不同（0.5s vs 2s）——时长用于消歧
    let lib1 = root.join("lib1");
    let lib2 = root.join("lib2");
    std::fs::create_dir_all(&lib1).unwrap();
    std::fs::create_dir_all(&lib2).unwrap();
    std::fs::write(lib1.join("song.wav"), wav_bytes_with_len(44100, 16, 176400)).unwrap(); // 2s
    std::fs::write(lib2.join("song.wav"), wav_bytes_with_len(44100, 16, 44100)).unwrap(); // 0.5s→0s
                                                                                          // 第三首歌彻底丢失 → unresolved
    let search = root.join("searchroot");
    std::fs::create_dir_all(&search).unwrap();
    std::fs::rename(lib1.join("song.wav"), search.join("song.wav")).unwrap();
    // 简化：搜索根 = 根目录下 searchroot（含 2s song.wav）；lib2 的 0.5s 也在根外
    // → 重建结构：searchroot 内放两个同名文件 + 一个丢失去失条目
    std::fs::create_dir_all(search.join("sub")).unwrap();
    std::fs::write(
        search.join("sub").join("song.wav"),
        wav_bytes_with_len(44100, 16, 44100),
    )
    .unwrap(); // 0s

    let list = root.join("broken.m3u8");
    // 失效条目用**本平台的**不存在的绝对路径（Linux 上反斜杠不是分隔符，
    // 写死 Windows 形态会让 file_name 解析成整串——CI ubuntu 实测暴露）
    let dead_a = if cfg!(windows) {
        "Q:\\dead\\link\\song.wav"
    } else {
        "/dead/link/song.wav"
    };
    let dead_b = if cfg!(windows) {
        "Q:\\dead\\lost.wav"
    } else {
        "/dead/lost.wav"
    };
    std::fs::write(
        &list,
        format!("#EXTM3U\n#EXTINF:2,song long\n{dead_a}\n#EXTINF:1,lost\n{dead_b}\n"),
    )
    .unwrap();

    let rep = import_and_repair(&list, &search, None).unwrap();
    assert_eq!(rep.total_entries, 2);
    assert_eq!(
        rep.repaired.len(),
        1,
        "时长 2s 应唯一定位到 searchroot/song.wav"
    );
    let (_, fixed) = &rep.repaired[0];
    assert!(
        fixed
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with("searchroot/song.wav"),
        "应修复到时长匹配的候选: {}",
        fixed.display()
    );
    assert_eq!(rep.unresolved.len(), 1, "彻底丢失的条目应 unresolved");

    // 修复后清单：unresolved 以 # FAIL 注释保留（审计不丢行）
    let fixed_text = std::fs::read_to_string(rep.written.as_ref().unwrap()).unwrap();
    assert!(
        fixed_text.contains(&format!("# FAIL {dead_b}")),
        "{fixed_text}"
    );
    assert!(fixed_text.contains("#EXTINF:2,song long"));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn import_resolves_relative_entries_against_playlist_dir() {
    let root = uniq_root("rel");
    let pl_dir = root.join("pls");
    std::fs::create_dir_all(pl_dir.join("music")).unwrap();
    std::fs::write(
        pl_dir.join("music").join("a.wav"),
        wav_bytes_with_len(44100, 16, 88200),
    )
    .unwrap();
    let list = pl_dir.join("list.m3u8");
    std::fs::write(&list, "#EXTM3U\n#EXTINF:1,a\nmusic/a.wav\n").unwrap();

    let rep = import_and_repair(&list, &root, None).unwrap();
    assert_eq!(rep.ok, 1, "相对条目应按清单所在目录解析");
    assert!(rep.repaired.is_empty());
    std::fs::remove_dir_all(&root).ok();
}
