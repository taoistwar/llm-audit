# deployment

## quick start (development)

```bash
cargo run
```

## release build

```bash
cargo build --release
./target/release/llm-audit
```

## static build (musl)

```bash
# 使用 musl 进行全静态编译，生成的可执行文件不依赖目标系统的任何动态库。
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```
