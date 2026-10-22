# 编写 Ability

[English](writing-abilities.md) · 中文

## 包布局

每个 Ability 一个目录，根目录放置 `ability.yml`。Rust 包典型结构：

```
ability.yml
build.rs
src/
  logic/          # 业务代码
  generated/      # 声明驱动生成代码
cli|mcp|tools|grpc|skill/
  info.yml        # 协议投影（按需）
```

共享能力位于 `abilitys/<分类>/<name>`，应用自有能力位于 `apps/<app>/ability/<name>`；包内布局相同。

## Handler 与 Factory

- **Handler**（默认）：每个 method 对应 `src/logic/<method>.rs`，导出 `generated::provider::provider`
- **Factory**（`factory: true`）：绑定期配置构造实例时，自定义 `Ability` + `into_factory`

## 最小声明

```yaml
framework_version: "1"
id: echo
protocols: [tools, mcp, cli, http, fun]
depends: []
stateful: false
methods:
  - id: run
    description: Echo text
    input:
      type: object
      properties:
        text: { type: string }
      required: [text]
```

`framework_version` 须与框架一致；存在协议 `info.yml` 时版本需对齐。

## 可见性

`open` 默认为 false，仅属主应用可调用。其他应用需要调用时设 `open: true`。事件拦截属于属主应用。其他应用可以通过 `AppControl` 监听已开放的 Ability。

## 跨能力调用

只依赖对方的 **client** 契约：

```rust
use ability_echo::EchoExt;

ctx.echo().run(args).await?;
```

## 对外表面

| 目标 | 声明 |
|---|---|
| LLM tools | `protocols` 含 `tools` |
| MCP | `mcp` + `mcp/info.yml` |
| CLI | `cli` + `cli/info.yml` |
| HTTP / Fun | `protocols` 含 `http` / `fun` |

在应用中通过 `AbilityExpose` 启用实例。见 [清单与裁剪](catalog-and-expose.zh-CN.md)。
