pub fn env_flag_true(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// 变量已设置且为常见「关闭」取值时为 true；未设置时不视为关闭（便于保留默认行为）。
/// 审计日志里 `input` / `output` JSON 字符串的最大 UTF-8 字节数。`0` 表示不截断。
pub fn audit_log_max_chars_from_env() -> usize {
    match std::env::var("AUDIT_LOG_MAX_CHARS") {
        Ok(s) => {
            let t = s.trim();
            if t.is_empty() {
                return 16_384;
            }
            match t.parse::<usize>() {
                Ok(0) => usize::MAX,
                Ok(n) => n,
                Err(_) => 16_384,
            }
        }
        Err(_) => 16_384,
    }
}

pub fn env_flag_explicit_false(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
        .unwrap_or(false)
}

pub fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(default)
}
