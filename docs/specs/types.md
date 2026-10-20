# 抽象类型与只读表面

本文固定 `Article`、`HotList` 和 `modalprism-agent`。存储见 `docs/specs/strata-storage.md`，配置见 `docs/specs/config.md`，装配见 `docs/specs/assembly.md`，LEAF 链接见 `docs/specs/leaf-integration.md`。

## 契约

抽象类型实现 `name`、`version`、`schema`、`selector`、`project`、`on_commit`。

`project` 只读。资源不满足 `selector` 时返回 JSON `null`。

`on_commit` 可以返回一个变更集，也可以返回 JSON `null`。类型实现不调用 `apply`。类型数据的写入只出现在这个变更集里。

`schema` 是类型自己的 JSON Schema 文档，放在配置目录的 `schemas/`。配置加载器不接受 CUE。是否改用 CUE 仍未规定。

结构化 facet 仍以 JSON 文本存放。物化列不在本文范围。

## Article

实例 id 是 `type_article`。配置模块 id 是 `type/article`。

`selector` 要求 facet `struct:article`。

`project` 的对象：

| 字段 | 来源 |
|---|---|
| `rid` | 资源 id |
| `struct` | `struct:article` 的结构值 |
| `readable` | `md:readable` 的文本。超过内联上限、只留在 blob 中的正文同样读出。没有则为 `null` |
| `cover` | `image:cover` 的 `sha256` 与 `size`；没有则为 `null` |

不返回正文字节。`on_commit` 返回 `null`。

## HotList

实例 id 是 `type_hotlist`。配置模块 id 是 `type/hotlist`。

一期快照是一个资源，带 facet `struct:hotlist.snapshot`。`part_of` 从快照指向榜单资源。`entry` 从快照指向条目资源，`attrs.rank` 是名次。

`project` 的对象：

| 字段 | 来源 |
|---|---|
| `rid` | 快照资源 id |
| `list` | `part_of` 的目标；没有则为 `null` |
| `snapshot` | `struct:hotlist.snapshot` 的结构值 |
| `entries` | `entry` 关系，每项含 `dst`、`rank`、`attrs` |
| `delta` | `struct:hotlist.delta` 的结构值；没有则为 `null` |

`on_commit` 按 `part_of` 目标分组。没有 `part_of` 的快照自成一组。组内按资源 `createdAt` 的时间点排序，同一时间点再按资源 id。

组内至少有两期时，用最后两期的 `entry` 比较。结果写入最后一期的 `struct:hotlist.delta`，正文是结构值，媒体类型是 `application/json`。

| 字段 | 内容 |
|---|---|
| `rankDelta` | 数组。每项含 `dst`、`from`、`to`、`delta`。`from` 是上一期名次，`to` 是这一期名次，`delta` 是 `from - to`。名次相同的条目不进入数组 |
| `newEntries` | 这一期出现、上一期没有的条目资源 id |

两个数组都空时不写该 facet。已有 `struct:hotlist.delta` 与新结构值相同时，变更集不含这条 `putFacet`。

变更集的 `producer.module` 是 `type_hotlist`，`producer.version` 是 `1`，`config_generation` 取已生效配置快照的代际号。`lineage.params_hash` 是 `hotlist.delta`。`idempotency_key` 是这些操作 JSON 的内容哈希。没有任何操作时返回 `null`。

采集进程在 `committer.flush` 得到可见快照之后调用 `HotList::on_commit`，传入已生效配置快照，把返回的变更集 `apply`，再 `flush`。`on_commit` 读取该已生效快照。类型实现不调用 `apply`。

`spec.keepSnapshots` 是正整数时，组内更早的快照用 `Tombstone` 收进同一个变更集，理由是 `keepSnapshots`。配置 schema 把 `spec.state` 保留为策略字段名，因此份数写在 `spec.keepSnapshots`，不写在 `spec.state`。缺省不裁剪。

## 只读进程

可执行文件是 `modalprism-agent`。

```text
modalprism-agent [--data DIR] [--config DIR]
```

缺省 `--data` 是 `var`，`--config` 是 `config`。未知参数的消息是 `未知参数: <参数>`。缺少取值的消息是 `缺少参数值: <参数>`。

打开数据目录的规则与采集进程相同：`strata.lance.uri` 非空、控制库后端为 `postgres`，或控制库、暂存路径不是该数据目录时，消息是 `存储位置未规定`，不创建数据目录。`strata.blob.inline_size_threshold`、`strata.blob.dedicated_size_threshold` 和 `strata.staging.ttl` 取这次打开时的快照，缺省分别是 64KiB、16MiB、24h。空目录仍得到 `control.db`、`lance/`、`staging/`。

进程只 `install` 并 `load` 这些实例：`strata_read`、`health`、`config_inspect`、`type_article`、`type_hotlist`。

不加载 `strata_write`、`strata_queue`、`committer`、`planner`、`runner`、`egress`、`paid_gateway`、`config` 和访问器。进程不构造 Planner、Runner、出站网关、付费网关和访问器。

`health` 只有 `status`。它读取已发布状态，scope 是 `health`，键是 `source/<source>`。没有记录时 `status` 为 `ok`，`alert` 为 false。不调用 `evaluate`。

`config_inspect` 提供 `validate`、`diff`、`explain`。校验失败不替换已打开的快照。进程不加载 `config.load`。

`strata_read` 的方法与 `docs/specs/assembly.md` 相同。

类型方法：

| 方法 | 参数 |
|---|---|
| `project` | `rid` |
| `on_commit` | 无。返回变更集或 `null` |

参数无法反序列化时消息是 `spec 无效`。`spec 无效` 与 `存储位置未规定` 的 ErrorKind 是 `InvalidInput`。其余失败的 ErrorKind 是 `Failed`。

## 协议表面

本进程打开 LEAF 的 MCP Port。该 Port 使用标准输入输出，把 Gateway 的目录列成工具。端口实现没有 TLS，也没有鉴权协议。

`AccessKind::Tools` 在该 LEAF 提交里没有 Port，只作为运行时门控。上面五个实例登记 `fun`、`mcp`、`tools`。交给 MCP Port 的目录把每个方法展开成一个实例，实例 id 是 `instance.method`，工具名与之相同。只读方法的 `readOnlyHint` 为 true。`strata_read`、`health`、`config_inspect`、`type_article`、`type_hotlist` 的已登记方法都是只读，返回查询结果或变更集，不调用 `apply`。进程内调用仍使用原来的实例 id 和方法名。

该提交没有 Skill Port。本进程不放置 `info.yml`，不运行 codegen，不另开 Skill 监听。

不打开 HTTP Port，也不打开 CLI Port。

## 未闭合

与 `docs/specs/assembly.md` 相同，另加：

- 结构化 facet 的物化列
- Skill 入口（该 LEAF 提交没有对应 Port）
