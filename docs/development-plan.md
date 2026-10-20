# ModalPrism 开发计划

八个阶段的实现与规格都在仓库中。各阶段的行为以 `docs/specs/` 为准。未闭合的部署与协议问题列在文末，不在代码里另作规定。

## 阶段 1　领域契约

`crates/strata-model` 定义资源标识、命名空间、facet 引用、变更集、查询、原始信封、访问任务和错误类别。该 crate 不依赖 LEAF。资源 ID、facet 引用和区间引用可以解析并往返。变更集与查询按设计中的字段名序列化。

## 阶段 2　Strata

`crates/strata-store` 与 `crates/strata-client` 提供本地目录上的 SQLite 控制库、Lance 表、暂存区和 Committer，以及进程内的 query、apply、queue。空目录可以创建控制库、Lance 目录和暂存区。一个变更集的写入在同一个快照中全部可见，或都不可见。`accepted` 之后的重放不产生重复 facet 版本。身份键与 blob 只在不存在时插入。游标、数据与回执在同一个变更集中提交。业务调用不接触 Lance API。Blob v2 列与 `merge_insert` 使用锁定的 Lance 版本。查询范围、恢复和未接收的正文见 `docs/specs/strata-storage.md`。PostgreSQL 控制库与多实例租约仍未规定。

## 阶段 3　配置

`crates/config` 把 YAML 目录合并成不可变快照，带四套内置预设、JSON Schema 校验、差异、字段溯源、文件监听与代际切换。未知字段、缺失引用、锁定冲突和低于作用域的键使整份新快照失败，旧快照保持。预设只提供 `policy` 基线。系统作用域的键只随全局选中的预设生效。需要重启的项在热加载时不改已经打开的数据目录。规则见 `docs/specs/config.md`。配置加载器接受 JSON Schema，不接受 CUE。

## 阶段 4　访问器与适配器契约

`crates/accessors` 与 `crates/adapters` 定义 Accessor、SearchAdapter、ContentAdapter、Detector、路由 DAG，以及识别、Feed、列表、正文、链接这些纯计算适配器。适配器没有写入口。声明为确定性的适配器在相同输入和配置下得到相同输出。路由依次使用部件角色与声明类型、识别结果、`probe`。同一 raw facet 只识别一次。规则见 `docs/specs/access-adapters.md`。

## 阶段 5　调度与治理

`crates/egress`、`crates/paid` 与 `crates/planner` 提供出站网关、付费网关、Cadence、Health、Planner 和 Runner。对外请求只经过出站端口。预算错误不计入信源失败。回执与数据处于同一个变更集。租约到期前同一 `(source, locator)` 不重复派发。原任务在 `retry_at` 已到或租约到期后，由下一次 `tick` 重新领取。进程停止时释放租约。调度使用 accepted 游标，对外读取使用 visible 游标。规则见 `docs/specs/scheduling.md`。

## 阶段 6　内置访问器

`crates/accessors-builtin` 实现 `http`、`rss`、`api`、`paid-api`、`search`、`push`、`file`、`browser`。访问器只产出原始信封。部件字节写入暂存区，信封只携带引用。`push` 使用组合根自己的 HTTP 监听。`browser` 的网络子请求指向出站网关的正向代理。`capture.network=all-bodies` 的覆盖度未实测，不作为已完成项。规则与未实测项见 `docs/specs/builtin-accessors.md`。

## 阶段 7　装配与一键启动

`ability/` 下的采集 Ability 与 `apps/modalprism-collector` 组成采集进程。命令是 `modalprism-collector`。空目录启动后得到 `control.db`、`lance/`、`staging/`。再次启动沿用已发布的快照。配置校验失败时不替换当前快照。进程退出时释放任务租约。每次访问按已生效快照组装出站策略和付费策略。运维 HTTP 去掉响应里的 `Access-Control-*` 头。组合根直接依赖 `frame-core` 与 `ability-core` 链接 Ability，并依赖 `frame-port-http` 提供运维 HTTP。端口上没有 TLS 和鉴权协议。监听地址以外的访问控制未规定。链接与循环见 `docs/specs/leaf-integration.md` 和 `docs/specs/assembly.md`。

## 阶段 8　抽象类型与只读表面

`crates/types` 实现 `Article` 与 `HotList`。`project` 只读。超过内联上限的文章正文写入 `readable`。热榜按 `createdAt` 的时间点排序。`on_commit` 读取已生效配置快照，返回变更集或 JSON `null`，类型实现不调用 `apply`。采集进程在快照可见之后调用 `HotList::on_commit`，并把变更集交给 Strata。`ability/type_article` 与 `ability/type_hotlist` 把这两个类型挂到 Ability 上。`apps/modalprism-agent` 是只读进程，命令是 `modalprism-agent`。它只安装 `strata_read`、`health`、`config_inspect`、`type_article`、`type_hotlist`，不构造 Planner、Runner、出站网关、付费网关和访问器。打开 Strata 时采用配置里的 blob 阈值和暂存 ttl。工具面是 MCP 的标准输入输出，工具名是 `instance.method`，只读方法的 `readOnlyHint` 为 true。`AccessKind::Tools` 没有 Port。该 LEAF 提交没有 Skill Port，进程不放置 `info.yml`，不运行 codegen。规则见 `docs/specs/types.md`。

## 未闭合

- 端口访问控制（TLS、鉴权，以及环回地址以外的绑定）
- embedding 正文形态和维度
- `Body::External` 的内容哈希
- 是否改用 CUE
- 多实例租约与 PostgreSQL
- Web Bot Auth 签名与会话轮换
- 对象存储
- 合并付费响应的归属
- `paid.budget` 的计量单位
- 浏览器内核的未实测项，见 `docs/specs/builtin-accessors.md`
- 结构化 facet 的物化列
- Skill 入口（该 LEAF 提交没有对应 Port）
