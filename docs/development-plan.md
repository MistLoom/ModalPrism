# ModalPrism 开发计划

本文描述当前的实现顺序。阶段按依赖关系排列：后一阶段使用前一阶段已经固定的契约。某阶段的技术要求若尚未闭合，写在 `docs/specs/`，不在本文件里补全实现。

## 阶段 1　领域契约

**目标**　固定各阶段共用的数据契约，使存储、配置、访问和调度使用同一套标识与报文。

**范围**　Cargo workspace 与 `crates/strata-model`。包含资源标识、命名空间、facet 引用、变更集、查询、原始信封、访问任务和错误类别。不包含存储引擎、配置加载、网络访问和 LEAF 装配。

**交付**　workspace 清单、`strata-model`、对应单元测试。

**退出标准**

- `cargo build` 与 `cargo test` 通过。
- 资源 ID、facet 引用和区间引用可以解析并往返。
- 变更集与查询按设计中的字段名序列化。
- 本 crate 不依赖 LEAF。

## 阶段 2　Strata

**目标**　资源系统成为唯一数据面：只追加、单写者、按快照可见。

**范围**　`crates/strata-store` 与 `crates/strata-client`。SQLite 控制存储、Lance 表、暂存区、Committer，以及进程内的 query、apply、queue。PostgreSQL 作为控制存储的另一种后端，接口在本阶段定下；多实例租约与共享暂存区仍受设计第 13 节的部署约束。查询范围、恢复和未接收的正文见 `docs/specs/strata-storage.md`。

**交付**　本地目录上的初始化路径；`apply` 的 accepted 结果；刷写后的快照读取。

**退出标准**

- 控制库、Lance 目录和暂存区可以由空目录创建。
- 一个变更集的写入在同一个快照中全部可见，或都不可见。
- `accepted` 之后的崩溃不丢失该变更集；重放不产生重复 facet 版本。
- 身份键与 blob 只在不存在时插入。
- 游标、数据与回执在同一个变更集中提交；失败不推进游标。
- 业务调用不接触 Lance API。
- `blobs` 上 Blob v2 列与 `merge_insert` 的写入行为须在本阶段用锁定的 Lance 版本实测。未完成实测时，该路径不算结束。

## 阶段 3　配置

**目标**　全部信源与系统配置来自 YAML，一次加载得到一份不可变快照。

**范围**　`crates/config`。层级合并、预设、锁定、策略作用域、JSON Schema 校验，以及 explain 所需的字段来源。文件监听、防抖与代际切换。规则见 `docs/specs/config.md`。

**交付**　四套内置预设；校验、差异和溯源的库接口。

**退出标准**

- 未知字段、缺失引用、锁定冲突、低于作用域的键，使整份新快照失败，旧快照保持。
- 预设只提供 `policy` 基线；profile 与实例中的显式键覆盖预设。
- 系统作用域的键只随全局选中的预设生效。
- 需要重启的项在热加载时不生效。

## 阶段 4　访问器与适配器契约

**目标**　访问产出原始信封，适配器只返回结构化输出，写入一律变成变更集。

**范围**　`crates/accessors` 与 `crates/adapters` 中的契约、路由和纯计算适配器（识别、Feed、列表、正文、链接）。出站与付费以端口被调用，限速、熔断和预算的实现不在本阶段。规则见 `docs/specs/access-adapters.md`。

**交付**　Accessor、SearchAdapter、ContentAdapter、Detector 契约；路由 DAG；针对夹具信封的适配器测试。

**退出标准**

- 适配器没有写入口。
- 声明为确定性的适配器，在相同输入和配置下得到相同输出。
- 路由依次使用部件角色与声明类型、识别结果、`probe`。
- 同一 raw facet 只识别一次，结果供后续适配器复用。

## 阶段 5　调度与治理

**目标**　任务派发、节奏、出站和付费在控制存储上形成闭环。

**范围**　`crates/egress`、`crates/paid`、`crates/planner`。出站网关（库调用与正向代理）、付费网关、Cadence、Health、Planner、Runner。规则见 `docs/specs/scheduling.md`。

**交付**　按注册域的令牌桶、熔断、robots 口径、SSRF 检查、付费占位与回执、租约队列、按错误类别的退避。

**退出标准**

- 对外请求只经过出站端口。
- 预算错误不计入信源失败。
- 回执与数据处于同一个变更集。
- 租约到期前同一 `(source, locator)` 不重复派发；进程停止时释放租约。
- 调度使用 accepted 游标，对外读取使用 visible 游标。

## 阶段 6　内置访问器

**目标**　设计中的访问器接到已实现的出站与付费端口。

**范围**　`http`、`rss`、`api`、`paid-api`、`search`、`push`、`file`、`browser`。规则见 `docs/specs/builtin-accessors.md`。

**交付**　各访问器实现与协议级测试。

**退出标准**

- 访问器只产出原始信封，不解析业务字段。
- 部件字节写入暂存区，信封只携带引用。
- `push` 使用组合根自己的 HTTP 监听。
- `browser` 的网络子请求指向出站网关的正向代理。`capture.network=all-bodies` 的覆盖度未实测前，不作为本阶段已完成项。

## 阶段 7　装配与一键启动

**目标**　一条命令完成存储初始化并进入采集循环。

**范围**　`ability/` 下的 Ability crate，以及 `apps/modalprism-collector`。链接方式以 `docs/specs/leaf-integration.md` 为准。组合根持有 Planner、Runner、Committer、健康评估和配置监听的循环。

**交付**　采集进程的启动路径。

**退出标准**

- 空目录启动后，控制库、Lance 目录和暂存区可用。
- 再次启动不破坏已发布的快照。
- 配置校验失败时不替换当前快照。
- 进程退出时释放任务租约。
- 组合根直接依赖 `frame-core` 与 `ability-core` 链接 Ability。端口访问控制未规定前，不在端口上增加鉴权协议。

## 阶段 8　抽象类型与只读表面

**目标**　在 Strata 之上提供类型投影，以及面向 Agent 的只读入口。

**范围**　AbstractType 契约、`Article` 与 `HotList`，以及 `apps/modalprism-agent`。

**退出标准**

- 类型数据只通过变更集写入。
- 只读进程不加载写入、队列、Committer、Planner、Runner 和访问器。
