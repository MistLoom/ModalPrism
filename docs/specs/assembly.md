# 装配与一键启动

本文固定采集进程的启动、Ability 链接和循环。存储见 `docs/specs/strata-storage.md`，配置见 `docs/specs/config.md`，调度见 `docs/specs/scheduling.md`，LEAF 链接见 `docs/specs/leaf-integration.md`。

## 组合根资源（`crates/host`）

采集进程在 `AbilityHost` 之外持有 `HostResources`：已打开的数据目录、配置快照、Strata、Planner、出站与付费网关、push 访问器等。组合根与 Ability 工厂通过 `Arc<HostResources>` 注入共享状态；进程内调度循环与 `host::api::invoke(host, ability, method, args)` 经 Gateway 调用各 Ability 方法。

`HostResources::call("ability.method", args)` 是面向运维适配层的稳定字符串契约（例如 `config.generation`、`planner.trigger`），实现上路由到同一套 Ability 方法。它不属于 LEAF 的跨 Ability `ctx.call` 图，也不向 `/v1/abilities` 暴露。

Web 控制台（`crates/console`、`crates/console-api`）不是 Ability，不进入 `ability/` 链接表。HTTP 处理在 ops 端口挂载 `/console/`，业务逻辑通过上述 `call` 契约读状态与触发操作，见 `docs/console.md`。

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
4. 构造 `AbilityHost`，对下列实例 `install` 再 `load`。不调用 `AppBuilder::bootstrap`。
5. 绑定三个环回监听，进入循环。

配置监听使用 `config::watch`，防抖 2 秒。新快照校验失败时保留当前快照。`strata.lance.uri`、`strata.control_store`、`strata.staging.dir` 属于重启项，热加载不改已经打开的数据目录。

进程收到中断或 `SIGTERM` 时调用 `Planner::shutdown`，释放控制库里的任务租约，然后退出。

## 循环

循环在组合根里，经 Gateway 调用：

| 间隔 | 调用 |
|---|---|
| 30 秒 | `planner.tick`，然后对返回的每个任务调用 `runner.run_task`。返回的跟进任务先 `flush`，再逐个执行 |
| `policy.commit.flush_interval`，缺省 30 秒 | `committer.flush` |
| 60 秒 | `health.evaluate` |

`committer.flush` 得到可见快照序号后，采集进程调用 `HotList::on_commit`，传入已生效配置快照，把返回的变更集 `apply`，再 `flush`。类型实现不调用 `apply`。

`planner.tick`、`runner.run_task`、`flush_interval`、push 监听和健康快照都读 `applied`。`health.evaluate` 每 60 秒调用一次。传给 `Planner::snapshot` 的间隔是 `policy.state.snapshot_interval`，缺省 15 分钟。进行中的任务把变更集的 `config_generation` 写成任务上的 `generation`。

`planner.tick` 的并发容量是固定值：`free` 4、`paid` 1、`render` 2、`push` 4。健康计算使用的产量下降比是 0.3，连续失败阈值是 5。配置里没有这两类键。

定位符是 `channel` 的信源不进入 `tick`。推送由 push 监听调用 `PushAccessor::ingest`。接受成功后，按该信源的管道生成变更集，`apply` 并 `flush`。管道为空时，正文写成 `raw` facet。单次推送的失败不停止监听。

`runner.run_task` 在进程内调用访问器和 `adapters` 的内置注册表。访问器拿到的策略是 `overlay_system_policy(applied.system, 信源 policy)`。出站策略和付费 `unknown_receipt` 按同一次 `applied` 组装：robots 的模式、crawl delay、豁免类别来自信源，豁免名单、缓存时间、不可达策略、私网和身份来自系统，`unknown_receipt` 来自访问器。同一次访问的 HTTP 传输超时是该任务的截止时间，取访问器 `limits.timeout`，缺省 30 秒。进行中的请求在截止时间被中断。时限长于 30 秒时，请求保持到该时限。租约时长是同一个 `limits.timeout` 的 1.5 倍。pipeline 为空时，每个 `document` 部件写成 `raw` facet，正文是 `Body::Staged`。pipeline 非空时采用 `adapters::run` 返回的操作。`needs` 非空且该信源 `detail.maxFetches` 大于 0 的 `follows` 入队，访问器与 spec 取 `FollowRequest`（缺省 `accessor/http` 与空 spec），原因是 `Follow`。入队前跳过 `struct:page` 已含全部所需字段的资源，留下的数量不超过 `maxFetches`。`runner.run_task` 返回这些任务。采集循环在执行它们之前先 `flush`，使列表写入的身份进入可见快照。跟进任务成功时只应用变更集并确认，不改写信源下次时间和游标。失败时退回该任务，不改写信源下次时间。到期的跟进任务在后续扫描里按免费队列剩余容量领取。信源已不在快照中时 `nack`，消息是 `config`。

完成回执的 `logicalKey` 是付费网关为该 `receipt_id` 保存的逻辑键，`service` 是该次回执上的服务名。

访问器请求了会话名时，类别 `config`，消息 `session`。不轮换会话。

付费 endpoint 是 `http` 或 `https` 时经出站网关发送。其他 endpoint 的类别是 `permanent`，消息是 `scheme`。一次调用一个信封。信源 `paid.budget.requests` 是一个 `window` 内允许的付费请求次数。Planner 在派出新的计划任务时按这个次数限流。次数用尽时本窗口跳过该信源，`planner.next` 的 `skipReason` 为 `budget`，`budgetUsed` 是本窗口已派出的次数。窗口滚动后计数归零并恢复派出。

`MODALPRISM_MOLI` 是 moli 可执行文件路径，缺省在 `PATH` 中查找 `moli`。浏览器子请求的代理 URL 是本次进程绑定的正向代理。`docs/specs/builtin-accessors.md` 的未实测项仍然有效。

## 监听

正向代理是 `egress::serve_each`。每接受一个连接，用当时的 `applied` 系统策略组装出站策略。push 是组合根自己的 HTTP 监听，只接受 `POST /<channel>`，把请求头和正文交给 `ingest`。策略同样经过 `overlay_system_policy`。正文上限 8 MiB。它不使用 HTTP Port 的 invoke 路由。

运维 HTTP 端口使用 `frame-port-http` 的路由，并去掉响应里的 `Access-Control-*` 头和只为跨源协商写入的 `Vary`。目录和调用只放行 `strata_read`、`planner`、`health`。Host 不设置单一的 `access_gate`。进程内调用直接使用 `AbilityHost`。

LEAF 的 CLI Port 会解析自己的子命令。本进程的参数由上面的命令行处理，不打开该 Port。

## Ability

每个 Ability 一个 crate，声明在 `ability.yml`。Host 使用 Rust 里的 Manifest。不运行 LEAF codegen，不放置协议 `info.yml`。

`stateful: true`：`strata_read`、`strata_write`、`strata_queue`、`committer`、`planner`、`runner`、`health`、`config`、`egress`、`paid_gateway`。

`config_inspect` 与 `accessor_*` 不是 stateful。

实例 id 与 ability id 相同。访问器实例 id 把 `builtin:<name>` 里的 `-` 换成 `_`，例如 `builtin:paid-api` 对应 `accessor_paid_api`。

方法参数是 JSON 对象。无法反序列化时消息是 `spec 无效`。经 Gateway 返回时，`spec 无效`、`reserve`、`gateway`、`session`、`存储位置未规定`、`监听地址不是环回地址` 的 ErrorKind 是 `InvalidInput`。其余失败的 ErrorKind 是 `Failed`。调度用的错误类别只出现在 `runner.run_task` 内部。

| 方法 | 参数 |
|---|---|
| `strata_read.kinds` | `rid`，可选 `includeVariants` |
| `strata_read.has` | `rid`、`kind`，可选 `variant` |
| `strata_read.get` | `facet`（FacetRef 字符串），可选 `lazy`。返回版本、哈希、媒体类型、大小、文本、结构、`PartRef`。不返回正文 |
| `strata_read.slice` | `span`（SpanRef 字符串）。正文是文本时返回字符串。二进制区间的消息是 `gateway` |
| `strata_read.resolve` | `identity` |
| `strata_read.history` | `facet` |
| `strata_read.lineage` | `facet`，可选 `direction`（`up` 或 `down`，缺省 `up`）、`depth`（缺省 1） |
| `strata_read.related` | `rid`，可选 `rel`、`direction`（缺省 `out`） |
| `strata_read.query` | `query` |
| `strata_read.changes` | `after`，可选 `limit` |
| `strata_write.apply` | `changeset` |
| `strata_queue.enqueue` | `tasks` |
| `strata_queue.lease` | `queue`、`count`、`leaseMs` |
| `strata_queue.extend` | `taskId`、`leaseMs` |
| `strata_queue.nack` | `taskId`、`retryAt`、`error` |
| `committer.flush` | 无 |
| `committer.maintain` | 无。调用 `recover` 并返回数据目录。存储接口没有压缩方法，本方法不压缩 |
| `committer.recover` | 无 |
| `committer.status` | 无。返回 `control.db`、`lance`、`staging` 的路径 |
| `planner.tick` | 无 |
| `planner.trigger` | `source`。把该信源的 `next_at` 写成当前时间，再执行一次 `tick`。其他已到期信源仍会派出 |
| `planner.next` | `source` |
| `runner.run_task` | `task` |
| `health.evaluate` | 无。调用 `Planner::snapshot`，有操作时 `apply` 并 `commit_snapshot` |
| `health.status` | `source`。键是 `source/<source>` |
| `config.load` | 无。失败时保留当前快照 |
| `config.generation` | 无 |
| `config_inspect.validate` | 无。重新读取目录，不替换当前快照 |
| `config_inspect.diff` | 无。当前快照与重新读取的快照比较。重新读取失败则返回错误，不替换当前快照 |
| `config_inspect.explain` | `module` |
| `egress.check` | `url`。返回解析到的地址是否被私网规则挡住 |
| `egress.status` | 无。返回本次正向代理 URL |
| `paid_gateway.reserve` | 无。占位和发送在 `PaidGateway::call` 里一次完成。本方法类别 `config`，消息 `reserve`，不扣费 |
| `paid_gateway.complete` | `receiptId` |
| `paid_gateway.status` | `receiptId` |
| `accessor_*.validate` | `spec` |
| `accessor_*.access` | `taskId`、`sourceId`、`locator`、`spec`，可选 `cursor`、`policy` |

内置适配器由 `runner.run_task` 调用 `adapters::Registry::builtins`。适配器不另建 Ability 实例。

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
