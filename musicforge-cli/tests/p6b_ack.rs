//! P6b 验收回归：ACK 闸 / 插件管理配置面 / 默认构建响亮降级。

use musicforge_cli::plugins;

/// ACK 闸纯函数：ack_required 且未确认 → 拒绝；确认后放行；无申报不拦。
#[test]
fn ack_gate_blocks_until_acknowledged() {
    // 高风险未确认 → 拒绝，码可辨
    let err = plugins::ack_gate(true, "kwm-migration", &[]).unwrap_err();
    assert_eq!(err.mf_code(), "MF-PLUGIN-ACK-REQUIRED");
    // 确认后放行
    assert!(plugins::ack_gate(true, "kwm-migration", &["kwm-migration".into()]).is_ok());
    // 无申报（低风险）→ 永不拦截
    assert!(plugins::ack_gate(false, "ai-openai-compatible", &[]).is_ok());
}

/// acknowledge：幂等追加 + config 持久化；空名显式拒绝。
#[test]
fn acknowledge_roundtrip_and_rejects_empty_name() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("nested").join("config.json");

    plugins::acknowledge(&cfg_path, "kwm-migration").unwrap();
    plugins::acknowledge(&cfg_path, "kwm-migration").unwrap(); // 幂等
    let cfg = musicforge_core::config::AppConfig::load(&cfg_path).unwrap();
    assert_eq!(cfg.plugins.acked, vec!["kwm-migration".to_string()]);

    let err = plugins::acknowledge(&cfg_path, "  ").unwrap_err();
    assert_eq!(err.mf_code(), "MF-CONFIG-INVALID");
    // 被拒绝的调用不得改写既有 acked
    let cfg2 = musicforge_core::config::AppConfig::load(&cfg_path).unwrap();
    assert_eq!(cfg2.plugins.acked.len(), 1);
}

/// set_enabled：整表覆盖语义（启用 → 全禁）。
#[test]
fn set_enabled_overrides_whole_table() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.json");

    // B6：trim 生效 + 重复名拒绝（GUI 同款语义：显式失败优于静默去重）
    plugins::set_enabled(&cfg_path, &["  a  ".into(), "b".into()]).unwrap();
    let cfg = musicforge_core::config::AppConfig::load(&cfg_path).unwrap();
    assert_eq!(
        cfg.plugins.enabled,
        vec!["a".to_string(), "b".to_string()],
        "trim 后按序写入"
    );

    let err = plugins::set_enabled(&cfg_path, &["c".into(), "c".into()]).unwrap_err();
    assert!(
        err.to_string().contains("重复的插件名"),
        "重复名必须显式拒绝: {err}"
    );

    plugins::set_enabled(&cfg_path, &[]).unwrap();
    let cfg = musicforge_core::config::AppConfig::load(&cfg_path).unwrap();
    assert!(cfg.plugins.enabled.is_empty(), "全禁 = 离线铁律恢复");

    // 空名拒绝 + 被拒调用不改写既有配置
    plugins::set_enabled(&cfg_path, &["x".into(), "y".into()]).unwrap();
    let err = plugins::set_enabled(&cfg_path, &["  ".into()]).unwrap_err();
    assert_eq!(err.mf_code(), "MF-CONFIG-INVALID");
    let cfg2 = musicforge_core::config::AppConfig::load(&cfg_path).unwrap();
    assert_eq!(cfg2.plugins.enabled.len(), 2, "被拒调用不得改写配置");
}

/// status：config 状态 + 白名单清单装配（坏清单跳过，与 GUI 同源语义）。
#[test]
fn status_includes_config_and_installed() {
    let base = tempfile::tempdir().unwrap();
    let pdir = base.path().join("plugins").join("mock-ai");
    std::fs::create_dir_all(&pdir).unwrap();
    std::fs::write(
        pdir.join("plugin.json"),
        r#"{"name":"mock-ai","api_version":"1.0.0","kind":"ai","network":false}"#,
    )
    .unwrap();

    let cfg = musicforge_core::config::AppConfig::default();
    let v = plugins::status(&cfg, &[base.path().join("plugins")]);
    assert_eq!(v["enabled"], serde_json::json!([]));
    assert_eq!(v["installed"].as_array().unwrap().len(), 1);
    assert_eq!(v["installed"][0]["name"], "mock-ai");
}

/// 默认构建（无 plugin-host）：format_migrate 必须响亮报 MF-PLUGIN-NOT-FOUND，
/// 绝不静默装作执行过（G5 教训 / B4 同源原则）。
#[cfg(not(feature = "plugin-host"))]
#[test]
fn format_migrate_without_runtime_fails_loudly() {
    let err = musicforge_cli::format_migrate("kwm-migration", "s", "o", None, None).unwrap_err();
    assert_eq!(err.mf_code(), "MF-PLUGIN-NOT-FOUND", "{err}");
}
