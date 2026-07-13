use reqwest::Client;
use tracing::info;

use crate::AUDIT_TARGET;
use crate::LangfuseConfig;
use crate::ingestion_timestamp;
use std::time::Duration;
use uuid::Uuid;

pub async fn langfuse_post_batch(
    client: &Client,
    cfg: &LangfuseConfig,
    batch: Vec<serde_json::Value>,
) -> Result<(), String> {
    let url = format!(
        "{}/api/public/ingestion",
        cfg.base_url().trim_end_matches('/')
    );
    let n = batch.len();
    let body = serde_json::json!({ "batch": batch });
    for attempt in 1..=3 {
        let response = client
            .post(&url)
            .basic_auth(cfg.public_key(), Some(cfg.secret_key()))
            .json(&body)
            .send()
            .await;
        let resp = match response {
            Ok(resp) => resp,
            Err(error) if attempt < 3 => {
                tokio::time::sleep(Duration::from_millis(100 * (1 << (attempt - 1)))).await;
                tracing::warn!(target: AUDIT_TARGET, "langfuse transport error, retrying: {error}");
                continue;
            }
            Err(error) => return Err(error.to_string()),
        };

        let status = resp.status();
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_millis(100 * (1 << (attempt - 1))))
            .min(Duration::from_secs(10));
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            if attempt < 3 && (status.as_u16() == 429 || status.is_server_error()) {
                tracing::warn!(target: AUDIT_TARGET, "langfuse HTTP {status}, retrying");
                tokio::time::sleep(retry_after).await;
                continue;
            }
            return Err(format!("HTTP {status}: {text}"));
        }
        let value: serde_json::Value = serde_json::from_str(&text).map_err(|_| {
            format!(
                "langfuse ingestion: {} but body is not JSON (len {})",
                status,
                text.len()
            )
        })?;
        if let Some(errors) = value.get("errors").and_then(|e| e.as_array())
            && !errors.is_empty()
        {
            return Err(format!("langfuse ingestion partial errors: {value}"));
        }
        info!(target: AUDIT_TARGET, "langfuse ingestion ok ({n} events)");
        return Ok(());
    }
    unreachable!()
}

/// 单个 trace-create 事件
pub fn build_trace_create_event(
    trace_id: &str,
    path: &str,
    input: serde_json::Value,
    started_at: &str,
) -> serde_json::Value {
    let ts = ingestion_timestamp();
    serde_json::json!({
        "id": Uuid::new_v4().to_string(),
        "timestamp": ts,
        "type": "trace-create",
        "body": {
            "id": trace_id,
            "timestamp": started_at,
            "name": format!("llm {path}"),
            "metadata": { "path": path, "source": "llm-audit" },
            "input": input,
        }
    })
}

/// 单个 generation-create 事件
pub fn build_generation_create_event(
    gen_id: &str,
    trace_id: &str,
    model: &str,
    input: serde_json::Value,
    started_at: &str,
) -> serde_json::Value {
    let ts = ingestion_timestamp();
    serde_json::json!({
        "id": Uuid::new_v4().to_string(),
        "timestamp": ts,
        "type": "generation-create",
        "body": {
            "id": gen_id,
            "traceId": trace_id,
            "name": "llm",
            "startTime": started_at,
            "model": model,
            "input": input,
        }
    })
}

/// 单个 generation-update 事件
pub fn build_generation_update_event(
    gen_id: &str,
    output: serde_json::Value,
    completed_at: &str,
    elapsed_ms: u64,
    usage: Option<serde_json::Value>,
) -> serde_json::Value {
    let ts = ingestion_timestamp();
    serde_json::json!({
        "id": Uuid::new_v4().to_string(),
        "timestamp": ts,
        "type": "generation-update",
        "body": {
            "id": gen_id,
            "endTime": completed_at,
            "output": output,
            "usage": usage,
            "metadata": {
                "latencyMs": elapsed_ms,
            },
        }
    })
}

/// 一次性提交 trace + generation create + generation update
///
/// 同一 batch 内的事件，Langfuse worker 会按顺序处理，
/// 这样可以避免两次独立 POST 时 generation-update 抢在 generation-create 之前到达。
pub struct FullGenerationBatch<'a> {
    pub trace_id: &'a str,
    pub generation_id: &'a str,
    pub path: &'a str,
    pub input: serde_json::Value,
    pub model: &'a str,
    pub output: serde_json::Value,
    pub started_at: &'a str,
    pub completed_at: &'a str,
    pub elapsed_ms: u64,
    pub usage: Option<serde_json::Value>,
}

pub fn build_langfuse_full_batch(record: FullGenerationBatch<'_>) -> Vec<serde_json::Value> {
    vec![
        build_trace_create_event(
            record.trace_id,
            record.path,
            record.input.clone(),
            record.started_at,
        ),
        build_generation_create_event(
            record.generation_id,
            record.trace_id,
            record.model,
            record.input,
            record.started_at,
        ),
        build_generation_update_event(
            record.generation_id,
            record.output,
            record.completed_at,
            record.elapsed_ms,
            record.usage,
        ),
    ]
}

pub fn build_langfuse_start_batch(
    trace_id: &str,
    gen_id: &str,
    path: &str,
    input: serde_json::Value,
    model: &str,
    started_at: &str,
) -> Vec<serde_json::Value> {
    vec![
        build_trace_create_event(trace_id, path, input.clone(), started_at),
        build_generation_create_event(gen_id, trace_id, model, input, started_at),
    ]
}

pub fn build_generation_update_batch(
    gen_id: &str,
    output: serde_json::Value,
    completed_at: &str,
    elapsed_ms: u64,
    usage: Option<serde_json::Value>,
) -> Vec<serde_json::Value> {
    vec![build_generation_update_event(
        gen_id,
        output,
        completed_at,
        elapsed_ms,
        usage,
    )]
}
