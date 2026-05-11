use crate::AppState;
use crate::get_bind_addr;
use crate::get_proxy_handler;
use crate::init_tracing;
use crate::health_handler;
use crate::post_proxy_handler;
use crate::tls_pem_paths;
use axum::{Router, routing::get};
use axum_server::tls_rustls::RustlsConfig;

use tokio::net::TcpListener;
use tracing::info;

pub async fn start_server() {
    dotenvy::dotenv().ok();

    let _log_guard = init_tracing();

    let state = AppState::factory();
    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/{*path}", get(get_proxy_handler).post(post_proxy_handler))
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
        axum_server::bind_rustls(addr, rustls)
            .serve(app.into_make_service())
            .await
            .expect("HTTPS server error");
    } else {
        info!("llm-audit running at http://{}", addr);
        let listener = TcpListener::bind(addr).await.unwrap();
        axum::serve(listener, app).await.unwrap();
    }
}
