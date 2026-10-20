# ModalPrism

## 构建

构建需要系统已安装 `protoc`（Protocol Buffers 编译器）。`lance` 的构建脚本会调用它。Debian 与 Ubuntu 上的包名是 `protobuf-compiler`。

```text
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
