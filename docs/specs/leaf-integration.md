# LEAF 集成要求

对照 LEAF 提交 `ffabb6128ae6e006c6c3cee3a706532501f26219`。

本文只记录该提交中已经存在的行为，以及 ModalPrism 必须满足的要求。

## 依赖边界

| 包 | 允许的依赖 | 禁止的依赖 |
|---|---|---|
| 领域库（`crates/`） | 本仓库 crate 与第三方库 | `ability-sdk`、`ability-support`、`frame-core`、`frame-compose`、`ability-core`、任一 `frame-port-*` |
| Ability crate（`ability/`） | `ability-support` | `frame-core`、`frame-compose`、`ability-core`、任一 `frame-port-*` |
| 组合根（`apps/modalprism-collector`、`apps/modalprism-agent`） | `frame-core`、`ability-core`、`frame-port-http`、`frame-port-mcp` | Ability 的实现逻辑 |

`ability-sdk` 再导出 `ability-support`，并声明自身不依赖 `frame-core` 与 port。`ability-sdk` 在 LEAF workspace 的 exclude 列表中，Cargo 不能把它当作该仓库的 git 依赖。Ability crate 依赖同一提交的 `ability-support`。`ability-support` 的 crate 说明禁止依赖 `frame-core`、port 与 `ability-core`。架构约束第 7 条写的是：外部包不得依赖 `frame-core`、port 或 `ability-core`。组合根为链接本仓库 Ability，直接依赖 `frame-core` 与 `ability-core`。采集进程的运维 HTTP 端口依赖 `frame-port-http`。只读进程的工具面依赖 `frame-port-mcp`。该提交的 MCP Port 使用标准输入输出，把 Gateway 目录里的实例 id 当作工具名。只读进程交给该端口的目录把每个方法展开成实例 id 为 `instance.method` 的单独条目，因此列出的工具名是 `instance.method`。`AccessKind::Tools` 没有 Port。该提交没有 Skill Port。领域库与 Ability crate 不依赖 `frame-core` 与 `ability-core`。

Ability 实现 `Ability` / `AbilityProvider`，通过 `manifest`、`create`、`invoke`、`on_lifecycle` 接入。跨 Ability 调用只使用对方的 client 契约或 `AbilityCtx::call`。

## 发现与链接

LEAF 的发现范围是固定的。

- `discover_ability_roots` 只收集 `<workspace>/common/ability/<name>/ability.yml` 与 `<workspace>/apps/<app>/ability/<name>/ability.yml`。
- `WorkspaceCatalog::discover` 与 `frame-compose` 的 `build.rs` 使用的 workspace 根，是 `frame-compose` 编译时 `CARGO_MANIFEST_DIR` 的上两级，即 LEAF 仓库根。
- `linked` 表示该 id 出现在 `frame-compose` 按上述目录生成的 `linked_ids()` 中，并且对应 Cargo feature 已打开。
- `AppBuilder::bootstrap` 先 `install_all`（只安装上述已链接 Provider），再 `load_exposed`（只 load 清单中 `linked` 且 Expose 启用的 id）。
- `WorkspaceCatalog` 没有追加外部根目录的接口。

因此本仓库的 `ability.yml` 不会被发现，也不会被编译进 `frame-compose`。

本仓库 Ability 的链接要求：

1. Ability crate 导出 `AbilityProvider`。它不调用 Host。
2. 组合根在进程内执行 `install(source, provider)`，再 `load(LoadSpec)`。`LoadSpec.source` 必须已经 install。实例名在同一 Host 内不可重复。
3. `stateful: true` 的能力在一个进程内只允许一个实例。`strata_read`、`strata_write`、`strata_queue`、`committer`、`planner`、`runner`、`health`、`config`、`egress`、`paid_gateway` 按此声明。
4. 归属在 load 时从 ability id 复制到实例 id。`authorize_port` 与 `authorize_call` 在目标没有归属记录时放行；有记录时，要求调用方属于同一应用，或目标 `open` 为 true。组合根若需要隔离，必须在 load 前写入归属。
5. `AppControl` 的可监听集合只来自 `WorkspaceCatalog`。本仓库实例不在该集合内。数据面变更不走这条监听路径，见下文事件一节。
6. `set_protocol_support` 未登记的实例，在 Host 设置了 `access_gate` 时会被 `invoke` 拒绝。未设置 `access_gate` 时，`invoke` 不检查协议声明。
7. Host 的 `access_gate` 只有一个 `Option<AccessKind>`，且位于 Host 内部共享状态上。一个 Host 不能同时按多个协议过滤。`frame/port-http` 自身不按协议声明过滤目录。
8. 组合根在打开每个端口之前，限制该端口可见的实例集合，使该端口只暴露已登记对应协议的实例。不用单个 `access_gate` 代表全部端口。

`external/showcase` 是 LEAF 仓库内的仓外接入示例，它直接依赖 `frame-compose`、`frame-core` 与 `frame-port-fun`。ModalPrism 的组合根同样直接依赖 `frame-core` 与 `ability-core` 完成本仓库 Ability 的 install 与 load。

远程进程使用另一条路径：客端只依赖 `ability-sdk` 的 `serve` / `serve_stdio`，帧格式在 `ability_support::remote`，一行一帧 JSON。宿主侧 `ProcProvider` 与 `connect` 位于 `ability-core`。`request_frame` 的超时是 10 秒，覆盖调用与生命周期应答。ModalPrism 的采集进程与 Ability 同进程链接，不把数据面放在这条远程路径上。

## 大载荷与二进制

调用信封是 `instance + method + args`。`InvokeRequest.args` 与 `InvokeResponse.data` 的类型是 `serde_json::Value`。远程帧的 `Invoke.args` 与 `Ok.value` 同样是 JSON 值。这条路径没有二进制参数，也没有流式返回值。

LEAF 仓库内 `store` 的二进制方法把字节做成 base64 字符串，按 24 KiB 分块放进 JSON。那是 `store` 自己的载荷形式。ModalPrism 的部件字节不使用该形式，也不进入 Gateway。

要求：

1. 访问器把部件字节写入 Strata 暂存区，以 sha256 与长度标识。
2. 信封、适配器输入、变更集和 Gateway 参数只携带 `PartRef` 或区间引用。
3. 同一进程内按引用读取，返回共享字节缓冲。
4. 文本 facet 作为 JSON 字段传递。超过 `strata.inline_text_max` 的文本转入暂存区。
5. 暂存文件在引用它的变更集可见之后才删除。无引用文件按 `strata.staging.ttl` 清理。
6. 对外的变更订阅使用 `strata_read.changes`：调用方传入已见快照序号，返回该序号之后的已可见变更。不把数据面变更做成 Gateway 的流式响应。

## 事件

LEAF 有三类事件，实现在 `ability-core` 的 `EventBus`。

| 类 | 产生 | 投递 |
|---|---|---|
| Host | 引擎：`host.load`、`host.unload`、`ability.lifecycle.*`、`ability.invoke.before`、`ability.invoke.after`、`ability.invoke.error` | middleware，然后 pipeline，然后 message 与 parallel |
| Ability | 该 Ability 的 `ctx.emit`。名称必须在 `ability.yml` 中声明为 `dir: emit`，载荷须通过声明的 `payload_schema` | 同上 |
| User | `AbilityHost::emit_user` | 只交给 `UserEventSink`，不进入 middleware、pipeline、message、parallel |

Host 与 Ability 的投递语义：

- 处理者名单在本次 emit 开始时复制。之后的登记不影响本次。
- middleware 不调用 `next` 时，后面的处理者不会运行。
- pipeline 按登记顺序执行。`dispatch` 丢弃 pipeline 与 middleware 的错误，`ctx.emit` 在 schema 通过后返回成功。
- message 用 `tokio::spawn` 发出，emit 不等待。`listen` 注册为 message，通道是无界 `mpsc`。发送失败被忽略，接收端已关闭时该次事件丢失。通道不提供从序号续读。
- parallel 会被 emit 等待到全部结束。
- 并发的两次 emit 之间没有全局顺序锁。
- 实例 unload 时，引擎卸掉绑在该实例上的事件登记。
- 事件不落盘，没有确认，也没有重放。

`AppControl::listen` 只放行本应用、`common`，以及其他应用中 `open: true` 的 Ability 事件。事件没有实例时（用户事件）被丢掉。`intercept` 只作用于本应用与 `common`。

Strata 的 changefeed 不使用上述总线。可见性以快照序号为准，消费方保存自己已经处理的序号。进程内的 `watch` 与跨进程的 `changes` 都从该序号读取。丢失、重复和顺序由快照序号判定，不由 EventBus 的 message 通道判定。

Ability 仍可 `emit` 声明过的运行事件（例如健康告警）。这类事件只表示通知，不作为数据面的提交记录。

## 后台循环

LEAF 的范围是能力层运行时，应用主循环由使用方持有。生命周期由引擎推进：

`Loaded` → `Bound` → `on_lifecycle(Started)` → `Ready`

`Started` 返回错误时，相位变为 `Failed`，实例被移除。`Started` 返回之后实例才是 `Ready`。卸载顺序为 `Stopping` → `on_lifecycle(Stopped)` → 移除。默认的 `on_lifecycle` 为空。

引擎没有定时器、间隔任务或后台任务接口。

要求：

- Planner 节拍、Runner 工作者、Committer 刷写与维护、健康评估、配置监听，是组合根进程中的任务。
- 这些任务经 Gateway 调用对应方法。
- Ability 的 `on_lifecycle(Started)` 不得停在循环里。需要跨调用保留的状态放在 Factory 实例中。
- `on_lifecycle(Stopped)` 负责停止该实例自己启动的任务。引擎不会代为取消 Ability 内部的任务。

## 端口

`HttpPort` 接受一个 `SocketAddr`。路由为 `/health`、`/v1/abilities`、`/v1/abilities/{target}`、`/v1/abilities/{target}/invoke`。调用体是 JSON：`args`、可选 `method`、可选 `session_id`。实现中没有 TLS，没有鉴权，CORS 为 permissive。gRPC 与 MCP 端口的实现中同样没有鉴权与 TLS。

要求：

- 组合根传入监听地址。
- 不把这些端口描述为已鉴权入口。
- `push` 的原始请求体不进入上述 invoke 路由。push 监听由组合根单独绑定，鉴权与幂等键在该监听上完成。

## 热加载与实例集合

`apply_expose_delta`、`reload_one`、`AppHandle::reload_all` 只处理清单里 `linked` 的 id。它们不扫描本仓库，也不在运行时加载新的 Ability 二进制。

配置热加载继续由组合根完成：校验通过后切换快照指针，在途任务保持派发时的代际。Expose 中的实例集合、绑定期配置、控制存储后端、暂存区目录、Lance 建表参数和访问器实现标识变更时，重启进程。

## Web 控制台（非 LEAF 表面）

`docs/console.md` 描述的 Web UI 与 `/console/api/v1/*` 不属于 LEAF Ability 清单。实现为普通 HTTP 适配层（`console-api` → `console`），在采集进程内持有 `Arc<HostResources>`，通过文档化的 `HostResources::call("ability.method", …)` 调用采集后端，不实现 `AbilityProvider`，也不经 `/v1/abilities` 目录。LEAF 对齐范围仅覆盖 `ability/` 与组合根对 Ability 的 install/load、Gateway 循环与受限 ops 目录。

## 未决项

1. **端口访问控制。** LEAF 端口没有 TLS 与鉴权。端口访问控制（TLS、鉴权，以及环回地址以外的绑定）未规定。端口上不增加自定的鉴权协议。
