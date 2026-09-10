//! `musicforge-server` 入口：env 配置 → 路由 → 监听（R22：默认回环 + 随机 token）。

use musicforge_server::{build_router, ServerConfig, ServerState};

#[tokio::main]
async fn main() {
    let (cfg, token_generated) = match ServerConfig::from_env() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("X 配置错误: {e}");
            std::process::exit(1);
        }
    };

    // R22：token 首启生成时显式展示（一次性；此后读文件不重复打印）
    if token_generated {
        println!("=== MusicForge 首启：随机访问 token（请妥善保存）===");
        println!("{}", cfg.token);
        println!("=================================================");
    }

    let state = ServerState {
        token: cfg.token.clone(),
        ui_dir: cfg.ui_dir.clone(),
        data_dir: cfg.data_dir.clone(),
        library_dir: cfg.library_dir.clone(),
        allowed_roots: cfg.allowed_roots,
    };
    let app = build_router(state);

    let listener = match tokio::net::TcpListener::bind(&cfg.bind).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("X 绑定失败 {}: {e}", cfg.bind);
            std::process::exit(1);
        }
    };
    println!(
        "musicforge-server v{} 已启动: http://{}",
        env!("CARGO_PKG_VERSION"),
        cfg.bind
    );
    println!("  data_dir = {}", cfg.data_dir.display());
    println!("  ui_dir   = {}", cfg.ui_dir.display());

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
    println!("收到停机信号，退出。");
}
