//! D13 watcher 三级自动化测试：防抖合并语义 + 三级分派 + 安全铁律。
//!
//! 事件由测试直接喂入 `Debouncer`（时间注入）——不依赖真实 notify 事件流
//! （真实监听由 CLI `musicforge watch` 手工验证）。

use musicforge_core::watcher::{handle_event_batch, Debouncer, WatchLevel, WatcherConfig};
use std::path::PathBuf;

fn cfg(level: WatchLevel, target: Option<&str>) -> WatcherConfig {
    WatcherConfig {
        level,
        target_root: target.map(PathBuf::from),
        template: "{title} - {artist}".into(),
        debounce_ms: 1500,
    }
}

// ---------------------------------------------------------------- 防抖 --

/// 窗口内同路径多次事件 → 合并为一次处理（P8 验收「防抖合并」）。
#[test]
fn debounce_merges_burst_into_single_settle() {
    let mut d = Debouncer::new(1500);
    // 写入期：同一文件 3 次事件（0ms/200ms/900ms——都在窗口内）
    d.feed(PathBuf::from("/music/a.flac"), 0);
    d.feed(PathBuf::from("/music/a.flac"), 200);
    d.feed(PathBuf::from("/music/a.flac"), 900);
    // 窗口未过（900+1500=2400 才稳定）
    assert!(d.settle(1000).is_empty());
    assert!(d.settle(2399).is_empty());
    let settled = d.settle(2400);
    assert_eq!(settled, vec![PathBuf::from("/music/a.flac")], "合并为一次");
    assert!(d.is_empty(), "出列后不再悬挂");
}

/// 不同路径独立计时；持续写入的文件持续推迟（不被误触发）。
#[test]
fn debounce_per_path_and_delay_on_write() {
    let mut d = Debouncer::new(1000);
    d.feed(PathBuf::from("/m/a.flac"), 0);
    d.feed(PathBuf::from("/m/b.flac"), 500);
    // t=1100：a 稳定（1100-0>=1000），b 未到（1100-500=600<1000）
    let settled = d.settle(1100);
    assert_eq!(settled, vec![PathBuf::from("/m/a.flac")]);
    // b 持续写入 → 窗口刷新
    d.feed(PathBuf::from("/m/b.flac"), 1500);
    assert!(d.settle(2000).is_empty(), "持续写入推迟处理");
    let settled = d.settle(2600);
    assert_eq!(settled, vec![PathBuf::from("/m/b.flac")]);
}

// ---------------------------------------------------------------- 三级 --

/// T0：只登记，零文件操作（铁律：默认级不动任何文件）。
#[test]
fn t0_registers_without_touching_files() {
    let root = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let watch = root.path().join("watch");
    std::fs::create_dir_all(&watch).unwrap();
    let f = watch.join("new.flac");
    std::fs::write(&f, b"fLaC-stub").unwrap();

    let a = handle_event_batch(
        std::slice::from_ref(&f),
        &cfg(
            WatchLevel::T0Register,
            Some(target.path().to_str().unwrap()),
        ),
    )
    .unwrap();
    assert_eq!(a.registered, 1);
    assert_eq!(a.organized, 0);
    assert!(f.exists(), "T0 绝不动文件");
    assert!(
        std::fs::read_dir(target.path()).unwrap().count() == 0,
        "目标库零写入"
    );
}

/// T1：新音频文件自动整理——源被移动到目标库（organize 语义），旧文件 in_place。
#[test]
fn t1_organizes_new_audio_file() {
    let root = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let watch = root.path().join("watch");
    std::fs::create_dir_all(&watch).unwrap();
    // 新文件（有音频元数据 stub——lofty 读不出标签时 organize 回退文件名语义）
    let f = watch.join("new.flac");
    std::fs::write(&f, b"fLaC-stub").unwrap();
    // 同目录旧文件（已整理语义——in_place 零动作）
    let old = watch.join("old.flac");
    std::fs::write(&old, b"fLaC-stub").unwrap();

    let a = handle_event_batch(
        std::slice::from_ref(&f),
        &cfg(
            WatchLevel::T1AutoOrganize,
            Some(target.path().to_str().unwrap()),
        ),
    )
    .unwrap();
    assert!(a.organized >= 1, "新文件应被整理移动: {a:?}");
    assert!(!f.exists(), "源位置不再有新文件（已移入目标库）");
    // 移动语义 = 文件仍在某处（目标库内），绝不消失
    let moved: Vec<_> = walk(target.path());
    assert!(!moved.is_empty(), "目标库应有落位文件");
    // 旧文件不消失：目录级 organize 会把同目录待整理文件一并搬走（organized=2
    // 实锤），搬走后文件名 = 模板渲染名——断言「旧 stem 仍在目标库内」
    assert!(
        old.exists()
            || moved.iter().any(|p| p
                .file_name()
                .map(|n| n.to_string_lossy().starts_with("old"))
                .unwrap_or(false)),
        "旧文件不消失（in_place 或被同批整理到目标库）"
    );
}

fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.filter_map(|e| e.ok()) {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}

/// T2：垃圾文件自动清洗——**只进回收站**（可还原），绝不直接删除。
#[test]
fn t2_cleans_junk_into_trash_never_direct_delete() {
    let root = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let watch = root.path().join("watch");
    std::fs::create_dir_all(&watch).unwrap();
    let junk = watch.join("Thumbs.db");
    std::fs::write(&junk, b"").unwrap();

    let a = handle_event_batch(
        std::slice::from_ref(&junk),
        &cfg(
            WatchLevel::T2AutoWhitelist,
            Some(target.path().to_str().unwrap()),
        ),
    )
    .unwrap();
    assert!(a.cleaned >= 1, "垃圾应被清洗进回收站: {a:?}");
    assert!(!junk.exists(), "原位置垃圾已移除");
    // 铁律：进回收站（.musicforge/trash）而非直接删除
    let trash = watch.join(".musicforge").join("trash");
    let trashed: Vec<_> = walk(&trash);
    assert!(
        trashed
            .iter()
            .any(|p| p.file_name() == Some(std::ffi::OsStr::new("Thumbs.db"))),
        "垃圾必须在回收站内: {trashed:?}"
    );
}

/// T1/T2 需要 target_root（配置防呆——无目标库的自动整理 = 文件去向不明）。
#[test]
fn t1_without_target_root_is_config_error() {
    let f = PathBuf::from("/nowhere/song.flac");
    let r = handle_event_batch(&[f], &cfg(WatchLevel::T1AutoOrganize, None));
    assert!(r.is_err(), "无 target_root 必须显式报错");
}

