# 架构概览

[English](architecture.md) · 中文

## 模型

**应用**拥有自己的 Ability。外部门户与跨能力调用共用同一条 **Gateway** 信封：`instance + method + args`。

```
Ports (CLI / HTTP / MCP / Fun / …)
        │
     Gateway
        │
   入口应用
        ├─ 本应用 Ability
        └─ 其他应用的 open Ability
```

`ability.yml` 的 `open` 默认为 false。其他应用只能调用 `open: true` 的 Ability。

事件的监听和拦截由入口应用的 `AppControl` 登记。拦截只作用于本应用的 Ability。监听还可以收到其他应用已开放 Ability 的事件。过滤由引擎实现。

## 分层

| 层 | 职责 |
|---|---|
| 契约 | Manifest、Ctx、生命周期、客户端生成 |
| 传输 | Gateway、Port、Expose、控制器 |
| 引擎 | Host、事件总线、调用与事件准入 |
| 组合 | 目录发现、启动装配、`AppControl` |
| 能力 | 仓库根 `abilitys/<分类>/<name>` 为共享实现；`apps/<app>/ability/<name>` 为应用自有 |
| 应用 | 场景进程与装配策略 |

Node / Python 能力通过 **Guest 线协议** 接入，声明模型与 Rust 共用 `ability.yml`。见 [Guest 协议](ability-guest-protocol.zh-CN.md)。

## 可见性与治理

- **共享能力**：对入口应用开放调用、监听与拦截（具体集合由装配决定）。
- **应用自有能力**：默认仅属主可调用；`open: true` 时允许其他应用调用或监听。
- **仓外能力包**：通过仓库根 `SDK/` 下 Guest SDK 与线协议接入，不直接链接运行时内核。

## 约束

1. Port 只依赖 `Gateway`，不直接调用 Ability 实现
2. Ability 不按「来自 MCP / CLI / LLM tools」分支
3. Expose 只裁 **Port → Gateway**；它不授予跨应用调用
4. 跨应用调用要求目标 `open: true`
5. 拦截属于属主应用；监听其他应用能力要求目标 `open: true`
6. 协议身份属于声明与 Port，而不是内核业务类型
