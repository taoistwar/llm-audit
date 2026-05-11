//! 经 llm-audit 代理对 HTTP / HTTPS 做冒烟请求（POST `/v1/chat/completions`，非流式）。
//!
//! 需要代理与上游已启动。上游若为 OpenAI 兼容且需鉴权，请设置 `OPENAI_API_KEY`（会作为
//! `Authorization: Bearer …` 发出，由代理原样转发）。HTTPS 自签证书时默认**不校验**证书（仅本地测试）。
//!
//! ```text
//! cargo run --bin proxy_smoke
//! cargo run --bin proxy_smoke -- --http-only
//! cargo run --bin proxy_smoke -- --https-only
//! cargo run --bin proxy_smoke -- --verify-tls
//!
//! OPENAI_API_KEY=sk-... HTTP_PROXY_URL=... HTTPS_PROXY_URL=... TEST_MODEL=gpt-4o-mini cargo run --bin proxy_smoke
//! ```

use reqwest::Client;
use serde_json::json;
use std::time::Duration;

const DEFAULT_HTTP: &str = "http://127.0.0.1:19001";
const DEFAULT_HTTPS: &str = "https://127.0.0.1:19001";

struct Args {
    http_only: bool,
    https_only: bool,
    verify_tls: bool,
    help: bool,
}

fn parse_args() -> Args {
    let mut http_only = false;
    let mut https_only = false;
    let mut verify_tls = false;
    let mut help = false;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--http-only" => http_only = true,
            "--https-only" => https_only = true,
            "--verify-tls" => verify_tls = true,
            "--help" | "-h" => help = true,
            _ => {}
        }
    }
    Args {
        http_only,
        https_only,
        verify_tls,
        help,
    }
}

fn print_usage() {
    println!(
        "\
用法: cargo run --bin proxy_smoke -- [选项]

选项:
  --http-only    只测 HTTP_PROXY_URL（默认 {DEFAULT_HTTP}）
  --https-only   只测 HTTPS_PROXY_URL（默认 {DEFAULT_HTTPS}）
  --verify-tls   HTTPS 校验证书（默认接受自签，仅本地测试）
  -h, --help     显示本说明

环境变量:
  HTTP_PROXY_URL    HTTP 代理基地址
  HTTPS_PROXY_URL   HTTPS 代理基地址
  TEST_MODEL        上游模型名（默认 gpt-4o-mini）
  OPENAI_API_KEY    非空时加入 Authorization: Bearer（OpenAI 兼容上游必填）
"
    );
}

fn env_trim(name: &str, default: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| default.to_string())
        .trim_end_matches('/')
        .to_string()
}

fn client(strict_tls: bool) -> Result<Client, reqwest::Error> {
    let mut b = Client::builder().timeout(Duration::from_secs(120));
    if !strict_tls {
        b = b.danger_accept_invalid_certs(true);
    }
    b.build()
}

async fn post_generate(
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
    println!("--- {label}: POST {base_url}/v1/chat/completions ---");
    let obj = post_generate(&c, base_url).await?;
    let preview = obj
        .pointer("/choices/0/message/content")
        .and_then(|x| x.as_str())
        .or_else(|| obj.get("response").and_then(|x| x.as_str()))
        .unwrap_or("")
        .chars()
        .take(200)
        .collect::<String>();
    println!("    response 预览: {preview:?}");
    println!("    OK ({label})\n");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = parse_args();
    if args.help {
        print_usage();
        return Ok(());
    }
    let http_u = env_trim("HTTP_PROXY_URL", DEFAULT_HTTP);
    let https_u = env_trim("HTTPS_PROXY_URL", DEFAULT_HTTPS);

    let mut errors: u32 = 0;

    if args.https_only {
        if let Err(e) = run_one("HTTPS", &https_u, args.verify_tls).await {
            eprintln!("FAIL HTTPS: {e}");
            errors += 1;
        }
        std::process::exit(if errors > 0 { 1 } else { 0 });
    }

    if args.http_only {
        if let Err(e) = run_one("HTTP", &http_u, true).await {
            eprintln!("FAIL HTTP: {e}");
            errors += 1;
        }
        std::process::exit(if errors > 0 { 1 } else { 0 });
    }

    if let Err(e) = run_one("HTTP", &http_u, true).await {
        eprintln!("FAIL HTTP: {e}");
        errors += 1;
    }

    if let Err(e) = run_one("HTTPS", &https_u, args.verify_tls).await {
        eprintln!("FAIL HTTPS: {e}");
        errors += 1;
    }

    if errors > 0 {
        eprintln!("完成: {errors} 项失败");
        std::process::exit(1);
    }
    println!("完成: HTTP 与 HTTPS 均通过");
    Ok(())
}
