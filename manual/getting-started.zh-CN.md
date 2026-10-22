# 快速开始

[English](getting-started.md) · 中文

## 环境

- 较新的 Rust 工具链
- 在项目根目录执行构建与运行

## 创建应用

新增应用 crate，依赖运行时装配库并完成 bootstrap。应用名与装配配置中的入口应用一致。

```rust
use frame_compose::AppBuilder;

let app = AppBuilder::new()
    .app("my-app")
    .bootstrap()
    .await;
```

`AppHandle` 提供 `gateway`、`host`、`expose`、`catalog` 与 `control()`。

## 调用

```rust
use frame_core::{Gateway, InstanceId, InvokeRequest, MethodId};
use serde_json::json;

let response = app
    .gateway
    .invoke(InvokeRequest::new(
        InstanceId::new("read_file")?,
        MethodId::new("run")?,
        json!({ "path": "README.md" }),
    ))
    .await?;
```

同应用的 `ctx.call` 不受 `AbilityExpose` 限制。调用其他应用的 Ability 要求目标 `open: true`。默认装配会链接仓库根 `abilitys/` 下的共享能力。

## 订阅事件

```rust
use ability_support::{EventClass, EventFilter, EventListener};

let mut events = app
    .control()
    .listen(EventFilter {
        class: Some(EventClass::Ability),
        ..Default::default()
    })
    .await?;

while let Some(event) = events.next().await {
    println!("{} {}", event.name(), event.payload());
}
```

`listen` 收到本应用、共享能力以及已开放的其他应用 Ability 事件。`intercept` 作用于本应用与共享能力。

## 示例

仓库提供可运行的示例应用（Agent 与 Web 场景）。在根目录使用 `cargo run -p <应用 crate 名> -- …` 启动；具体包名见 workspace 成员列表。

生态插件为可选，见 [生态插件](ecosystem-plugins.zh-CN.md)。

## 后续阅读

- [清单与裁剪](catalog-and-expose.zh-CN.md) — 协议清单与实例过滤的代码用法
- [编写 Ability](writing-abilities.zh-CN.md)
