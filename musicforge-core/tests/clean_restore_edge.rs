//! 稳定审计（2026-09-08）回归：回收站碰撞与整体还原边界（B2/B3）。
//!
//! 场景全部自构造；复用 P3 清洗执行器与还原器，不触碰 qa_yan 保护文件。

use std::path::{Path, PathBuf};

use musicforge_core::scan::{apply_clean_plan, restore_from_trash, CleanAction, CleanPlan};

fn uniq_root(tag: &str) -> PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("mf-cleanedge-{tag}-{n}-{}", std::process::id()))
}

fn plan_for(root: &Path, path: &Path) -> CleanPlan {
    CleanPlan {
        actions: vec![CleanAction {
            path: path.to_path_buf(),
            rule_id: "MF-CLEAN-001",
        }],
        empty_dirs: Vec::new(),
        trash_root: root.join(".musicforge/trash"),
        scan_root: root.to_path_buf(),
    }
}

/// B3 回归：回收站内同一落位撞名（同 task 重放形态）→ 第二次必须以 " (2)"
/// 后缀进回收站（旧实现 rename 撞名失败 → 整批中断）。回收站按
/// `<trash>/<task_id>/` 分任务，CLI 每次新 task_id 时本分支是纯防御；
/// 本测试用同一 task_id 两次触发碰撞分支钉死行为。
#[test]
fn second_clean_of_same_name_goes_to_suffixed_trash_slot() {
    let root = uniq_root("b3");
    std::fs::create_dir_all(&root).unwrap();
    let item = root.join("Thumbs.db");
    let task = "t-b3-same";

    // 第一次清洗
    std::fs::write(&item, b"first").unwrap();
    let out1 = apply_clean_plan(&plan_for(&root, &item), task).unwrap();
    assert_eq!(out1.moved, 1);
    assert!(root
        .join(".musicforge/trash")
        .join(task)
        .join("Thumbs.db")
        .exists());

    // 同一 task 重放（还原后重跑等场景的碰撞形态）→ (2) 后缀落位
    std::fs::write(&item, b"second").unwrap();
    let out2 = apply_clean_plan(&plan_for(&root, &item), task).unwrap();
    assert_eq!(out2.moved, 1, "回收站撞名必须以 (2) 后缀落位，不得中断");
    assert!(root
        .join(".musicforge/trash")
        .join(task)
        .join("Thumbs (2).db")
        .exists());
    assert_eq!(
        std::fs::read(root.join(".musicforge/trash").join(task).join("Thumbs.db")).unwrap(),
        b"first",
        "第一次的牺牲项内容不变"
    );
    assert_eq!(
        std::fs::read(
            root.join(".musicforge/trash")
                .join(task)
                .join("Thumbs (2).db")
        )
        .unwrap(),
        b"second"
    );

    // 审计行 from 必须是实际落位（含后缀），保证还原可达
    let rb = std::fs::read_to_string(out2.rollback_manifest.unwrap()).unwrap();
    assert!(rb.contains("Thumbs (2).db"), "审计行必须指向实际落位: {rb}");

    std::fs::remove_dir_all(&root).ok();
}

/// B2 回归：还原时目标已被占用（用户重建同名文件）→ 邻位 "(restored)" 落盘，
/// 绝不覆盖占用者，也绝不中断；回收站缺文件的行跳过不中断。
#[test]
fn restore_with_occupied_target_goes_to_sibling_and_skips_missing() {
    let root = uniq_root("b2");
    std::fs::create_dir_all(&root).unwrap();
    let item = root.join("Thumbs.db");

    // 清洗两次（两次同路径，靠 B3 后缀区分）
    std::fs::write(&item, b"first").unwrap();
    let out1 = apply_clean_plan(&plan_for(&root, &item), "t-b2-1").unwrap();
    std::fs::write(&item, b"second").unwrap();
    let out2 = apply_clean_plan(&plan_for(&root, &item), "t-b2-2").unwrap();
    let rb1 = out1.rollback_manifest.clone().unwrap();

    // 还原第一批：目标位空闲 → 原位还原
    let n1 = restore_from_trash(&rb1).unwrap();
    assert_eq!(n1, 1);
    assert_eq!(std::fs::read(&item).unwrap(), b"first");

    // 还原第二批：目标位已被第一批占用 → 必须邻位 "(restored)" 落盘（旧实现中断）
    let rb2 = out2.rollback_manifest.unwrap();
    let n2 = restore_from_trash(&rb2).unwrap();
    assert_eq!(n2, 1, "还原碰撞必须邻位落盘，不得失败: {rb2:?}");
    assert_eq!(std::fs::read(&item).unwrap(), b"first", "占用者内容不变");
    assert_eq!(
        std::fs::read(root.join("Thumbs (restored).db")).unwrap(),
        b"second",
        "第二批还原到邻位"
    );

    // 回收站缺文件（第一批已还原走 → from 自然缺失）→ 跳过该行，不中断
    let n3 = restore_from_trash(&rb1).unwrap();
    assert_eq!(n3, 0, "from 缺失 → 跳过（旧实现 Err 中断）");

    std::fs::remove_dir_all(&root).ok();
}
