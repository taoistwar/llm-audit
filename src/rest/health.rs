use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// 就绪探测：不访问上游 LLM，仅表示本代理进程在监听。
pub async fn health_handler() -> Response {
    (
        StatusCode::OK,
        Json(json!({
            "status": "ok",
            "service": "llm-audit",
        })),
    )
        .into_response()
}
