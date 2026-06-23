pub mod env_utils;
pub use env_utils::{audit_log_max_chars_from_env, env_flag_explicit_false, env_flag_true};

pub fn ingestion_timestamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn ingestion_timestamp_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn json_for_log(v: &serde_json::Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "<non-serializable>".to_string())
}

pub fn parse_input_value(body_bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(body_bytes).unwrap_or_else(|_| {
        serde_json::Value::String(String::from_utf8_lossy(body_bytes).into_owned())
    })
}
