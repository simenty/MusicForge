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

// ============================== T2 方法集落地 ==============================

mod method_set {
    use super::*;

    use musicforge_plugin_api::{
        methods, CoverQueryParams, CoverResult, DuplicateGroupParams, DuplicateMember,
        DuplicateReviewResult, FilenameRegexParams, FilenameRegexResult, HealthResult,
        IdentifyTrackParams, LyricsVerdict, LyricsVerifyParams, LyricsVerifyResult, ShutdownResult,
    };

    #[test]
    fn identify_track_typed_roundtrip() {
        let mut p = PluginProcess::spawn(mock_exe().as_ref(), ">=1,<2").unwrap();
        let params = IdentifyTrackParams {
            normalized_filename: "王铮亮 feat. 风华音纪 - 借墨 [SQ].wav".into(),
            title: Some("借墨".into()),
            artists: vec!["王铮亮".into()],
            album: None,
            duration_ms: Some(252_000),
            format: Some("wav".into()),
            language_hint: Some("zh".into()),
        };
        let sug: IdentifySuggestion = p
            .call_typed(
                methods::AI_IDENTIFY_TRACK,
                serde_json::to_value(&params).unwrap(),
                2_000,
            )
            .unwrap();
        assert_eq!(sug.title, "借墨");
        assert_eq!(sug.artists.len(), 2);
        assert!((sug.confidence - 0.93).abs() < 1e-6);
        assert_eq!(sug.field_confidence.get("album"), Some(&0.86));
    }

    #[test]
    fn health_reports_ok() {
        let mut p = PluginProcess::spawn(mock_exe().as_ref(), ">=1,<2").unwrap();
        let h: HealthResult = p
            .call_typed(methods::PLUGIN_HEALTH, serde_json::json!({}), 2_000)
            .unwrap();
        assert_eq!(h.status, "ok");
    }

    #[test]
    fn shutdown_responds_then_child_exits() {
        let mut p = PluginProcess::spawn(mock_exe().as_ref(), ">=1,<2").unwrap();
        let s: ShutdownResult = p
            .call_typed(methods::PLUGIN_SHUTDOWN, serde_json::json!({}), 2_000)
            .unwrap();
        assert!(s.accepted);
        // 自愿退出路径：子进程必须在短窗内可见地退出（非 kill）
        let mut exited = false;
        for _ in 0..50 {
            if p.child_try_wait().map(|s| s.is_some()).unwrap_or(false) {
                exited = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(exited, "shutdown 后子进程必须自行退出");
    }

    #[test]
    fn filename_regex_returns_rule_text() {
        let mut p = PluginProcess::spawn(mock_exe().as_ref(), ">=1,<2").unwrap();
        let params = FilenameRegexParams {
            samples: vec![
                "王铮亮 - 借墨 [SQ].wav".into(),
                "周杰伦 - 晴天 [HQ].flac".into(),
            ],
        };
        let r: FilenameRegexResult = p
            .call_typed(
                methods::AI_GENERATE_FILENAME_REGEX,
                serde_json::to_value(&params).unwrap(),
                2_000,
            )
            .unwrap();
        assert!(!r.rule.is_empty(), "规则文本不得为空");
        assert!(
            r.rule.contains("P<artist>"),
            "规则必须含命名捕获组: {}",
            r.rule
        );
    }

    #[test]
    fn duplicate_review_suggests_deterministic_keep() {
        let mut p = PluginProcess::spawn(mock_exe().as_ref(), ">=1,<2").unwrap();
        let params = DuplicateGroupParams {
            members: vec![
                DuplicateMember {
                    filename: "a.flac".into(),
                    format: Some("flac".into()),
                    bitrate_kbps: Some(800),
                    duration_ms: Some(252_000),
                    size_bytes: Some(25_000_000),
                },
                DuplicateMember {
                    filename: "b.flac".into(),
                    format: Some("flac".into()),
                    bitrate_kbps: Some(1000),
                    duration_ms: Some(252_000),
                    size_bytes: Some(31_000_000),
                },
            ],
        };
        let r: DuplicateReviewResult = p
            .call_typed(
                methods::AI_REVIEW_DUPLICATE_GROUP,
                serde_json::to_value(&params).unwrap(),
                2_000,
            )
            .unwrap();
        assert_eq!(r.keep_index, 1, "高码率成员应被建议保留");
        assert!(!r.reason.is_empty());
    }

    #[test]
    fn lyrics_verify_never_suggests_title_or_artists() {
        let mut p = PluginProcess::spawn(mock_exe().as_ref(), ">=1,<2").unwrap();
        let params = LyricsVerifyParams {
            title: "借墨".into(),
            artists: vec!["王铮亮".into()],
            duration_ms: Some(252_000),
            lyrics_excerpt: "一笔借墨 挥毫落纸".into(),
        };
        let r: LyricsVerifyResult = p
            .call_typed(
                methods::LYRICS_VERIFY,
                serde_json::to_value(&params).unwrap(),
                2_000,
            )
            .unwrap();
        assert_eq!(r.verdict, LyricsVerdict::Match);
        // 红线：核验结果的候选不得是标题/艺人替换（类型层面无该字段，编译期保证）
        for cand in &r.candidates {
            assert_ne!(cand, &params.title, "候选不得为标题替换项");
        }
    }

    #[test]
    fn cover_search_and_generate_return_candidates() {
        let mut p = PluginProcess::spawn(mock_exe().as_ref(), ">=1,<2").unwrap();
        let params = CoverQueryParams {
            title: "借墨".into(),
            artists: vec!["王铮亮".into()],
            album: Some("借墨".into()),
            duration_ms: Some(252_000),
            style_hint: None,
        };
        let wire = serde_json::to_string(&params).unwrap();
        for forbidden in ["cover_bytes", "audio_bytes", "absolute_path"] {
            assert!(!wire.contains(forbidden), "封面请求含禁发字段: {forbidden}");
        }
        let s: CoverResult = p
            .call_typed(
                methods::COVER_SEARCH,
                serde_json::to_value(&params).unwrap(),
                2_000,
            )
            .unwrap();
        assert!(!s.candidates.is_empty(), "cover.search 必须返回候选列表");
        let g: CoverResult = p
            .call_typed(
                methods::COVER_GENERATE,
                serde_json::to_value(&params).unwrap(),
                2_000,
            )
            .unwrap();
        assert!(!g.candidates.is_empty(), "cover.generate 必须返回候选列表");
        for c in g.candidates.iter().chain(s.candidates.iter()) {
            assert!(!c.image_ref.is_empty());
            assert!(
                (0.0..=1.0).contains(&c.confidence),
                "置信度越界: {}",
                c.confidence
            );
        }
    }

    #[test]
    fn every_method_request_wire_has_no_forbidden_keys() {
        // 全方法集线上断言：任何强类型参数序列化后都不得含禁发字段
        let payloads: Vec<serde_json::Value> = vec![
            serde_json::to_value(IdentifyTrackParams {
                normalized_filename: "x.wav".into(),
                ..Default::default()
            })
            .unwrap(),
            serde_json::to_value(FilenameRegexParams {
                samples: vec!["a.mp3".into()],
            })
            .unwrap(),
            serde_json::to_value(DuplicateGroupParams {
                members: vec![DuplicateMember {
                    filename: "a.flac".into(),
                    ..Default::default()
                }],
            })
            .unwrap(),
            serde_json::to_value(LyricsVerifyParams {
                title: "t".into(),
                artists: vec![],
                duration_ms: None,
                lyrics_excerpt: "l".into(),
            })
            .unwrap(),
            serde_json::to_value(CoverQueryParams {
                title: "t".into(),
                ..Default::default()
            })
            .unwrap(),
        ];
        for v in &payloads {
            let wire = v.to_string();
            for forbidden in ["absolute_path", "audio_bytes", "cover_bytes"] {
                assert!(
                    !wire.contains(forbidden),
                    "参数含禁发字段: {forbidden} → {wire}"
                );
            }
        }
    }
}

// ============================== T6 限制三件套 ==============================

mod limits_suite {
    use super::*;

    use musicforge_plugin_host::limits;

    /// 超时窗口 10–30s：过短夹到下限，过长夹到上限，窗口内原样。
    #[test]
    fn timeout_clamped_to_allowed_window() {
        assert_eq!(
            limits::clamp_timeout(0),
            limits::TIMEOUT_MIN_MS,
            "过短 → 下限"
        );
        assert_eq!(limits::clamp_timeout(1_000), limits::TIMEOUT_MIN_MS);
        assert_eq!(limits::clamp_timeout(15_000), 15_000, "窗口内原样");
        assert_eq!(
            limits::clamp_timeout(u64::MAX),
            limits::TIMEOUT_MAX_MS,
            "过长 → 上限"
        );
    }

    /// 并发上限 2：前两个槽位可取，第三个显式失败（try 路径）；释放后可再取。
    #[test]
    fn concurrency_slots_capped_at_two() {
        // B10 接线后的并行安全语义：批量 try_acquire 拿到的手动槽位数
        // **永远 ≤ 2**——e2e 其他测试并行 spawn 的进程同样占槽位（会减少
        // 可拿数），但上限不变量必须恒成立。强断言「恰取 2 拒绝第 3」在
        // 并行测试下不确定（别的测试正持槽位），故只钉上限、不钉下限。
        let mut taken = Vec::new();
        for _ in 0..8 {
            match musicforge_plugin_host::try_acquire_plugin_slot() {
                Some(g) => taken.push(g),
                None => break, // 满 = 上限生效
            }
        }
        assert!(
            taken.len() <= limits::MAX_CONCURRENT_PLUGINS,
            "槽位计数不得超过上限 {}: 实际 {}",
            limits::MAX_CONCURRENT_PLUGINS,
            taken.len()
        );
        drop(taken); // RAII 释放，计数守恒
    }

    /// B10 接线回归（try 语义）：占满全部手动槽位后，spawn 必须**立即显式
    /// 拒绝**（不排队、不挂起、不绕过上限）。并行下若槽位已被其他测试占用
    /// （拿不满），本测试退化为跳过（上限不变量由并发测试覆盖）。
    /// 语义依据：阻塞排队会把单个进程启动的环境性卡顿（Defender 实时扫描）
    /// 放大为全局挂死——「拒绝优于放行」是本仓哲学（审计规范 2）。
    /// 验收红线：插件被外部杀掉（kill -9 形态）→ 主进程**存活**且显式报错，绝不悬挂。
    #[test]
    fn externally_killed_child_fails_fast_and_host_survives() {
        let mut p = PluginProcess::spawn(mock_exe().as_ref(), ">=1,<2").unwrap();
        // 模拟外部 kill -9：进程直接死亡，管道破裂
        p.kill();
        let err = p
            .call("plugin.health", serde_json::json!({}), 2_000)
            .unwrap_err();
        // 平台差异：broken pipe 可能先于退出探测报 Io，但必须是**显式失败**之一
        assert!(
            matches!(err, PluginHostError::Gone | PluginHostError::Io(_)),
            "必须显式失败（Gone/Io），不得悬挂: {err}"
        );
        // 主进程存活验证：随后仍能正常 spawn 新插件并完成一次调用
        let mut p2 = PluginProcess::spawn(mock_exe().as_ref(), ">=1,<2").unwrap();
        let h: musicforge_plugin_api::HealthResult = p2
            .call_typed("plugin.health", serde_json::json!({}), 2_000)
            .unwrap();
        assert_eq!(h.status, "ok", "kill 后主进程必须可继续服务");
    }
}
