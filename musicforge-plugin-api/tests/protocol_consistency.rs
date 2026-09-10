//! P9 先行：协议一致性套件（SDK v1 冻结）。
//!
//! **冻结语义**：本套件钉死协议 v0.1（`v1` 模块）的全部外部可见契约——
//! 任何值变更都会让这里的断言变红，从而**强制走显式的协议修订流程**
//! （更新 docs/plugin-protocol.md §13 修订记录 + 本套件同步 + 双插件仓
//! 协同发版评估 R26），杜绝静默漂移。
//!
//! 对照物 = `docs/plugin-protocol.md`（v0.1-draft）：
//! - §4.1 plugin.init 握手 / §5 方法集 / §6 事件 / §7 权限模型 / §8 错误模型
//! - §13 版本治理（HOST_API_MAJOR=1 区间协商 D20）

use musicforge_plugin_api::{codes, events, methods, v1, PluginError, PluginKind, PluginManifest, PluginPermissions, Request, Response};

// ---------------------------------------------------------------- 常量冻结 --

/// §13/D20：宿主 API major——major 变更 = 破坏性协议演进（需全插件协同）。
#[test]
fn host_api_major_frozen() {
    assert_eq!(codes::HOST_API_MAJOR, 1);
}

/// §4.1：协议版本 v0.1 —— 握手协商的锚（legacy 降级兼容 v0.7.0-0.8.0）。
#[test]
fn protocol_version_frozen() {
    assert_eq!(v1::PROTOCOL_VERSION, 1);
}

/// §10 安全规则：单行 16MB 上限（P2 裁决——防巨型 JSON 打爆解析器）。
#[test]
fn message_size_limit_frozen() {
    assert_eq!(v1::MAX_MESSAGE_BYTES, 16 * 1024 * 1024);
}

/// 超时姿态冻结（§4.1 init 10s / §5.4 migrate 10min / 停机宽限 5s / 空闲回收 5min）。
#[test]
fn timeout_constants_frozen() {
    assert_eq!(v1::INIT_TIMEOUT_MS, 10_000);
    assert_eq!(v1::MIGRATE_TIMEOUT_MS, 600_000);
    assert_eq!(v1::SHUTDOWN_GRACE_MS, 5_000);
    assert_eq!(v1::IDLE_RECYCLE_MS, 5 * 60_000);
}

/// §4.2 崩溃禁用阈值：连续 3 次（Handshake/Timeout/Protocol/Gone）本会话禁用。
#[test]
fn crash_disable_threshold_frozen() {
    assert_eq!(v1::CRASH_DISABLE_THRESHOLD, 3);
}

// ---------------------------------------------------------------- 方法集 --

/// §5 方法集全集（15 个）——新增/删除方法必须先改协议文档再动这里（冻结）。
const PROTOCOL_METHODS: &[(&str, &str)] = &[
    (methods::PLUGIN_MANIFEST, "plugin"),
    (methods::PLUGIN_HEALTH, "plugin"),
    (methods::PLUGIN_SHUTDOWN, "plugin"),
    (methods::PLUGIN_INIT, "plugin"),
    (methods::AI_IDENTIFY_TRACK, "ai"),
    (methods::AI_IDENTIFY_TRACKS, "ai"),
    (methods::AI_GENERATE_FILENAME_REGEX, "ai"),
    (methods::AI_REVIEW_DUPLICATE_GROUP, "ai"),
    (methods::LYRICS_VERIFY, "lyrics"),
    (methods::LYRICS_SEARCH, "lyrics"),
    (methods::COVER_SEARCH, "cover"),
    (methods::COVER_GENERATE, "cover"),
    (methods::FORMAT_MIGRATE, "format"),
    (methods::FORMAT_PROBE, "format"),
    (methods::FORMAT_VALIDATE, "format"),
    (methods::FORMAT_CANCEL, "format"),
];

/// 方法集：数量、命名空间前缀、无重复——与文档 §5 五域一一对应。
#[test]
fn method_registry_matches_protocol_doc() {
    // §5.1 plugin.*（4）§5.2 ai.*（4）§5.3 lyrics/cover（2+2）§5.4 format.*（4）
    let by_ns = |ns: &str| PROTOCOL_METHODS.iter().filter(|(_, n)| *n == ns).count();
    assert_eq!(by_ns("plugin"), 4);
    assert_eq!(by_ns("ai"), 4);
    assert_eq!(by_ns("lyrics"), 2);
    assert_eq!(by_ns("cover"), 2);
    assert_eq!(by_ns("format"), 4);
    assert_eq!(PROTOCOL_METHODS.len(), 16);
    // 每个常量的字面值必须以自己的命名空间开头（防复制粘贴串域）
    for (method, ns) in PROTOCOL_METHODS {
        assert!(
            method.starts_with(&format!("{ns}.")),
            "方法 {method} 的字面值与命名空间 {ns} 不符"
        );
    }
    // 无重复字面值
    let mut names: Vec<_> = PROTOCOL_METHODS.iter().map(|(m, _)| *m).collect();
    names.sort_unstable();
    let before = names.len();
    names.dedup();
    assert_eq!(names.len(), before, "方法字面值存在重复");
}

/// §5.4：format 域方法字面值精确冻结（高风险域——插件/宿主双侧硬编码）。
#[test]
fn format_methods_frozen() {
    assert_eq!(methods::FORMAT_MIGRATE, "format.migrate");
    assert_eq!(methods::FORMAT_PROBE, "format.probe");
    assert_eq!(methods::FORMAT_VALIDATE, "format.validate");
    assert_eq!(methods::FORMAT_CANCEL, "format.cancel");
}

/// §6 事件（X39）：无 id 消息路由的两个事件名冻结。
#[test]
fn event_names_frozen() {
    assert_eq!(events::PROGRESS, "event.progress");
    assert_eq!(events::LOG, "event.log");
}

/// §8/X42：错误稳定码冻结（传输/协议层 MF-PLUGIN-*；业务码走 source_code 透传）。
#[test]
fn error_codes_frozen() {
    assert_eq!(codes::API_INCOMPATIBLE, "MF-PLUGIN-API-INCOMPATIBLE");
    assert_eq!(codes::TIMEOUT, "MF-PLUGIN-TIMEOUT");
    assert_eq!(codes::FAILED, "MF-PLUGIN-FAILED");
    assert_eq!(codes::METHOD_UNKNOWN, "MF-PLUGIN-METHOD-UNKNOWN");
    assert_eq!(codes::MANIFEST_INVALID, "MF-PLUGIN-MANIFEST-INVALID");
}

// ------------------------------------------------------------ 信封形状 --

/// 键集合比较辅助（serde_json Map 为字母序——冻结语义 = 集合而非顺序）。
fn sorted_keys(v: &serde_json::Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

/// §3 消息模型：请求信封序列化键集 = {id, method, params}（强类型 NDJSON）。
#[test]
fn request_envelope_shape_frozen() {
    let v = serde_json::to_value(Request {
        id: "r1".into(),
        method: "plugin.init".into(),
        params: serde_json::json!({}),
    })
    .unwrap();
    assert_eq!(
        sorted_keys(&v),
        vec!["id", "method", "params"],
        "请求信封键集冻结"
    );
}

/// §3：响应信封——ok=true 只带 result；ok=false 只带 error（互斥由 skip 序列化保证）。
#[test]
fn response_envelope_shape_frozen() {
    let ok = serde_json::to_value(Response::ok("r1", serde_json::json!(1))).unwrap();
    assert_eq!(
        sorted_keys(&ok),
        vec!["id", "ok", "result"],
        "成功信封不带 error 键"
    );

    let err = serde_json::to_value(Response::err("r1", codes::FAILED, "x")).unwrap();
    assert_eq!(
        sorted_keys(&err),
        vec!["error", "id", "ok"],
        "失败信封不带 result 键"
    );
}

/// §8/X42：错误体 {code, message}；source_code 仅在有值时序列化（向后兼容）。
#[test]
fn plugin_error_shape_frozen() {
    let plain = serde_json::to_value(PluginError::new(codes::TIMEOUT, "超时")).unwrap();
    let keys: Vec<&str> = plain.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["code", "message"], "无业务码时不序列化 source_code");

    let sourced = serde_json::to_value(PluginError::with_source(
        codes::FAILED,
        "ekey 无效",
        "QMC-EKEY-INVALID",
    ))
    .unwrap();
    assert_eq!(sourced["source_code"], "QMC-EKEY-INVALID");
}

// ------------------------------------------------------------ 清单契约 --

/// §7：plugin.json 序列化键集冻结（9 字段）——字段增删 = 协议修订。
#[test]
fn manifest_serde_shape_frozen() {
    let m = PluginManifest {
        name: "kwm-migration".into(),
        api_version: "1.0.0".into(),
        kind: PluginKind::FormatAdapter,
        network: false,
        data_sent: vec!["input_path".into()],
        data_not_sent: vec!["audio_bytes".into()],
        ack_required: true,
        extensions: vec!["kwm".into()],
        permissions: PluginPermissions::default(),
    };
    let v = serde_json::to_value(&m).unwrap();
    assert_eq!(
        sorted_keys(&v),
        vec![
            "ack_required",
            "api_version",
            "data_not_sent",
            "data_sent",
            "extensions",
            "kind",
            "name",
            "network",
            "permissions",
        ],
        "plugin.json 键集冻结（9 字段）"
    );
}

/// §7：kind 值域冻结（6 类；alias 兼容 v0.7.0–v0.8.0 旧清单值是收敛豁免项）。
#[test]
fn plugin_kind_value_domain_frozen() {
    for (raw, expect) in [
        ("ai-provider", PluginKind::Ai),
        ("lyrics-provider", PluginKind::LyricsProvider),
        ("cover-provider", PluginKind::CoverProvider),
        ("format-adapter", PluginKind::FormatAdapter),
        ("nas-adapter", PluginKind::NasAdapter),
        ("notification", PluginKind::Notification),
    ] {
        let kind: PluginKind = serde_json::from_value(serde_json::json!(raw)).unwrap();
        assert_eq!(kind, expect, "kind 值 {raw} 反序列化漂移");
        // 序列化回写 = 原值（canonical 名，不用 alias）
        assert_eq!(serde_json::to_value(kind).unwrap(), serde_json::json!(raw));
    }
}

/// §7/§4.10：三禁位（delete/move/upload）语义冻结——声明即拒载。
#[test]
fn forbidden_permission_trio_frozen() {
    assert!(!PluginPermissions::default().has_forbidden());
    let p = PluginPermissions {
        delete_source_file: true,
        ..PluginPermissions::default()
    };
    assert!(p.has_forbidden());
    let p = PluginPermissions {
        move_source_file: true,
        ..PluginPermissions::default()
    };
    assert!(p.has_forbidden());
    let p = PluginPermissions {
        upload_audio: true,
        ..PluginPermissions::default()
    };
    assert!(p.has_forbidden());
}
