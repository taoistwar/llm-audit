# deployment

## building
```bash
# 后端，使用 musl 进行全静态编译。（musl 是一个轻量级的 C 标准库，支持完全静态链接，生成的可执行文件不依赖目标系统的任何动态库。）
rustup target add x86_64-unknown-linux-musl
cargo build --release  --target x86_64-unknown-linux-musl
```
