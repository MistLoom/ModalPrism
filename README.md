# ModalPrism

## 构建

构建需要系统已安装 `protoc`（Protocol Buffers 编译器）。`lance` 的构建脚本会调用它。Debian 与 Ubuntu 上的包名是 `protobuf-compiler`。

```text
cargo build --workspace
cargo test --workspace --exclude 'frame-*' --exclude 'ability-support*' --exclude ai-protocol --exclude ability-core
cargo clippy --workspace --all-targets --exclude 'frame-*' --exclude 'ability-support*' --exclude ai-protocol --exclude ability-core -- -D warnings
```

`frame/`、`SDK/` 与 `apps/ability-core` 是随仓库提供的 LEAF 框架代码。测试与 clippy 门禁只覆盖本项目 crate，框架按上游状态构建。
