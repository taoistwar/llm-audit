use crate::{
    AUDIT_TARGET, AppState, FullGenerationBatch, build_generation_update_batch,
    build_langfuse_full_batch, build_langfuse_start_batch, langfuse_post_batch, log_audit_request,
    log_audit_response, parse_input_value, parse_llm_output, parse_llm_usage, request_is_streaming,
};
use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, HeaderName, Response, StatusCode, header},
};
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::{BodyExt, Limited};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::warn;
use uuid::Uuid;

const LOOP_HEADER: &str = "x-llm-audit-proxy";

fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "proxy-connection"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn copy_upstream_headers(response: &mut Response<Body>, headers: &HeaderMap) {
    for (name, value) in headers {
        if !is_hop_by_hop(name) {
            response.headers_mut().append(name, value.clone());
        }
    }
}

fn capture_chunk(buf: &mut Vec<u8>, chunk: &[u8], max: usize, truncated: &mut bool) {
    let remaining = max.saturating_sub(buf.len());
    let take = remaining.min(chunk.len());
    buf.extend_from_slice(&chunk[..take]);
    *truncated |= take < chunk.len();
}

fn captured_output(buf: &[u8], truncated: bool, error: Option<&str>) -> serde_json::Value {
    let output = parse_llm_output(buf);
    if !truncated && error.is_none() {
        return output;
    }
    serde_json::json!({
        "output": output,
        "captureTruncated": truncated,
        "proxyError": error,
    })
}

pub async fn post_proxy_handler(
    State(state): State<AppState>,
    req: Request<Body>,
) -> Result<Response<Body>, StatusCode> {
    let (parts, body) = req.into_parts();

    if parts.headers.contains_key(LOOP_HEADER) {
        warn!("loop detected – request already passed through this proxy, dropping");
        return Err(StatusCode::LOOP_DETECTED);
    }

    let path = parts
        .uri
        .path_and_query()
        .map(|x| x.as_str())
        .unwrap_or("")
        .to_string();

    let body_bytes = Limited::new(body, state.request_body_max_bytes())
        .collect()
        .await
        .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?
        .to_bytes();
    let input_val = parse_input_value(&body_bytes);
    let streaming = request_is_streaming(&body_bytes);
    let model = input_val
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("unknown")
        .to_string();
    let trace_id = Uuid::new_v4().to_string();
    let gen_id = Uuid::new_v4().to_string();
    let audit_max = state.audit_log_max_chars();

    if state.audit_log_enabled() {
        log_audit_request(&trace_id, &path, &model, &input_val, audit_max);
    }

    let url = format!("{}{}", state.llm_url(), path);
    let mut req_builder = state.http().request(parts.method, url);
    for (key, value) in &parts.headers {
        if key != header::HOST && key != header::CONTENT_LENGTH && !is_hop_by_hop(key) {
            req_builder = req_builder.header(key, value);
        }
    }
    req_builder = req_builder.header(LOOP_HEADER, "1");

    let started_at = chrono::Utc::now();
    let timer = std::time::Instant::now();
    let resp = match req_builder.body(body_bytes).send().await {
        Ok(resp) => resp,
        Err(error) => {
            let elapsed_ms = timer.elapsed().as_millis() as u64;
            let output = serde_json::json!({ "proxyError": error.to_string() });
            if state.audit_log_enabled() {
                log_audit_response(&trace_id, 502, &output, audit_max, elapsed_ms);
            }
            if let Some(cfg) = state.langfuse().clone() {
                let started_ts = started_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                let completed_ts =
                    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                let batch = build_langfuse_full_batch(FullGenerationBatch {
                    trace_id: &trace_id,
                    generation_id: &gen_id,
                    path: &path,
                    input: input_val,
                    model: &model,
                    output,
                    started_at: &started_ts,
                    completed_at: &completed_ts,
                    elapsed_ms,
                    usage: None,
                });
                let http = state.http().clone();
                state.spawn_background(async move {
                    if let Err(error) = langfuse_post_batch(&http, &cfg, batch).await {
                        warn!(target: AUDIT_TARGET, "langfuse ingestion: {error}");
                    }
                });
            }
            return Err(StatusCode::BAD_GATEWAY);
        }
    };

    let status = resp.status();
    let upstream_status = status.as_u16();
    let response_headers = resp.headers().clone();
    let capture_max = state.response_capture_max_bytes();
    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(32);

    if streaming {
        let (start_done_tx, start_done_rx) = tokio::sync::oneshot::channel::<bool>();
        if let Some(cfg) = state.langfuse().clone() {
            let batch = build_langfuse_start_batch(
                &trace_id,
                &gen_id,
                &path,
                input_val.clone(),
                &model,
                &started_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            );
            let http = state.http().clone();
            state.spawn_background(async move {
                let ok = match langfuse_post_batch(&http, &cfg, batch).await {
                    Ok(()) => true,
                    Err(error) => {
                        warn!(target: AUDIT_TARGET, "langfuse trace/generation create: {error}");
                        false
                    }
                };
                let _ = start_done_tx.send(ok);
            });
        } else {
            let _ = start_done_tx.send(false);
        }

        let lf = state.langfuse().clone();
        let http = state.http().clone();
        let audit_enabled = state.audit_log_enabled();
        state.spawn_background(async move {
            let mut stream = resp.bytes_stream();
            let mut captured = Vec::new();
            let mut truncated = false;
            let mut stream_error = None;
            while let Some(item) = stream.next().await {
                match item {
                    Ok(chunk) => {
                        capture_chunk(&mut captured, &chunk, capture_max, &mut truncated);
                        if tx.send(Ok(chunk)).await.is_err() {
                            stream_error = Some("client disconnected".to_string());
                            break;
                        }
                    }
                    Err(error) => {
                        stream_error = Some(error.to_string());
                        let _ = tx.send(Err(std::io::Error::other(error.to_string()))).await;
                        break;
                    }
                }
            }
            drop(tx);
            let elapsed_ms = timer.elapsed().as_millis() as u64;
            let output = captured_output(&captured, truncated, stream_error.as_deref());
            let usage = parse_llm_usage(&captured);
            if audit_enabled {
                log_audit_response(&trace_id, upstream_status, &output, audit_max, elapsed_ms);
            }
            if let Some(cfg) = lf
                && start_done_rx.await.unwrap_or(false)
            {
                let batch = build_generation_update_batch(
                    &gen_id,
                    output,
                    &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    elapsed_ms,
                    usage,
                );
                if let Err(error) = langfuse_post_batch(&http, &cfg, batch).await {
                    warn!(target: AUDIT_TARGET, "langfuse generation-update: {error}");
                }
            }
        });
    } else {
        let lf = state.langfuse().clone();
        let http = state.http().clone();
        let audit_enabled = state.audit_log_enabled();
        state.spawn_background(async move {
            let mut stream = resp.bytes_stream();
            let mut captured = Vec::new();
            let mut truncated = false;
            let mut stream_error = None;
            while let Some(item) = stream.next().await {
                match item {
                    Ok(chunk) => {
                        capture_chunk(&mut captured, &chunk, capture_max, &mut truncated);
                        if tx.send(Ok(chunk)).await.is_err() {
                            stream_error = Some("client disconnected".to_string());
                            break;
                        }
                    }
                    Err(error) => {
                        stream_error = Some(error.to_string());
                        let _ = tx.send(Err(std::io::Error::other(error.to_string()))).await;
                        break;
                    }
                }
            }
            drop(tx);
            let elapsed_ms = timer.elapsed().as_millis() as u64;
            let output = captured_output(&captured, truncated, stream_error.as_deref());
            let usage = parse_llm_usage(&captured);
            if audit_enabled {
                log_audit_response(&trace_id, upstream_status, &output, audit_max, elapsed_ms);
            }
            if let Some(cfg) = lf {
                let started_ts = started_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                let completed_ts =
                    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                let batch = build_langfuse_full_batch(FullGenerationBatch {
                    trace_id: &trace_id,
                    generation_id: &gen_id,
                    path: &path,
                    input: input_val,
                    model: &model,
                    output,
                    started_at: &started_ts,
                    completed_at: &completed_ts,
                    elapsed_ms,
                    usage,
                });
                if let Err(error) = langfuse_post_batch(&http, &cfg, batch).await {
                    warn!(target: AUDIT_TARGET, "langfuse ingestion: {error}");
                }
            }
        });
    }

    let mut response = Response::new(Body::from_stream(ReceiverStream::new(rx)));
    *response.status_mut() = status;
    copy_upstream_headers(&mut response, &response_headers);
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::post_proxy_handler;
    use crate::AppState;
    use axum::{
        Router,
        body::Body,
        extract::State,
        http::{Request, StatusCode, header},
        routing::post,
    };
    use http_body_util::BodyExt;

    async fn test_state() -> AppState {
        let upstream = Router::new().route(
            "/v1/test",
            post(|| async {
                (
                    [
                        (header::CONTENT_TYPE, "application/json"),
                        (header::HeaderName::from_static("x-request-id"), "req-123"),
                    ],
                    r#"{"choices":[{"message":{"content":"ok"}}]}"#,
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, upstream).await.unwrap();
        });
        AppState::new(
            format!("http://{addr}"),
            reqwest::Client::new(),
            None,
            false,
            16_384,
        )
    }

    #[tokio::test]
    async fn preserves_upstream_headers_and_body() {
        let request = Request::post("/v1/test")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"model":"test","stream":false}"#))
            .unwrap();
        let response = post_proxy_handler(State(test_state().await), request)
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        assert_eq!(response.headers()["x-request-id"], "req-123");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(String::from_utf8_lossy(&body).contains("content"));
    }

    #[tokio::test]
    async fn rejects_oversized_request_body() {
        let mut state = test_state().await;
        state.set_request_body_max_bytes(4);
        let request = Request::post("/v1/test").body(Body::from("12345")).unwrap();
        let result = post_proxy_handler(State(state), request).await;
        assert_eq!(result.unwrap_err(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}
