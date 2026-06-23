use crate::AUDIT_TARGET;
use crate::AppState;
use crate::build_generation_update_batch;
use crate::build_langfuse_full_batch;
use crate::build_langfuse_start_batch;
use crate::langfuse_post_batch;
use crate::log_audit_request;
use crate::log_audit_response;
use crate::parse_input_value;
use crate::parse_llm_output;
use crate::request_is_streaming;
use axum::{
    body::Body,
    extract::{Request, State},
    http::{Response, StatusCode, header},
};
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::BodyExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::warn;
use uuid::Uuid;

const LOOP_HEADER: &str = "x-llm-audit-proxy";

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

    let body_bytes = body
        .collect()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .to_bytes();

    let url = format!("{}{}", state.llm_url(), path);
    let mut req_builder = state.http().request(parts.method.clone(), url);

    for (key, value) in parts.headers.iter() {
        if key == header::HOST
            || key == header::CONTENT_LENGTH
            || key == header::TRANSFER_ENCODING
            || key == header::CONNECTION
        {
            continue;
        }
        req_builder = req_builder.header(key, value);
    }
    req_builder = req_builder.header(LOOP_HEADER, "1");

    let started_at = chrono::Utc::now();
    let timer = std::time::Instant::now();

    let resp = req_builder
        .body(body_bytes.clone())
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    let input_val = parse_input_value(&body_bytes);
    let model = input_val
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("unknown")
        .to_string();

    let trace_id = Uuid::new_v4().to_string();
    let gen_id = Uuid::new_v4().to_string();

    let status = resp.status();
    let upstream_status = status.as_u16();
    let streaming = request_is_streaming(&body_bytes);

    let audit_max = state.audit_log_max_chars();
    if state.audit_log_enabled() {
        log_audit_request(&trace_id, &path, &model, &input_val, audit_max);
    }

    if streaming {
        // 流式：先发 trace-create + generation-create（运行期间用户能在 Langfuse 看到 trace），
        // 通过 oneshot 等待其完成后再发 generation-update，避免 update 抢跑在 create 之前。
        // true = Langfuse trace/generation create 已成功，才允许发 generation-update
        let (start_done_tx, start_done_rx) = tokio::sync::oneshot::channel::<bool>();

        if !state.audit_log_enabled() && state.langfuse().is_none() {
            log_audit_request(&trace_id, &path, &model, &input_val, audit_max);
        }

        if let Some(cfg) = state.langfuse() {
            let started_ts = started_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            let batch = build_langfuse_start_batch(
                &trace_id,
                &gen_id,
                &path,
                input_val.clone(),
                &model,
                &started_ts,
            );
            let http = state.http().clone();
            let cfg = cfg.clone();
            let tid = trace_id.clone();
            let path_c = path.clone();
            let model_c = model.clone();
            let input_c = input_val.clone();
            let audit_enabled = state.audit_log_enabled();
            tokio::spawn(async move {
                let start_ok = match langfuse_post_batch(&http, &cfg, batch).await {
                    Ok(()) => true,
                    Err(e) => {
                        warn!(target: AUDIT_TARGET, trace_id = %tid, "langfuse trace/generation create: {e}");
                        if !audit_enabled {
                            log_audit_request(&tid, &path_c, &model_c, &input_c, audit_max);
                        }
                        false
                    }
                };
                let _ = start_done_tx.send(start_ok);
            });
        } else {
            let _ = start_done_tx.send(false);
        }

        let lf = state.langfuse().clone();
        let http = state.http().clone();
        let gid = gen_id.clone();
        let tid_stream = trace_id.clone();
        let audit_enabled = state.audit_log_enabled();
        let timer = timer;
        let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(32);
        tokio::spawn(async move {
            let mut s = resp.bytes_stream();
            let mut buf: Vec<u8> = Vec::new();
            while let Some(item) = s.next().await {
                match item {
                    Ok(chunk) => {
                        buf.extend_from_slice(&chunk);
                        if tx.send(Ok(chunk)).await.is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(std::io::Error::other(e.to_string()))).await;
                        return;
                    }
                }
            }
            drop(tx);

            let elapsed = timer.elapsed();
            let elapsed_ms = elapsed.as_millis() as u64;
            let completed_at = chrono::Utc::now();

            let out = parse_llm_output(&buf);
            if audit_enabled {
                log_audit_response(&tid_stream, upstream_status, &out, audit_max, elapsed_ms);
            }
            match lf {
                Some(cfg) => {
                    // 等待 start batch 完成；仅 create 成功后再发 update（失败则跳过，避免对不存在 generation 打点）
                    let start_ok = start_done_rx.await.unwrap_or(false);
                    if !start_ok {
                        if !audit_enabled {
                            log_audit_response(&tid_stream, upstream_status, &out, audit_max, elapsed_ms);
                        }
                        return;
                    }
                    let completed_ts = completed_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                    let batch = build_generation_update_batch(&gid, out.clone(), &completed_ts, elapsed_ms);
                    if let Err(e) = langfuse_post_batch(&http, &cfg, batch).await {
                        warn!(target: AUDIT_TARGET, trace_id = %tid_stream, "langfuse generation-update: {e}");
                        if !audit_enabled {
                            log_audit_response(&tid_stream, upstream_status, &out, audit_max, elapsed_ms);
                        }
                    }
                }
                None => {
                    if !audit_enabled {
                        log_audit_response(&tid_stream, upstream_status, &out, audit_max, elapsed_ms);
                    }
                }
            }
        });
        let body = Body::from_stream(ReceiverStream::new(rx));
        let mut response = Response::new(body);
        *response.status_mut() = status;
        return Ok(response);
    }

    // 非流式：等上游响应到齐后，把 trace-create / generation-create / generation-update
    // 一次性塞进同一个 batch 提交，Langfuse 按 batch 内顺序处理，杜绝事件错序。
    let full = resp.bytes().await.map_err(|_| StatusCode::BAD_GATEWAY)?;

    let elapsed = timer.elapsed();
    let elapsed_ms = elapsed.as_millis() as u64;
    let completed_at = chrono::Utc::now();

    let out = parse_llm_output(&full);

    if state.audit_log_enabled() {
        log_audit_response(&trace_id, upstream_status, &out, audit_max, elapsed_ms);
    }

    match state.langfuse() {
        Some(cfg) => {
            let started_ts = started_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            let completed_ts = completed_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            let batch = build_langfuse_full_batch(
                &trace_id,
                &gen_id,
                &path,
                input_val.clone(),
                &model,
                out.clone(),
                &started_ts,
                &completed_ts,
                elapsed_ms,
            );
            let http = state.http().clone();
            let cfg = cfg.clone();
            let tid = trace_id.clone();
            let path_c = path.clone();
            let model_c = model.clone();
            let input_c = input_val.clone();
            let out_c = out.clone();
            let audit_enabled = state.audit_log_enabled();
            tokio::spawn(async move {
                if let Err(e) = langfuse_post_batch(&http, &cfg, batch).await {
                    warn!(target: AUDIT_TARGET, trace_id = %tid, "langfuse ingestion: {e}");
                    if !audit_enabled {
                        log_audit_request(&tid, &path_c, &model_c, &input_c, audit_max);
                        log_audit_response(&tid, upstream_status, &out_c, audit_max, elapsed_ms);
                    }
                }
            });
        }
        None => {
            if !state.audit_log_enabled() {
                log_audit_request(&trace_id, &path, &model, &input_val, audit_max);
                log_audit_response(&trace_id, upstream_status, &out, audit_max, elapsed_ms);
            }
        }
    }

    let mut response = Response::new(Body::from(full));
    *response.status_mut() = status;
    Ok(response)
}
