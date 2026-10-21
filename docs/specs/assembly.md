# 装配与一键启动

本文固定采集进程的启动、Ability 链接和循环。存储见 `docs/specs/strata-storage.md`，配置见 `docs/specs/config.md`，调度见 `docs/specs/scheduling.md`，LEAF 链接见 `docs/specs/leaf-integration.md`。

## 组合根

采集进程在 `AbilityHost` 上完成全部 Ability 的 `install` 与 `load`，装配逻辑集中在 `apps/modalprism-assembly`（由 `modalprism-collector` / `modalprism-agent` 的 `boot` 调用）。有状态 Ability 声明 `factory: true` 与 `config_schema`，由 `LoadSpec.with_config` 在工厂中构造实例并持有状态；无状态 Ability 由 `ability.yml` codegen 生成 handler 提供者。`committer` 工厂打开 Strata；其余能力经 `AbilityCtx::call` 访问存储与治理面，不在进程级注册表共享句柄。进程内持有三个 Gateway 表面：

| 表面 | 用途 |
|---|---|
| `ConsoleGateway` | 控制台与组合根写循环：不经 HTTP 协议门控，应用内已加载实例均可调用 |
| `HttpOpsGateway` | 运维 HTTP `/v1/abilities`：`AbilityExpose` 白名单 + `AccessKind::Http` + `unknown: deny` |
| `McpReadGateway` | 只读进程 MCP：`AbilityExpose` 白名单 + `AccessKind::Mcp` + `unknown: deny` |

写入与控制类 Ability（`strata_write`、`strata_queue`、`committer`、`runner`、`egress` 等）在 manifest 上 `open: false`，并配置应用内 `grant`。对外 HTTP 与 MCP 目录仅列出显式启用的只读或运维查询实例；控制台受控写操作只经 `console-api` 进程内的 `ConsoleGateway` 调用。

只读进程 `modalprism-agent` 与采集进程共用同一组 Ability crate，经 `install_read_agent` 装配只读依赖子集，不加载写入面实例。

Web 控制台（`crates/console`、`crates/console-api`）不是 Ability。HTTP 在 ops 端口挂载 `/console/`，后端只持有 `Arc<dyn Gateway>`（控制台路径为 `ConsoleGateway`），见 `docs/console.md`。

## 命令

可执行文件是 `modalprism-collector`。

```text
modalprism-collector [--data DIR] [--config DIR] [--http ADDR] [--proxy ADDR] [--push ADDR]

同一 ops 端口提供 Web 控制台：`http://<http>/console/` 与 `/console/api/v1/*`。详见 `docs/console.md`。`cargo build -p modalprism-collector` 会在有 Node 时自动构建并嵌入前端；也可设置 `MODALPRISM_CONSOLE_DIST` 或 `SKIP_CONSOLE_WEB_BUILD`。
```

缺省：

| 参数 | 缺省 |
|---|---|
| `--data` | `var` |
| `--config` | `config` |
| `--http` | `127.0.0.1:17880` |
| `--proxy` | `127.0.0.1:17881` |
| `--push` | `127.0.0.1:17882` |

三个监听地址的 IP 必须是环回地址。其他地址拒绝启动，消息是 `监听地址不是环回地址`，并且不创建数据目录。端口实现没有 TLS，也没有鉴权协议。监听地址以外的访问控制未规定。

相对路径相对进程的当前目录。

## 启动顺序

1. 用 `config::validate` 读取配置目录。失败则进程退出，不打开数据目录。
2. 快照里出现下列任一情况时拒绝启动，消息是 `存储位置未规定`，不打开数据目录：
   - `strata.lance.uri` 非空
   - `strata.control_store.backend` 为 `postgres`
   - `strata.control_store.path` 与 `<data>/control.db` 不是同一路径
   - `strata.staging.dir` 与 `<data>/staging` 不是同一路径
3. 打开数据目录。`strata.blob.inline_size_threshold`、`strata.blob.dedicated_size_threshold` 和 `strata.staging.ttl` 取这次打开时的快照，缺省分别是 64KiB、16MiB、24h。空目录因此得到 `control.db`、`lance/`、`staging/`。再次打开沿用已有快照，不删除已发布的版本。
4. 调用 `install_collector`，构造 `AbilityHost` 与上述 Gateway，对清单中的实例 `install` 再 `load`。不调用 `AppBuilder::bootstrap`。
5. 绑定三个环回监听，进入循环。

配置监听使用 `config::watch`，防抖 2 秒。新快照校验失败时保留当前快照。`strata.lance.uri`、`strata.control_store`、`strata.staging.dir` 属于重启项，热加载不改已经打开的数据目录。

进程收到中断或 `SIGTERM` 时经 Gateway 调用 `planner.shutdown`，释放控制库里的任务租约，然后退出。

## 循环

循环在组合根里，经 `ConsoleGateway` 调用：

| 间隔 | 调用 |
|---|---|
| 30 秒 | `planner.tick`，然后对返回的每个任务调用 `runner.run_task`。返回的跟进任务先 `flush`，再逐个执行 |
| `policy.commit.flush_interval`，缺省 30 秒 | `committer.flush` |
| 60 秒 | `health.evaluate` |

`committer.flush` 得到可见快照序号后，组合根经 `type_hotlist` 与 `strata_write` 完成热榜提交（见 `apply_hotlist`）。类型投影经 `type_article` / `type_hotlist` Ability 暴露。

`planner.tick`、`runner.run_task`、`flush_interval`、push 监听和健康快照都读 `applied`。`health.evaluate` 每 60 秒调用一次。传给计划快照的间隔是 `policy.state.snapshot_interval`，缺省 15 分钟。进行中的任务把变更集的 `config_generation` 写成任务上的 `generation`。

`planner.tick` 的并发容量是固定值：`free` 4、`paid` 1、`render` 2、`push` 4。健康计算使用的产量下降比是 0.3，连续失败阈值是 5。配置里没有这两类键。

定位符是 `channel` 的信源不进入 `tick`。推送由 push 监听经 `push_ingress` Ability 入库。接受成功后，按该信源的管道生成变更集并刷写。管道为空时，正文写成 `raw` facet。单次推送的失败不停止监听。

`runner.run_task` 在进程内经 `ctx.call` 调用访问器 Ability 与 `adapters` 内置注册表。访问器拿到的策略是 `overlay_system_policy(applied.system, 信源 policy)`。出站策略和付费 `unknown_receipt` 按同一次 `applied` 组装。同一次访问的 HTTP 传输超时是该任务的截止时间，取访问器 `limits.timeout`，缺省 30 秒。租约时长是同一个 `limits.timeout` 的 1.5 倍。pipeline 为空时，每个 `document` 部件写成 `raw` facet。pipeline 非空时采用 `adapters::run` 返回的操作。`needs` 非空且该信源 `detail.maxFetches` 大于 0 的 `follows` 入队。入队前跳过 `struct:page` 已含全部所需字段的资源。跟进任务成功时只应用变更集并确认。失败时 `nack`。信源已不在快照中时 `nack`，消息是 `config`。

付费 endpoint 是 `http` 或 `https` 时经 `egress` Ability 发送。信源 `paid.budget.requests` 是一个 `window` 内允许的付费请求次数。Planner 在派出新的计划任务时按这个次数限流。

`MODALPRISM_MOLI` 是 moli 可执行文件路径，缺省在 `PATH` 中查找 `moli`。浏览器子请求的代理 URL 是本次进程绑定的正向代理。`docs/specs/builtin-accessors.md` 的未实测项仍然有效。

## 监听

正向代理是 `egress::serve_each`。push 是组合根自己的 HTTP 监听，只接受 `POST /<channel>`，经 `push_ingress` 处理。正文上限 8 MiB。

运维 HTTP 端口使用 `frame-port-http` 的路由，并去掉响应里的 `Access-Control-*` 头和只为跨源协商写入的 `Vary`。目录和 invoke 经 `HttpOpsGateway`，仅放行 Expose 中启用的实例，并校验 HTTP 协议登记。

## Ability 布局

共享 Ability 位于 `abilitys/<分类>/<name>/`（`ability.yml`、`build.rs`、`src/logic/`、`src/generated/`）。应用自有 Ability 位于 `apps/modalprism-collector/ability/<name>/`。实例 id 与 ability id 相同。访问器实例 id 把 `builtin:<name>` 里的 `-` 换成 `_`。

`stateful: true` 的存储与网关类：`strata_read`、`strata_write`、`strata_queue`、`committer`、`runner`、`config`、`egress`、`paid_gateway`。`planner`、`health`、`config_inspect` 与各 `accessor_*` 按 manifest 声明。

方法参数是 JSON 对象。无法反序列化时消息是 `spec 无效`。经 Gateway 返回时，`spec 无效`、`reserve`、`gateway`、`session`、`存储位置未规定`、`监听地址不是环回地址` 的 ErrorKind 是 `InvalidInput`。其余失败的 ErrorKind 是 `Failed`。

完整方法面见各 `ability.yml` 与 `docs/specs/scheduling.md`。

## 未闭合

- 端口访问控制（TLS、鉴权，以及环回地址以外的绑定）
- embedding 正文形态和维度
- `Body::External` 的内容哈希
- 是否改用 CUE
- 多实例租约与 PostgreSQL
- Web Bot Auth 签名与会话轮换
- 对象存储
- 合并付费响应的归属
- 浏览器内核的未实测项，见 `docs/specs/builtin-accessors.md`
