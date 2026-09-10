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
    };
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
