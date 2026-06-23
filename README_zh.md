# llm-audit

面向 [OpenAI](https://OpenAI.com/) HTTP API 的轻量 Rust 反向代理：将 **POST** 请求原样转发到 OpenAI；在配置好密钥时，通过 Langfuse 公共 [Ingestion API](https://langfuse.com/docs/api)（`POST /api/public/ingestion`）把调用记录写入 [Langfuse](https://langfuse.com/) 做可观测性。

[English README](README.md)

## 功能

- 透明代理：路径与查询字符串与客户端一致，请求体与请求头转发到 OpenAI。
- **流式**响应：边向客户端推送分块，边在服务端聚合；流结束后尽力解析并上报 Langfuse 的 `output`。
- **非流式**响应：读完整包体后再发送 Langfuse `generation-update`。
- Langfuse **可选**：公钥与私钥未同时配置时，仅跳过上报，代理照常。
- **健康检查**：`GET /health` 返回 JSON（不访问上游 LLM），便于负载均衡或编排探活。
- **滚动文件日志**：使用 `tracing-appender`，默认写入 `LOG_DIR`（默认 `logs/`）、文件名前缀 `llm-proxy`，按天滚动；默认同时输出到控制台，可通过环境变量关闭。
- **可选 HTTPS**：同时设置非空的 `TLS_CERT_PATH` 与 `TLS_KEY_PATH`（PEM）时，在 `BIND_ADDR` 上使用 [Rustls](https://github.com/rustls/rustls) 提供 TLS；未设置则仍为 HTTP。

## 环境要求

- Rust 工具链（与 `Cargo.toml` 中的 edition 2024 一致）。

## 配置说明

优先从进程环境变量读取；若当前工作目录存在 `.env`，会通过 [dotenvy](https://crates.io/crates/dotenvy) 加载（文件不存在则忽略）。

| 变量 | 是否必填 | 默认值 | 说明 |
|------|----------|--------|------|
| `LLM_URL` | 否 | `http://127.0.0.1:11434` | OpenAI 根地址（不要写 API 路径；客户端路径会拼在后面）。末尾 `/` 可写可不写，程序会自动去掉。 |
| `BIND_ADDR` | 否 | `127.0.0.1:19001` | 代理监听地址，格式 `host:port`。 |
| `TLS_CERT_PATH` | 否 | — | PEM 证书（链）文件路径。须与 `TLS_KEY_PATH` **同时**设为非空字符串才会在 `BIND_ADDR` 上启用 HTTPS；否则监听 HTTP。 |
| `TLS_KEY_PATH` | 否 | — | PEM 私钥文件路径。与 `TLS_CERT_PATH` 配对使用，规则同上。 |
| `HTTP_CLIENT_TIMEOUT_SECS` | 否 | `600` | 转发上游与 Langfuse 共用的 HTTP 客户端整请求超时（秒）；含读响应体，流式长连接须在时限内结束。设为 `0` 关闭超时（与原先无超时行为一致）。非法数值按 `600` 处理。 |
| `LANGFUSE_PUBLIC_KEY` | 启用 Langfuse 时需要 | — | Langfuse API 公钥（HTTP Basic 用户名）。 |
| `LANGFUSE_SECRET_KEY` | 启用 Langfuse 时需要 | — | Langfuse API 私钥（HTTP Basic 密码）。 |
| `LANGFUSE_BASE_URL` | 否 | `https://cloud.langfuse.com` | Langfuse 实例地址（自建示例：`http://localhost:3000`）。 |
| `LANGFUSE_ENABLE` | 否 | （未设置则按密钥决定） | 设为 `0`、`false`、`no`、`off`（大小写不敏感）时**关闭**向 Langfuse 写入，即使已配置公钥与私钥；未设置或非上述关闭值时，仍须公钥与私钥均非空才会上报。 |

仅当 **`LANGFUSE_PUBLIC_KEY` 与 `LANGFUSE_SECRET_KEY` 均非空**，且未将 `LANGFUSE_ENABLE` 设为关闭值时，才会启用 Langfuse。

| `AUDIT_LOG_ENABLE` | 否 | 关闭 | 设为 `1`、`true`、`yes`、`on`（大小写不敏感）时，对每条经本代理拦截并转发的 LLM 请求与响应写入本地审计日志文件（`target: llm_audit`），与 Langfuse 是否上报成功无关。 |
| `AUDIT_LOG_MAX_CHARS` | 否 | `16384` | 审计日志里 `input` / `output` JSON 的最大 UTF-8 字节数。设为 `0` 表示**不截断**（完整写入）。其它正整数为自定义上限；非法值按 `16384` 处理。 |
| `LOG_DIR` | 否 | `logs` | 滚动日志目录（不存在会自动创建）。 |
| `LOG_ROTATION` | 否 | `daily` | `daily`，或 `hourly`/`hour`，或 `minutely`/`minute`。 |
| `LOG_DISABLE_STDOUT` | 否 | 关闭 | 为真时只写文件、不往控制台打日志。 |
| `RUST_LOG` | 否 | `info` | `tracing` 过滤规则，例如 `info,llm_audit=debug`。 |

**安全提示：** 勿将真实密钥提交到仓库；用 `.gitignore` 忽略 `.env`。默认日志目录 `logs/` 已在 `.gitignore` 中忽略。

## 运行

构建与部署说明见 [docs/deployment.md](docs/deployment.md)。

## 客户端如何对接

把 OpenAI 客户端/SDK 的基地址改成代理地址（即 `BIND_ADDR` 对应的 `http://主机:端口` 或 `https://主机:端口`，取决于是否配置了 TLS），而不是直连 OpenAI。`curl` 示例（HTTP）：

```bash
curl http://127.0.0.1:19001/api/chat -d '{"model":"llama3","messages":[{"role":"user","content":"hi"}],"stream":false}'
```

若已启用 HTTPS，请将 URL 改为 `https://…`，自签证书场景下对 `curl` 可加上 `-k`（跳过校验）或 `--cacert` 指向你的 CA。

```bash
curl https://127.0.0.1:19001/api/chat -k -d '{"model":"llama3","messages":[{"role":"user","content":"hi"}],"stream":false}'
```

除 **`GET /health`** 外，代理路径仅注册 **POST**；其它方法与路径不会被转发。

## 写入 Langfuse 的事件

每次被代理的 POST 大致对应：

1. **`trace-create`** — 名称形如 `llm /api/chat`，`metadata` 含 `path` 与 `source: llm-audit`，`input` 为请求 JSON（无法解析则为字符串）。
2. **`generation-create`** — 绑定同一 trace，从请求体读取 `model`（若有），`input` 同上。
3. **`generation-update`** — OpenAI 响应结束后写入 `output`：优先从 JSON 的 `message.content`、`response` 等字段提取；`stream: true` 时按行解析 NDJSON 并拼接文本。

上报在独立异步任务中执行；失败会打 `warn` 日志，**不会**改变返回给调用方的 HTTP 响应。

## 许可证

若你添加了许可证文件，可在此补充说明。
