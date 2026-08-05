# llm-audit

A small Rust reverse proxy for [OpenAI](https://OpenAI.com/) HTTP APIs. It forwards **POST** requests unchanged and, when configured, sends traces to [Langfuse](https://langfuse.com/) via the public [ingestion API](https://langfuse.com/docs/api) (`POST /api/public/ingestion`).

[中文版说明](README_zh.md)

## Features

- Transparent proxy: same path and query as the client, body and headers forwarded to OpenAI.
- **Streaming and non-streaming** responses are forwarded in chunks while only a bounded prefix is captured for Langfuse.
- Langfuse is **optional**: if public/secret keys are not both set, the proxy still runs and only skips ingestion.
- **`GET /health`** JSON liveness probe (does not call the upstream LLM).
- **Rolling file logs** via [`tracing-appender`](https://crates.io/crates/tracing-appender): files under `LOG_DIR` with prefix `llm-proxy`, rotated daily by default; stdout mirror unless disabled.
- Optional **HTTPS**: when both `TLS_CERT_PATH` and `TLS_KEY_PATH` are set to non-empty PEM file paths, the proxy serves TLS on `BIND_ADDR` via [Rustls](https://github.com/rustls/rustls); otherwise it stays HTTP-only.

## Requirements

- Rust toolchain (edition 2024 as specified in `Cargo.toml`).

## Configuration

Environment variables are read from the process environment. If a `.env` file is present in the working directory, it is loaded first via [dotenvy](https://crates.io/crates/dotenvy) (missing file is ignored).

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `LLM_URL` | No | `http://127.0.0.1:11434` | OpenAI base URL (no API path here; client paths are appended). A trailing slash is optional and normalized away. |
| `BIND_ADDR` | No | `127.0.0.1:19001` | Address the proxy listens on (`host:port`). |
| `TLS_CERT_PATH` | No | — | Path to the PEM certificate (chain). HTTPS is enabled only when **both** this and `TLS_KEY_PATH` are set to non-empty strings; otherwise the listener is plain HTTP. |
| `TLS_KEY_PATH` | No | — | Path to the PEM private key. Used together with `TLS_CERT_PATH` as above. |
| `HTTP_CLIENT_TIMEOUT_SECS` | No | `600` | Per-request timeout (seconds) for the shared HTTP client used for upstream and Langfuse (includes reading the body; long streams must finish within the limit). Set to `0` to disable (matches previous no-timeout behavior). Invalid values fall back to `600`. |
| `REQUEST_BODY_MAX_BYTES` | No | `16777216` | Maximum request body size. Larger requests receive `413 Payload Too Large`. |
| `RESPONSE_CAPTURE_MAX_BYTES` | No | `4194304` | Maximum response bytes captured for Langfuse/audit logs. The full response is still forwarded and truncated captures are marked. Set to `0` to disable response capture. |
| `LANGFUSE_PUBLIC_KEY` | For Langfuse | — | Langfuse API public key (HTTP Basic username). |
| `LANGFUSE_SECRET_KEY` | For Langfuse | — | Langfuse API secret key (HTTP Basic password). |
| `LANGFUSE_BASE_URL` | No | `https://cloud.langfuse.com` | Langfuse host (e.g. self-hosted `http://localhost:3000`). |
| `LANGFUSE_ENABLE` | No | (unset: keys decide) | If `0`, `false`, `no`, or `off` (case-insensitive), **disable** Langfuse ingestion even when public/secret keys are set. When unset or any other value, ingestion still requires **both** keys non-empty. <br/><br/>Langfuse is enabled only when **both** `LANGFUSE_PUBLIC_KEY` and `LANGFUSE_SECRET_KEY` are non-empty and `LANGFUSE_ENABLE` is not explicitly turned off.|
| `AUDIT_LOG_ENABLE` | No | (off) | If enabled, write proxied request/response content to the local audit log. When disabled, content is never logged locally, including when Langfuse is unavailable. |
| `AUDIT_LOG_MAX_CHARS` | No | `16384` | Max UTF-8 bytes for `input` / `output` JSON in audit logs. Set to `0` for **no truncation** (full body). Other positive integers set a custom limit. Invalid values fall back to `16384`. |
| `LOG_DIR` | No | `logs` | Directory for rolling log files (created if missing). |
| `LOG_ROTATION` | No | `daily` | `daily`, `hourly` / `hour`, or `minutely` / `minute`. |
| `LOG_DISABLE_STDOUT` | No | (off) | If `true` / `1` / `yes` / `on`, only write logs to files (no console). |
| `RUST_LOG` | No | `info` | [`tracing` filter](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html), e.g. `info,llm_audit=debug`. |

**Security:** Do not commit real API keys. Keep `.env` out of version control (see `.gitignore`). The default `logs/` directory is listed in `.gitignore`.

## Run

See [docs/deployment.md](docs/deployment.md) for build and deployment instructions.

## Point clients at the proxy

Configure your OpenAI client or SDK to use the proxy base URL derived from `BIND_ADDR` as `http://host:port` or `https://host:port`, depending on whether TLS env vars are set, instead of OpenAI directly. Example with `curl` (HTTP):

```bash
curl http://localhost:19001/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer sk-xxx" \
  -d '{
    "model": "deepseek-v4-flash",
    "messages": [
      {"role": "system", "content": "You are a helpful assistant."},
      {"role": "user", "content": "Hi, please introduce yourself in one sentence."}
    ],
    "temperature": 0.7,
    "max_tokens": 500
  }'
```

With HTTPS enabled, use `https://…`; for self-signed certs, `curl` may need `-k` (insecure) or `--cacert` pointing at your CA.

```bash
curl http://localhost:19001/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer sk-xxx" \
  -d '{
    "model": "deepseek-v4-flash",
    "stream":true,
    "messages": [
      {"role": "system", "content": "You are a helpful assistant."},
      {"role": "user", "content": "Hi, please introduce yourself in one sentence."}
    ],
    "temperature": 0.7,
    "max_tokens": 500
  }'
```

Aside from **`GET /health`**, only **POST** is registered for proxied paths; other methods are not forwarded.

## Langfuse events

For each proxied POST:

1. **`trace-create`** — trace name like `llm /api/chat`, metadata includes `path` and `source: llm-audit`, `input` is the request JSON (or a string if not valid JSON).
2. **`generation-create`** — linked to the trace, `model` from the request body when present, `input` same as above.
3. **`generation-update`** — after the response completes, derive `output` and recognized token usage from OpenAI Chat Completions JSON/SSE, common Responses API fields, or Ollama NDJSON. Truncation and connection errors are marked in the output.

Ingestion runs in tracked background tasks. Network errors, `429`, and server errors are retried up to three times; final failures are logged at `warn` and do not change the HTTP response. Graceful shutdown waits up to 10 seconds for pending tasks.

## License

If you add a license file, describe it here.
