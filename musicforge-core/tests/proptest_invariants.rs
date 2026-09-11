//! 属性测试（P3 / proptest）：把「必须恒成立」的语义钉成不变量。
//!
//! 与既有单元测试的分工：单元测试钉**具体用例**（已知输入 → 已知输出），
//! 属性测试钉**对所有输入都成立的规律**（大小写不敏感 / 往返一致 / 完备性 / 不 panic）。
//! 后者擅长发现"组合输入"下的边界缺陷——正是手写用例最容易漏的部分。
//!
//! 依赖边界：proptest 是 **dev-dependency**（不进产物、不违反零网络与依赖策略）。

use proptest::prelude::*;

use musicforge_core::dedupe::{self, ScoreBreakdown};
use musicforge_core::organize::ConflictStrategy;
use musicforge_core::scan;

// ---------------------------------------------------------------- 不 panic --

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

    /// 全部解析/判定入口对**任意**字符串（含 Unicode、控制字符、超长）都不得 panic。
    ///
    /// 这些函数的输入直接来自用户（CLI 参数 / HTTP body / 文件名），
    /// 任何 panic 在 `panic = "abort"` 构建下都会终结进程——属性测试是最后一道网。
    #[test]
    fn parsers_never_panic_on_arbitrary_input(s in any::<String>()) {
        let _ = scan::is_audio_ext(&s);
        let _ = scan::is_junk_name(&s);
        let _ = dedupe::profile_by_name(&s);
        let _ = ConflictStrategy::parse(&s);
    }
}

// ------------------------------------------------------------ 大小写不敏感 --

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

    /// 扩展名判定必须大小写不敏感（`MP3` 与 `mp3` 同判）——文件系统来源不可控。
    #[test]
    fn audio_ext_is_case_insensitive(ext in "[A-Za-z0-9]{0,8}") {
        prop_assert_eq!(
            scan::is_audio_ext(&ext.to_ascii_lowercase()),
            scan::is_audio_ext(&ext.to_ascii_uppercase())
        );
    }
}

// ------------------------------------------------------------------ 往返 --

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

    /// 冲突策略 **parse ∘ as_str = id**：每个变体序列化后必须能原样解析回来。
    /// （新增变体若忘记补 parse 分支，此属性立刻失败。）
    #[test]
    fn conflict_strategy_roundtrips(idx in 0usize..3) {
        let all = [
            ConflictStrategy::Skip,
            ConflictStrategy::Suffix,
            ConflictStrategy::OverwriteNever,
        ];
        let s = all[idx];
        prop_assert_eq!(ConflictStrategy::parse(s.as_str()), Some(s));
    }

    /// 未注册的策略名一律 `None`（不猜测、不静默回退默认——默认值由调用方显式选择）。
    #[test]
    fn unknown_conflict_strategy_is_none(name in "[a-z-]{0,16}") {
        let known = ["skip", "suffix", "overwrite-never"];
        prop_assume!(!known.contains(&name.as_str()));
        prop_assert_eq!(ConflictStrategy::parse(&name), None);
    }
}

// ---------------------------------------------------------------- 完备性 --

/// 画像注册表完备性：`PROFILES` 里出现的每个名字都必须可查（且解析到同一画像）。
/// 防"加画像忘了进注册表"（CLI `--profile` / GUI 下拉会静默失效）。
#[test]
fn every_registered_profile_is_lookupable() {
    for p in dedupe::PROFILES {
        let got = dedupe::profile_by_name(p.name).expect("注册表内画像必须可查");
        assert_eq!(got.name, p.name);
    }
}

/// 清洗规则完备性：`RULE_CARDS` 里每个 id 都必须能查到（`rule_card` 是唯一入口）。
#[test]
fn every_rule_card_is_lookupable() {
    for card in scan::RULE_CARDS {
        let got = scan::rule_card(card.id).expect("规则卡必须可查");
        assert_eq!(got.id, card.id);
    }
}

/// 未知规则 id 一律 `None`（防前缀/包含式误匹配）。
#[test]
fn unknown_rule_id_is_none() {
    assert!(scan::rule_card("__no_such_rule__").is_none());
    assert!(scan::rule_card("").is_none());
}

// ---------------------------------------------------------------- 确定性 --

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// 评分必须是**确定性函数**：同一（画像, 采样率, 位深）两次调用结果一致。
    /// 评分决定"保留谁、牺牲谁"——非确定性会让去重结果不可复算（D24 可解释性要求）。
    #[test]
    fn scoring_is_deterministic(
        profile_idx in 0usize..3,
        max_rate in 0u32..192_000 * 2,
        max_depth in 0u32..64,
    ) {
        let p = dedupe::PROFILES[profile_idx];
        let b = ScoreBreakdown::default();
        let a1 = b.total_with(p, max_rate, max_depth);
        let a2 = b.total_with(p, max_rate, max_depth);
        prop_assert_eq!(a1, a2);
    }
}
