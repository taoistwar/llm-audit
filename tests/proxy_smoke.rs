//! 经 llm-audit 代理对 HTTP / HTTPS 做冒烟请求（POST `/v1/chat/completions`，非流式）。
//!
//! ## 环境文件（相对仓库根目录，即 `CARGO_MANIFEST_DIR`）
//!
//! 1. 先加载 `.env`（已存在于环境中的变量不会被覆盖）。
//! 2. 再加载 `.env-dev`（**覆盖**已有变量，便于本地覆盖默认）。
//!
//! ## 运行
//!
//! 需要代理与上游已启动。默认 **`#[ignore]`**，避免无代理时 `cargo test` 失败：
//!
//! ```text
//! cargo test --test proxy_smoke -- --ignored --nocapture
//! ```
//!
//! 常用变量（可写在 `.env` / `.env-dev`）：`HTTP_PROXY_URL`、`HTTPS_PROXY_URL`、`TEST_MODEL`、
//! `OPENAI_API_KEY`（非空则带 `Authorization: Bearer`）。HTTPS 自签时默认不校验证书；设
//! `PROXY_SMOKE_VERIFY_TLS=1` 则启用校验。

use reqwest::Client;
use serde_json::json;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

const DEFAULT_HTTP: &str = "http://127.0.0.1:19001";
const DEFAULT_HTTPS: &str = "https://127.0.0.1:19001";

static LOAD_ENV: OnceLock<()> = OnceLock::new();

fn load_integration_env() {
    LOAD_ENV.get_or_init(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let _ = dotenvy::from_path(root.join(".env")).ok();
        let _ = dotenvy::from_path_override(root.join(".env-dev")).ok();
    });
}

fn env_trim(name: &str, default: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| default.to_string())
        .trim_end_matches('/')
        .to_string()
}

fn verify_tls_enabled() -> bool {
    matches!(
        std::env::var("PROXY_SMOKE_VERIFY_TLS")
            .map(|v| matches!(v.trim(), "1" | "true" | "yes" | "on")),
        Ok(true)
    )
}

fn client(strict_tls: bool) -> Result<Client, reqwest::Error> {
    let mut b = Client::builder().timeout(Duration::from_secs(120));
    if !strict_tls {
        b = b.danger_accept_invalid_certs(true);
    }
    b.build()
}

async fn post_chat_completions(
    client: &Client,
    base_url: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));
    let model = std::env::var("TEST_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());
    let body = json!({
        "model": model,
        "messages": [{"role": "user", "content": "Reply with exactly: OK"}],
        "stream": false,
    });

    let mut req = client.post(&url).json(&body);
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        let key = key.trim();
        if !key.is_empty() {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
    }

    let resp = req.send().await?;
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        return Err(format!("HTTP {status}: {text}").into());
    }

    let v: serde_json::Value = serde_json::from_str(&text)?;
    let done = v.get("done").and_then(|x| x.as_bool()).unwrap_or(true);
    if !done {
        return Err(format!("未收到完整响应: {v}").into());
    }
    if v.get("choices").is_none() && v.get("response").is_none() {
        return Err(format!("响应缺少 choices/response 字段: {v}").into());
    }
    Ok(v)
}

async fn run_one(
    label: &str,
    base_url: &str,
    strict_tls: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let c = client(strict_tls)?;
    eprintln!("--- {label}: POST {base_url}/v1/chat/completions ---");
    let obj = post_chat_completions(&c, base_url).await?;
    let preview = obj
        .pointer("/choices/0/message/content")
        .and_then(|x| x.as_str())
        .or_else(|| obj.get("response").and_then(|x| x.as_str()))
        .unwrap_or("")
        .chars()
        .take(200)
        .collect::<String>();
    eprintln!("    response 预览: {preview:?}");
    eprintln!("    OK ({label})\n");
    Ok(())
}

#[tokio::test]
#[ignore = "需要已启动的代理与上游；先配置 .env / .env-dev，再: cargo test --test proxy_smoke -- --ignored --nocapture"]
async fn proxy_smoke_http() {
    load_integration_env();
    let http_u = env_trim("HTTP_PROXY_URL", DEFAULT_HTTP);
    run_one("HTTP", &http_u, true).await.unwrap();
}

#[tokio::test]
#[ignore = "需要已启动的代理与上游；先配置 .env / .env-dev，再: cargo test --test proxy_smoke -- --ignored --nocapture"]
async fn proxy_smoke_https() {
    load_integration_env();
    let https_u = env_trim("HTTPS_PROXY_URL", DEFAULT_HTTPS);
    let strict = verify_tls_enabled();
    run_one("HTTPS", &https_u, strict).await.unwrap();
}
