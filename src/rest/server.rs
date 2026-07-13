use crate::AppState;
use crate::get_bind_addr;
use crate::health_handler;
use crate::init_tracing;
use crate::post_proxy_handler;
use crate::tls_pem_paths;
use axum::{Router, routing::get, routing::post};
use axum_server::tls_rustls::RustlsConfig;
use std::time::Duration;

use tokio::net::TcpListener;
use tracing::info;

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    info!("shutdown signal received");
}

pub async fn start_server() {
    let _log_guard = init_tracing();

    let state = AppState::factory();
    let shutdown_state = state.clone();
    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/{*path}", post(post_proxy_handler))
        .with_state(state);

    let addr = get_bind_addr();

    if let Some(tls) = tls_pem_paths() {
        let rustls = RustlsConfig::from_pem_file(&tls.cert_pem, &tls.key_pem)
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "TLS PEM 加载失败（TLS_CERT_PATH={:?}, TLS_KEY_PATH={:?}）: {e}",
                    tls.cert_pem, tls.key_pem
                )
            });
        info!("llm-audit running at https://{}", addr);
        let handle = axum_server::Handle::new();
        let shutdown_handle = handle.clone();
        tokio::spawn(async move {
            shutdown_signal().await;
            shutdown_handle.graceful_shutdown(Some(Duration::from_secs(30)));
        });
        axum_server::bind_rustls(addr, rustls)
            .handle(handle)
            .serve(app.into_make_service())
            .await
            .expect("HTTPS server error");
    } else {
        info!("llm-audit running at http://{}", addr);
        let listener = TcpListener::bind(addr).await.unwrap();
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .unwrap();
    }
    shutdown_state
        .wait_for_background_tasks(Duration::from_secs(10))
        .await;
}
