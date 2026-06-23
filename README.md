# llm-audit

A small Rust reverse proxy for [OpenAI](https://OpenAI.com/) HTTP APIs. It forwards **POST** requests unchanged and, when configured, sends traces to [Langfuse](https://langfuse.com/) via the public [ingestion API](https://langfuse.com/docs/api) (`POST /api/public/ingestion`).

[中文版说明](README_zh.md)

## Features

- Transparent proxy: same path and query as the client, body and headers forwarded to OpenAI.
- **Streaming** responses: chunks are streamed to the client while the full stream is buffered to build a best-effort `output` for Langfuse after the stream ends.
- **Non-streaming** responses: full body is read, then a Langfuse `generation-update` is sent.
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
| `LANGFUSE_PUBLIC_KEY` | For Langfuse | — | Langfuse API public key (HTTP Basic username). |
| `LANGFUSE_SECRET_KEY` | For Langfuse | — | Langfuse API secret key (HTTP Basic password). |
| `LANGFUSE_BASE_URL` | No | `https://cloud.langfuse.com` | Langfuse host (e.g. self-hosted `http://localhost:3000`). |
| `LANGFUSE_ENABLE` | No | (unset: keys decide) | If `0`, `false`, `no`, or `off` (case-insensitive), **disable** Langfuse ingestion even when public/secret keys are set. When unset or any other value, ingestion still requires **both** keys non-empty. |

Langfuse is enabled only when **both** `LANGFUSE_PUBLIC_KEY` and `LANGFUSE_SECRET_KEY` are non-empty and `LANGFUSE_ENABLE` is not explicitly turned off.

| `AUDIT_LOG_ENABLE` | No | (off) | If `1`, `true`, `yes`, or `on` (case-insensitive), write proxied LLM request/response audit records (`target: llm_audit`) to the rolling log file for every call, regardless of Langfuse success. |
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
curl http://127.0.0.1:19001/api/chat -d '{"model":"llama3","messages":[{"role":"user","content":"hi"}],"stream":false}'
```

With HTTPS enabled, use `https://…`; for self-signed certs, `curl` may need `-k` (insecure) or `--cacert` pointing at your CA.

```bash
curl https://127.0.0.1:19001/api/chat -k -d '{"model":"llama3","messages":[{"role":"user","content":"hi"}],"stream":false}'
```

Aside from **`GET /health`**, only **POST** is registered for proxied paths; other methods are not forwarded.

## Langfuse events

For each proxied POST:

1. **`trace-create`** — trace name like `llm /api/chat`, metadata includes `path` and `source: llm-audit`, `input` is the request JSON (or a string if not valid JSON).
2. **`generation-create`** — linked to the trace, `model` from the request body when present, `input` same as above.
3. **`generation-update`** — after the OpenAI response is complete, `output` is derived from JSON fields such as `message.content` or `response`, or from newline-delimited JSON chunks when `stream: true`.

Ingestion runs in background tasks; failures are logged at `warn` and do not change the HTTP response to the client.

## License

If you add a license file, describe it here.
