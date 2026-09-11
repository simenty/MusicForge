//! `musicforge-server` 入口：env 配置 → 路由 → 监听（R22：默认回环 + 随机 token）。

use musicforge_server::{build_router, init_logging, ServerConfig, ServerState};

#[tokio::main]
async fn main() {
    // P1-2：最先初始化日志（stdout → fpk cmd/main 已重定向到 data/logs/server.log）
    init_logging();

    let (cfg, token_generated) = match ServerConfig::from_env() {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("配置错误: {e}");
            std::process::exit(1);
        }
    };

    // R22：token 首启生成时显式展示（一次性；此后读文件不重复打印）
    if token_generated {
        println!("=== MusicForge 首启：随机访问 token（请妥善保存）===");
        println!("{}", cfg.token);
        println!("=================================================");
    }

    // P1-2：allowed_roots 稍后 move 进 state——先留计数供启动日志
    let allowed_roots_n = cfg.allowed_roots.len();

    let state = ServerState {
        token: cfg.token.clone(),
        auth_guard: std::sync::Arc::new(musicforge_server::AuthGuard::new()),
        ui_dir: cfg.ui_dir.clone(),
        data_dir: cfg.data_dir.clone(),
        library_dir: cfg.library_dir.clone(),
        allowed_roots: cfg.allowed_roots,
        auth_require_sign: cfg.auth_require_sign,
        auth_disabled: cfg.auth_disabled,
        nonce_seen: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
    };
    if cfg.auth_disabled {
        // 显式可见（降级绝不静默）：这是产品决策后的默认形态（fnOS 家庭内网）
        tracing::warn!(
            "MUSICFORGE_AUTH=off：鉴权已关闭——同一局域网内任何设备都可访问本服务（含破坏性操作；\
             产物进回收站可整体还原）。如需鉴权：移除该环境变量或设为 on 后重启"
        );
    } else if !cfg.auth_require_sign {
        tracing::warn!(
            "MUSICFORGE_AUTH_LEGACY=1：请求签名校验关闭（仅静态 token）——仅用于排查，勿长期启用"
        );
    }
    let app = build_router(state);

    let listener = match tokio::net::TcpListener::bind(&cfg.bind).await {
        Ok(l) => l,
        Err(e) => {
            // 端口被占用是 fnOS 升级启动失败的常见因（B27）——给出可操作提示
            tracing::error!("绑定失败 {}: {e}", cfg.bind);
            tracing::error!("若端口被占用：确认是否有残留 musicforge-server 进程");
            std::process::exit(1);
        }
    };
    // 启动行：形态信息全量可见（**不打印 token 值**）
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        bind = %cfg.bind,
        data_dir = %cfg.data_dir.display(),
        ui_dir = %cfg.ui_dir.display(),
        library_dir = ?cfg.library_dir.as_ref().map(|p| p.display().to_string()),
        allowed_roots = allowed_roots_n,
        pid = std::process::id(),
        "musicforge-server started"
    );
    if allowed_roots_n == 0 {
        tracing::warn!("MUSICFORGE_ALLOWED_ROOTS 未配置：路径域不约束（依赖 token 闸）");
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server 运行错误");
}

/// 优雅停机：SIGTERM（fpk stop → kill）/ Ctrl+C。
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM 监听失败")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("收到停机信号，优雅退出。");
}
