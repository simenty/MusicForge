//! P1 曲库索引层（library.rs）测试。
//!
//! 使用 hound 生成真实 WAV（core 的传递依赖，测试可直接使用）：
//! 覆盖索引链路（属性读取 → 入库 → 聚合）、无标签退化、
//! 损坏文件降级、重扫后陈旧行清理。

use std::path::Path;

use musicforge_core::db::Db;
use musicforge_core::library::index_library;
use musicforge_core::scan::ScanOptions;

/// 生成 0.5 秒 8kHz 单声道 16bit WAV。
fn make_wav(path: &Path, frames: usize) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 8000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).expect("create wav");
    for i in 0..frames {
        w.write_sample((i % 100) as i16).expect("write sample");
    }
    w.finalize().expect("finalize wav");
}

fn opts() -> ScanOptions {
    ScanOptions {
        recursive: true,
        ..Default::default()
    }
}

/// 索引链路：属性真实读取（时长/采样率/声道）+ 无标签回退到文件名 + 聚合。
#[test]
fn index_reads_properties_and_aggregates() {
    let dir = tempfile::tempdir().unwrap();
    make_wav(&dir.path().join("track-one.wav"), 4000);
    make_wav(&dir.path().join("track-two.wav"), 4000);

    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source(dir.path().to_str().unwrap(), None).unwrap();
    let out = index_library(&db, sid, dir.path(), &opts(), false).unwrap();

    assert_eq!(out.audio, 2);
    assert_eq!(out.indexed, 2);
    assert_eq!(out.failed, 0);
    assert_eq!(out.tagged, 0, "测试 WAV 无标签");
    assert_eq!(out.untagged, 2);
    assert_eq!(out.removed, 0);

    let tracks = db.list_tracks(10, 0).unwrap();
    assert_eq!(tracks.len(), 2);
    let first = &tracks[0];
    assert_eq!(first.title.as_deref(), Some("track-one"), "无标签时 title 退化为文件名");
    assert_eq!(first.sample_rate, Some(8000), "采样率来自真实属性读取");
    assert_eq!(first.channels, Some(1));
    assert_eq!(first.bit_depth, Some(16));
    let d = first.duration_ms.unwrap();
    assert!((400..=600).contains(&d), "0.5s 素材时长应在合理区间，实际 {d}ms");
    assert!(first.is_lossless, "wav 在识别集合内 → 应为无损容器");

    let st = db.library_stats().unwrap();
    assert_eq!(st.tracks, 2);
    assert!(st.total_size > 0);
    assert!(st.total_duration_ms > 0);
}

/// 平滑降级：损坏的「音频文件」（虚假扩展名）不中止索引，仅基础字段入库。
#[test]
fn corrupt_audio_degrades_without_aborting() {
    let dir = tempfile::tempdir().unwrap();
    make_wav(&dir.path().join("good.wav"), 4000);
    std::fs::write(dir.path().join("broken.flac"), b"this is not a flac").unwrap();

    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source(dir.path().to_str().unwrap(), None).unwrap();
    let out = index_library(&db, sid, dir.path(), &opts(), false).unwrap();

    assert_eq!(out.audio, 2);
    assert_eq!(out.indexed, 2, "损坏文件也必须入库（基础字段）");
    assert_eq!(out.failed, 1);
    assert_eq!(db.count_tracks().unwrap(), 2);

    let broken = db
        .list_tracks(10, 0)
        .unwrap()
        .into_iter()
        .find(|t| t.path.ends_with("broken.flac"))
        .expect("损坏文件应在库中");
    assert_eq!(broken.title.as_deref(), Some("broken"));
    assert_eq!(broken.format.as_deref(), Some("flac"));
    assert!(broken.is_lossless, "flac 扩展名 → 格式级无损标记");
    assert!(broken.duration_ms.is_none(), "读取失败 → 时长缺失");
}

/// 重扫清理：磁盘上已删除的文件，下一次索引后从库中消失。
#[test]
fn rescan_removes_deleted_files() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.wav");
    make_wav(&a, 4000);
    make_wav(&dir.path().join("b.wav"), 4000);

    let db = Db::open_in_memory().unwrap();
    let sid = db.upsert_source(dir.path().to_str().unwrap(), None).unwrap();
    index_library(&db, sid, dir.path(), &opts(), false).unwrap();
    assert_eq!(db.count_tracks().unwrap(), 2);

    std::fs::remove_file(&a).unwrap();
    // run_id 为秒级：连续两次索引可能同秒——先睡过 1 秒边界确保 run 标记可比
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let out = index_library(&db, sid, dir.path(), &opts(), false).unwrap();

    assert_eq!(out.removed, 1, "已删除的 a.wav 应被清理");
    assert_eq!(db.count_tracks().unwrap(), 1);
    assert!(db.list_tracks(10, 0).unwrap()[0].path.ends_with("b.wav"));
}
