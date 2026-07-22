fn content_from_json(value: &serde_json::Value) -> Option<&str> {
    value
        .pointer("/choices/0/delta/content")
        .and_then(|v| v.as_str())
        .or_else(|| {
            value
                .pointer("/choices/0/message/content")
                .and_then(|v| v.as_str())
        })
        .or_else(|| value.pointer("/choices/0/text").and_then(|v| v.as_str()))
        .or_else(|| value.get("output_text").and_then(|v| v.as_str()))
        .or_else(|| value.get("delta").and_then(|v| v.as_str()))
        .or_else(|| {
            value
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|v| v.as_str())
        })
        .or_else(|| value.get("response").and_then(|v| v.as_str()))
}

fn usage_from_json(value: &serde_json::Value) -> Option<serde_json::Value> {
    let usage = value.get("usage").unwrap_or(value);
    let prompt = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .or_else(|| value.get("prompt_eval_count"))
        .and_then(|v| v.as_u64());
    let completion = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .or_else(|| value.get("eval_count"))
        .and_then(|v| v.as_u64());
    let total = usage
        .get("total_tokens")
        .and_then(|v| v.as_u64())
        .or_else(|| prompt.zip(completion).map(|(a, b)| a + b));
    if prompt.is_none() && completion.is_none() && total.is_none() {
        return None;
    }
    Some(serde_json::json!({
        "promptTokens": prompt,
        "completionTokens": completion,
        "totalTokens": total,
    }))
}

pub fn parse_llm_usage(raw: &[u8]) -> Option<serde_json::Value> {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(raw) {
        return usage_from_json(&value);
    }
    let mut result = None;
    for raw_line in raw.split(|byte| *byte == b'\n') {
        let line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
        let line = line
            .strip_prefix(b"data:")
            .map(|data| data.strip_prefix(b" ").unwrap_or(data))
            .unwrap_or(line);
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(line)
            && let Some(usage) = usage_from_json(&value)
        {
            result = Some(usage);
        }
    }
    result
}

pub fn parse_llm_output(raw: &[u8]) -> serde_json::Value {
    if raw.is_empty() {
        return serde_json::Value::Null;
    }

    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(raw) {
        // Tool-call completions commonly use an empty content string; in that case the
        // surrounding response is the output and must be kept so tool_calls are not lost.
        return content_from_json(&value)
            .filter(|content| !content.is_empty())
            .map(|content| serde_json::Value::String(content.to_string()))
            .unwrap_or(value);
    }

    let mut text = String::new();
    for raw_line in raw.split(|byte| *byte == b'\n') {
        let line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
        let line = line
            .strip_prefix(b"data:")
            .map(|data| data.strip_prefix(b" ").unwrap_or(data))
            .unwrap_or(line);
        if line.is_empty() || line == b"[DONE]" {
            continue;
        }
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(line)
            && let Some(content) = content_from_json(&value)
        {
            text.push_str(content);
        }
    }

    if text.is_empty() {
        serde_json::Value::String(String::from_utf8_lossy(raw).into_owned())
    } else {
        serde_json::Value::String(text)
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_llm_output, parse_llm_usage};

    #[test]
    fn parses_openai_chat_json() {
        let raw = br#"{"choices":[{"message":{"content":"hello"}}]}"#;
        assert_eq!(parse_llm_output(raw), "hello");
    }

    #[test]
    fn preserves_openai_tool_call_response_when_content_is_empty() {
        let raw = br#"{
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "content": "",
                    "tool_calls": [{
                        "id": "call_123",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\":\"Cargo.toml\"}"
                        }
                    }]
                }
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        }"#;

        let output = parse_llm_output(raw);

        assert_eq!(
            output.pointer("/choices/0/message/tool_calls/0/function/name"),
            Some(&serde_json::Value::String("read_file".to_string()))
        );
    }

    #[test]
    fn parses_openai_sse() {
        let raw = b"data: {\"choices\":[{\"delta\":{\"content\":\"hel\"}}]}\n\n\
                    data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n\
                    data: [DONE]\n\n";
        assert_eq!(parse_llm_output(raw), "hello");
    }

    #[test]
    fn parses_ollama_ndjson() {
        let raw = b"{\"message\":{\"content\":\"a\"}}\n{\"response\":\"b\"}\n";
        assert_eq!(parse_llm_output(raw), "ab");
    }

    #[test]
    fn parses_openai_usage() {
        let raw = br#"{"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;
        let usage = parse_llm_usage(raw).unwrap();
        assert_eq!(usage["promptTokens"], 3);
        assert_eq!(usage["completionTokens"], 2);
        assert_eq!(usage["totalTokens"], 5);
    }
}
