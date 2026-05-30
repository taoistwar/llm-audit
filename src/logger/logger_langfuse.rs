use reqwest::Client;
use tracing::info;

use crate::AUDIT_TARGET;
use crate::LangfuseConfig;
use crate::ingestion_timestamp;
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
    let resp = client
        .post(url)
        .basic_auth(cfg.public_key(), Some(cfg.secret_key()))
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("HTTP {status}: {text}"));
    }
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|_| {
        format!(
            "langfuse ingestion: {} but body is not JSON (len {})",
            status,
            text.len()
        )
    })?;
    if let Some(errs) = v.get("errors").and_then(|e| e.as_array())
        && !errs.is_empty()
    {
        return Err(format!("langfuse ingestion partial errors: {v}"));
    }
    info!(target: AUDIT_TARGET, "langfuse ingestion ok ({n} events)");
    Ok(())
}

/// 单个 trace-create 事件
pub fn build_trace_create_event(
    trace_id: &str,
    path: &str,
    input: serde_json::Value,
) -> serde_json::Value {
    let ts = ingestion_timestamp();
    serde_json::json!({
        "id": Uuid::new_v4().to_string(),
        "timestamp": ts,
        "type": "trace-create",
        "body": {
            "id": trace_id,
            "timestamp": ts,
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
            "startTime": ts,
            "model": model,
            "input": input,
        }
    })
}

/// 单个 generation-update 事件
pub fn build_generation_update_event(
    gen_id: &str,
    output: serde_json::Value,
) -> serde_json::Value {
    let ts = ingestion_timestamp();
    serde_json::json!({
        "id": Uuid::new_v4().to_string(),
        "timestamp": ts,
        "type": "generation-update",
        "body": {
            "id": gen_id,
            "endTime": ts,
            "output": output,
        }
    })
}

/// 一次性提交 trace + generation create + generation update
///
/// 同一 batch 内的事件，Langfuse worker 会按顺序处理，
/// 这样可以避免两次独立 POST 时 generation-update 抢在 generation-create 之前到达。
pub fn build_langfuse_full_batch(
    trace_id: &str,
    gen_id: &str,
    path: &str,
    input: serde_json::Value,
    model: &str,
    output: serde_json::Value,
) -> Vec<serde_json::Value> {
    vec![
        build_trace_create_event(trace_id, path, input.clone()),
        build_generation_create_event(gen_id, trace_id, model, input),
        build_generation_update_event(gen_id, output),
    ]
}

pub fn build_langfuse_start_batch(
    trace_id: &str,
    gen_id: &str,
    path: &str,
    input: serde_json::Value,
    model: &str,
) -> Vec<serde_json::Value> {
    vec![
        build_trace_create_event(trace_id, path, input.clone()),
        build_generation_create_event(gen_id, trace_id, model, input),
    ]
}

pub fn build_generation_update_batch(
    gen_id: &str,
    output: serde_json::Value,
) -> Vec<serde_json::Value> {
    vec![build_generation_update_event(gen_id, output)]
}

/// 单个 tool-create 事件
pub fn build_tool_create_event(
    tool_id: &str,
    trace_id: &str,
    tool_name: &str,
    input: serde_json::Value,
    parent_observation_id: Option<&str>,
) -> serde_json::Value {
    let ts = ingestion_timestamp();
    serde_json::json!({
        "id": Uuid::new_v4().to_string(),
        "timestamp": ts,
        "type": "tool-create",
        "body": {
            "id": tool_id,
            "traceId": trace_id,
            "name": tool_name,
            "input": input,
            "startTime": ts,
            "parentObservationId": parent_observation_id,
        }
    })
}

/// 单个 tool-update 事件
pub fn build_tool_update_event(
    tool_id: &str,
    trace_id: &str,
    output: serde_json::Value,
    is_error: bool,
) -> serde_json::Value {
    let ts = ingestion_timestamp();
    serde_json::json!({
        "id": Uuid::new_v4().to_string(),
        "timestamp": ts,
        "type": "tool-update",
        "body": {
            "id": tool_id,
            "traceId": trace_id,
            "endTime": ts,
            "output": output,
            "level": if is_error { "ERROR" } else { "DEFAULT" },
        }
    })
}

/// 构建包含 tool 调用事件的 batch
///
/// 用于在 generation 输出中包含 tool_calls 时，
/// 将 tool observation 追加到 batch 中。
pub fn build_tool_events_batch(
    trace_id: &str,
    tool_id: &str,
    tool_name: &str,
    input: serde_json::Value,
    output: serde_json::Value,
    is_error: bool,
    parent_observation_id: Option<&str>,
) -> Vec<serde_json::Value> {
    vec![
        build_tool_create_event(tool_id, trace_id, tool_name, input, parent_observation_id),
        build_tool_update_event(tool_id, trace_id, output, is_error),
    ]
}
