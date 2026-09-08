//! P6b.2 桥接回归（feature = plugin-host，公开 API 面）：
//! common_work_root 收敛 / 注册表装配门槛三连。
//! 桥接规划→执行全流程（含暂存改名）见 lib.rs 单元测试（私有路径）。

#![cfg(feature = "plugin-host")]

use std::path::Path;

use musicforge_cli::format_bridge;

/// common_work_root：公共祖先收敛；跨前缀显式 None（绝不放宽边界）。
#[test]
fn common_work_root_finds_ancestor_or_none() {
    let a = Path::new("C:/music/lib/song.kwm");
    let b = Path::new("C:/music/out");
    assert_eq!(
        format_bridge::PluginFormatAdapter::common_work_root(a, b).unwrap(),
        Path::new("C:/music")
    );
    // 完全相同 → 自身
    assert_eq!(
        format_bridge::PluginFormatAdapter::common_work_root(a, a).unwrap(),
        Path::new("C:/music/lib/song.kwm")
    );
    // 跨盘 → None
    assert!(format_bridge::PluginFormatAdapter::common_work_root(
        Path::new("C:/a/x"),
        Path::new("D:/b/y")
    )
    .is_none());
}
