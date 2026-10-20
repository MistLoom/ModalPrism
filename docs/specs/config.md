# 配置快照

本文固定阶段 3 的配置加载行为。设计第 10 节是配置模型，这里只写实现必须遵守、且设计没有展开的规则。

## 目录

`validate` 读取一个配置目录：

- `system.yaml`：L0，必填。`kind` 为 `system`。缺省 `id` 记为 `system`。
- `defaults/`：L1 片段。可以含 `common.yaml`、`accessor.yaml`、`adapter.yaml`、`source.yaml`。片段不含 `id` 与 `kind`。
- `presets/`：预设。仓库带有 `minimal`、`standard`、`archive`、`debug` 四份，目录中的文件可以替换它们。
- `profiles/`、`accessors/`、`adapters/`、`sources/`、`types/`：对应 kind 的模块。子目录只用于分组。
- `schemas/`：JSON Schema 文件，不作为模块。
- `overlays/{env}/`：仅当调用方传入环境名时读取。相对路径与主文件相同的 overlay 深度合并到该主文件之后，例如 `overlays/prod/system.yaml`。没有主文件的 overlay 视为该层的新文件。

扩展名为 `.yaml` 或 `.yml`。其他文件不参与模块加载。已知目录与 `system.yaml` 以外的 YAML 文件使本次加载失败。每个文件只含一个文档。

## 键名

策略键与设计 10.4.1 的路径一致，并加上信源上的 `robots.exempt_as`、`robots.robots_reason`。`policy.preset` 与 `policy.locks` 写在 `policy` 对象内。

结构字段与设计 10.8 的示例一致：`schedule.expectedPeriod`、`schedule.batchKey`、`admission.allowUrlPrefixes`、`admission.denyUrlPrefixes`、`admission.publishedAfter`、`admission.initialBackfill`、`initialBackfill.maxAge`。`locator` 使用 `url`、`query`、`endpoint`、`channel` 之一。`locator.url` 以 `http://` 或 `https://` 开头。

`access.render` 缺省为 `auto`。`enabled` 缺省为 `true`。

## 预设

`policy.preset` 写成预设名 `standard`，或完整 id `preset/standard`。`standard` 的策略以设计 10.8 的示例为正文，并写入对照表中的 `robots.mode: obey-with-exemptions`。`archive` 与 `debug` 的 `extends` 为 `preset/standard`，只写对照表里与 `standard` 不同的键。`minimal` 独立写全量；对照表未单列的键与 `standard` 相同。`versions.facet_keep` 用对象表示：`minimal` 与表中的「只保留最新」为 `{ default: 1 }`，`archive` 的「全部」为 `{ default: all }`。

预设文件只含标识字段、`extends` 与 `policy`。`policy` 内不含 `preset` 与 `locks`。`system.yaml` 的 `policy.preset` 必填。

选择顺序：环境 overlay、模块实例、`extends` 中最后一个写出 `policy.preset` 的 profile、`system.yaml`。选中的预设先按自己的 `extends` 在 L2 内合并，再进入该模块的合并链。

## 合并

顺序：L1 默认、`system.yaml` 中该模块 kind 可接受的非系统作用域键、L2 预设、profile（从左到右）、实例、该实例的 overlay。

- 标量以后层为准。
- 映射按键递归合并。
- 列表整体替换。后层若是仅含 `$append`、`$prepend` 或 `$remove` 的对象，则追加、前插或按值删除。
- `null` 删除该键。合并结束后缺必填字段则本次加载失败。
- 同一 `id` 出现两次则失败。`id` 形如 `preset/standard`，前缀与 `kind`、所在目录一致。
- `id`、`kind`、`name` 写在实例文件上。
- profile 先展开自己的 `extends`。环使本次加载失败。

`${env:NAME}` 读取调用方提供的环境映射。`LoadOptions::from_process` 在每次 `open` 与 `reload` 时复制当时的进程环境。`enabled: false` 的模块保留 `${env:NAME}` 字面量，变量缺失不使本次加载失败，变量已设置时也不写入快照。启用的模块在变量缺失时整次加载失败。`${secret:path}` 读取调用方提供的密钥映射，只用来确认密钥存在。快照、`explain` 和 `diff` 保留 `${secret:path}`，不写入密钥明文。`${ref:id#a.b}` 引用另一份已合并配置的值，整个字符串为该占位符时保留目标类型。引用环、启用模块缺失的环境变量、缺失的密钥、缺失的引用都使本次加载失败。

## 作用域

系统作用域的键可以写在 `system.yaml`、其 overlay 和预设中。写在 profile 或模块实例中则本次加载失败。预设里的系统作用域键只进入全局选中预设所生成的系统策略；模块另选预设时，这些键不进入该模块。

信源作用域的键可以写在预设、profile 和信源中。写在访问器或适配器实例中则失败。profile 中的信源作用域键只合并进信源。访问器或适配器 `extends` 该 profile 时，这些键不进入该模块。`defaults/common.yaml` 使用与 profile 相同的作用域规则；`defaults/accessor.yaml`、`defaults/adapter.yaml`、`defaults/source.yaml` 使用对应 kind 的规则。

访问器作用域的键按同样规则只进入访问器。适配器与信源共用的键只进入这两类。`policy.locks` 只出现在 `system.yaml` 及其 overlay。

## 锁定

`policy.locks` 列出策略叶子路径。比较基准是 `system.yaml` 与其 overlay 合并后的值，不含预设。基准里没有该路径则失败。

系统作用域键比较全局有效值（含全局预设）。其他键比较每个会接受该键的模块的有效值。有效值与基准不同则本次加载失败。

## 校验

YAML 使用 1.2 核心模式。显式标签、重复键、一个文件中的多个文档使本次加载失败。

策略对象用 JSON Schema Draft 2020-12 校验，`additionalProperties: false`。未知顶层字段在进入合并前拒绝。

语义规则：

- `access.accessor` 指向已有访问器。`pipeline` 的 `run` 与 `run_if_accepts` 指向已有适配器。
- `role: search` 的 `produces` 各项为 `discovery`。`role: content` 的各项为带 `kind` 的对象。
- `robots.mode` 为 `ignore` 时信源必须有 `robots_reason`，且不写 `exempt_as`。写出 `exempt_as` 时，mode 必须是 `obey-with-exemptions`，类别属于系统策略的 `robots.exemptions`，并且必须有 `robots_reason`。`obey` 不写 `exempt_as`。
- `implementation` 以 `builtin:paid` 开头时，`paid.service` 为非空字符串。使用 `accessor/paid-api` 的信源，以及写出 `paid` 的信源，必须有 `paid.budget`：`requests` 为不小于 1 的整数，`window` 为时长。`requests` 是该窗口内允许的付费请求次数。
- `spec` 的键不与基础字段或 `policy` 的顶层键同名。`spec` 中的 `$schema` 指向 `schemas/` 下的文件时，用该 schema 校验 `spec` 的其余内容。
- `adaptive: true` 时 `floor`、`ceiling`、`expectedPeriod` 必填；否则 `interval` 必填。

一次加载中的任一错误都丢弃这份新结果。

## 快照、差异与溯源

成功的结果是不可变快照。`open` 的代际号为 1。`reload` 成功后代际号加 1，并替换当前快照。失败时保留原快照。

`diff` 比较两份快照的系统对象与各模块对象。`explain` 给出每个已写入字段的值、文件、行号；值来自预设时带上预设 id。模块选中的预设与写出该值的预设不同时，标明经由哪个预设。系统作用域键在模块的 explain 中显示全局有效值。`overlay_system_policy` 把这些键覆盖到模块策略上，访问器使用这份有效策略。

库接口是 `validate`、`diff`、`explain`、`open`、`reload`。本阶段没有命令行程序。

## 热加载

`watch` 监听配置目录，安静 `DEFAULT_DEBOUNCE`（2 秒）后调用 `reload`。`watch_hangup` 在 SIGHUP 时调用 `reload`。

以下路径在 `open` 时进入 `applied`，之后的 `reload` 不改变 `applied` 中的这些值：

- `strata.lance.uri`
- `strata.lance.storage_options`
- `strata.lance.new_table_data_storage_version`
- `strata.lance.new_table_enable_v2_manifest_paths`
- `strata.control_store`
- `strata.staging.dir`
- 已有访问器的 `implementation`
- 已有访问器的 `spec.moli.version`

`open` 之后新增的访问器不进入 `applied`。从目录中删除的访问器仍留在 `applied`，内容保持 `open` 时的配置。其余字段随新快照生效。`snapshot` 始终是最近一次成功加载的文件结果。`pending_restart` 列出 `snapshot` 与 `applied` 在上述路径上的差异。

信源、适配器及其 spec、节奏、准入、限速和信源或适配器作用域的策略随 `reload` 进入 `applied`。

## 未闭合

- 设计第 13 节第 9 项：是否改用 CUE。本阶段只使用 JSON Schema，不读取 CUE。
- 密钥的具体后端。本阶段只读取调用方传入的映射。
