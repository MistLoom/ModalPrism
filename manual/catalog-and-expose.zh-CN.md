# 清单、协议与 Expose

[English](catalog-and-expose.md) · 中文

场景应用分三步装配 Ability：发现源、用 `AbilityExpose` 决定加载哪些实例，再在得到的 `Gateway` 上打开 Port。

框架扫描仓库根 `abilitys/` 与各应用 `apps/<app>/ability/` 下的 `ability.yml`。共享能力对入口应用可调用、可监听、可拦截。每条 source 带有属主应用和 `open`。进程用 `AppBuilder::app` 或 `ComposeOptions::app` 声明入口应用。调用其他应用的 Ability 要求 `open: true`。监听与拦截走 `AppHandle::control`。

## 协议声明

每个 Ability 包在 `ability.yml` 中声明支持的协议。需要投影细节时，使用对应目录下的 `info.yml`（如 `mcp/`、`cli/`、`tools/`）。

```yaml
framework_version: "1"
id: read_file
protocols: [tools, mcp, cli, http, fun]
methods:
  - id: run
    description: Read a file
    input:
      type: object
      properties:
        path: { type: string }
      required: [path]
```

| 协议 | 声明位置 | 作用 |
|---|---|---|
| `tools` | `protocols`（可选 `tools/info.yml`） | LLM tool 发现 / `dispatch` |
| `mcp` | `protocols` + `mcp/info.yml` | MCP tool 投影 |
| `cli` | `protocols` + `cli/info.yml` | CLI Port / 独立 CLI 映射 |
| `grpc` | `protocols` + `grpc/info.yml` | gRPC 映射 |
| `skill` | `protocols` + `skill/info.yml` | Skill 元数据 |
| `http` / `fun` | 仅 `protocols` | HTTP / 进程内 Fun |

## 按协议获取工作区清单

```rust
use ability_support::ProtocolKind;
use frame_compose::WorkspaceCatalog;

let catalog = WorkspaceCatalog::discover();

let tools = catalog.sources_by_protocol(ProtocolKind::Tools);
let mcp = catalog.sources_by_protocol(ProtocolKind::Mcp);

let linked_tools: Vec<_> = tools
    .into_iter()
    .filter(|s| s.linked)
    .collect();
```

常用字段：`id`、`rel_root`、`root`、`linked`（当前二进制是否已链接该 Provider）。

## Expose 过滤

```rust
use frame_core::AbilityExpose;
use serde_json::json;

let expose = AbilityExpose::from_value(&json!({
    "unknown": "deny",
    "defaults": { "enabled": true },
    "abilities": {
        "write_file": { "enabled": false },
        "read_file": {
            "enabled": true,
            "config": { "max_size": 1_048_576 }
        }
    }
}))?;

assert!(expose.is_enabled("read_file"));
assert!(!expose.is_enabled("write_file"));

let enabled = expose.filter_enabled(["read_file", "write_file", "bash"]);
// unknown=deny 时未列出的 id 为关闭 ⇒ ["read_file"]
```

| API | 行为 |
|---|---|
| `AbilityExpose::from_value` | 解析 Expose JSON（`abilities` 映射；兼容旧键） |
| `AbilityExpose::open_all` | 默认启用未单独配置的源 |
| `is_enabled` / `resolve` | 单实例启用状态与绑定期 `config` |
| `filter_enabled` | 保留解析为启用的 id |

Expose 裁剪的是 **Port → Gateway** 的加载与可见性；能力之间的 `ctx.call` 不受 Expose 限制。

## 协议清单与 Expose 求交

```rust
use ability_support::ProtocolKind;
use frame_compose::{AppBuilder, WorkspaceCatalog};
use frame_core::AbilityExpose;
use serde_json::json;

let catalog = WorkspaceCatalog::discover();
let expose = AbilityExpose::from_value(&json!({
    "unknown": "deny",
    "defaults": { "enabled": true },
    "abilities": {
        "write_file": { "enabled": false },
        "bash": { "enabled": false }
    }
}))?;

let mcp_surface: Vec<_> = catalog
    .sources_by_protocol(ProtocolKind::Mcp)
    .into_iter()
    .filter(|s| s.linked && expose.is_enabled(&s.id))
    .map(|s| s.id.clone())
    .collect();

let tools_surface: Vec<_> = catalog
    .sources_by_protocol(ProtocolKind::Tools)
    .into_iter()
    .filter(|s| s.linked && expose.is_enabled(&s.id))
    .map(|s| s.id.clone())
    .collect();

let app = AppBuilder::new()
    .catalog(catalog)
    .expose(expose)
    .bootstrap()
    .await;
```

装配完成后，Port 会按 access gate 过滤：仅「已 load 且登记了该 Port 协议」的实例在该入口可见。

## 相关文档

- [快速开始](getting-started.zh-CN.md)
- [编写 Ability](writing-abilities.zh-CN.md)
