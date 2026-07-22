# deployment

## quick start (development)

```bash
cargo run
```

## release build

### dyn build

```bash
cargo build --release
./target/release/llm-audit
```

### static build (musl)

```bash
# 使用 musl 进行全静态编译，生成的可执行文件不依赖目标系统的任何动态库。
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

## config

config file `.env`, the content:

```env
LLM_URL=https://api.haimacloud.com/
BIND_ADDR=0.0.0.0:19001
AUDIT_LOG_ENABLE=false

LOG_DISABLE_STDOUT=true

LANGFUSE_ENABLE=true
LANGFUSE_BASE_URL=http://172.16.208.150:3000
LANGFUSE_PUBLIC_KEY=pk-lf-113a3a8f-b48d-47d9-a8a2-6009f31ba9af
LANGFUSE_SECRET_KEY=sk-lf-0d81e9a8-0e26-4879-8361-c575d63a3aff
```

## test

change YOUR_API_KEY to your real token:

```bash
curl http://127.0.0.1:19001/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -H 'Authorization: Bearer YOUR_API_KEY' \
  -d '{
    "model":"deepseek-v4-flash",
    "messages":[{"role":"user","content":"hi"}],
    "stream":false
  }'
```