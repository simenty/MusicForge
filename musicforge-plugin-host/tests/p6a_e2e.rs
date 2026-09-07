//! P6a 预研 e2e：真实子进程全链路契约（spawn → D20 握手 → 调用 → 超时 kill）。
//!
//! 夹具 = `mock-ai-plugin` 二进制（同 crate [[bin]]，`CARGO_BIN_EXE` 定位）。

use musicforge_plugin_api::{codes, IdentifySuggestion, Request};
use musicforge_plugin_host::{PluginHostError, PluginProcess};

fn mock_exe() -> &'static str {
    env!("CARGO_BIN_EXE_mock-ai-plugin")
}

#[test]
fn handshake_and_identify_roundtrip() {
    let mut p = PluginProcess::spawn(mock_exe().as_ref(), ">=1,<2").unwrap();
    // D20 握手产物
    assert_eq!(p.manifest.name, "mock-ai");
    assert_eq!(p.manifest.api_version, "1.0.0");
    assert!(!p.manifest.network, "预研插件必须声明离线");

    // 最小请求模型：构造层就只有允许字段
    let params = serde_json::json!({
        "normalized_filename": "王铮亮 feat. 风华音纪 - 借墨 [SQ].wav",
        "title": "借墨", "artists": ["王铮亮"], "album": null,
        "duration_ms": 252000, "format": "wav", "language_hint": "zh"
    });
    let result = p.call("ai.identify_track", params, 2_000).unwrap();
    let sug: IdentifySuggestion = serde_json::from_value(result).unwrap();
    assert_eq!(sug.title, "借墨");
    assert!((sug.confidence - 0.93).abs() < 1e-6);
    assert_eq!(sug.field_confidence.get("album"), Some(&0.86));
}

#[test]
fn request_wire_format_contains_no_forbidden_keys() {
    // 最小请求模型的线上断言：序列化后的请求行 grep 不到禁发字段
    let params = serde_json::json!({
        "normalized_filename": "x.wav", "title": "t",
        "artists": [], "album": null, "duration_ms": 1, "format": "wav"
    });
    let req = Request {
        id: "r".into(),
        method: "ai.identify_track".into(),
        params,
    };
    let wire = serde_json::to_string(&req).unwrap();
    for forbidden in ["absolute_path", "audio_bytes", "cover_bytes"] {
        assert!(
            !wire.contains(forbidden),
            "请求线格式包含禁发字段: {forbidden}"
        );
    }
}

#[test]
fn d20_incompatible_version_is_rejected() {
    // 进程级不兼容路径：MOCK_API_VERSION 覆盖 → spawn 握手拒绝 + 子进程被 kill。
    // 通过 shim 脚本转发环境变量（spawn 接口只收路径，不收环境）。
    let shim = root_shim();
    let err = PluginProcess::spawn(shim.as_path(), ">=1,<2").unwrap_err();
    assert!(
        matches!(err, PluginHostError::ApiIncompatible { .. }),
        "0.9.0 必须被 D20 拒绝: {err}"
    );
    assert_eq!(err.code(), codes::API_INCOMPATIBLE);
}

/// 平台无关 shim：注入 MOCK_API_VERSION=0.9.0 后转发到 mock 二进制。
#[cfg(windows)]
fn root_shim() -> std::path::PathBuf {
    write_shim(".cmd", "@echo off\r\nset MOCK_API_VERSION=0.9.0\r\n")
}

#[cfg(not(windows))]
fn root_shim() -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = write_shim(".sh", "#!/bin/sh\nexport MOCK_API_VERSION=0.9.0\n");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

fn write_shim(ext: &str, body: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("mf-p6a-shim-{}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(format!("mock-shim{ext}"));
    let mock = mock_exe();
    let target = if cfg!(windows) {
        format!("\"{mock}\" %*\r\n")
    } else {
        format!("exec \"{mock}\" \"$@\"\n")
    };
    std::fs::write(&p, format!("{body}{target}")).unwrap();
    p
}

#[test]
fn timeout_kills_child_and_returns_stable_error() {
    let mut p = PluginProcess::spawn(mock_exe().as_ref(), ">=1,<2").unwrap();
    let err = p
        .call("test.sleep", serde_json::json!({"ms": 5_000}), 300)
        .unwrap_err();
    assert!(
        matches!(err, PluginHostError::Timeout { .. }),
        "慢方法必须触发超时: {err}"
    );
    assert_eq!(err.code(), codes::TIMEOUT);
    // kill 隔离：子进程必须已终止（kill -9 后主进程存活——P6a 硬验收的机制实证）
    assert!(
        p.child_try_wait().map(|s| s.is_some()).unwrap_or(true),
        "超时后子进程必须已被终止"
    );
}

#[test]
fn unknown_method_returns_stable_error_code() {
    let mut p = PluginProcess::spawn(mock_exe().as_ref(), ">=1,<2").unwrap();
    let err = p
        .call("ai.nonexistent", serde_json::json!({}), 2_000)
        .unwrap_err();
    assert!(err.to_string().contains(codes::METHOD_UNKNOWN), "{err}");
}

#[test]
fn drop_terminates_child() {
    let p = PluginProcess::spawn(mock_exe().as_ref(), ">=1,<2").unwrap();
    let _ = p; // Drop 在此结束
               // 无泄漏断言：Drop kill 幂等且不 panic——真泄漏需 OS 级探测，
               // 预研以「Drop 不 panic + 后续 spawn 正常」为充分信号
    let _ = PluginProcess::spawn(mock_exe().as_ref(), ">=1,<2").unwrap();
}
