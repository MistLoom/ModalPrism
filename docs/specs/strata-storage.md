# Strata 存储要求

本文固定阶段 2 的存储行为。设计第 5 节是数据模型，这里只写实现必须遵守、且设计没有展开的规则。

## 目录与打开

`open` 从空目录创建：

- `{root}/control.db`：SQLite，WAL
- `{root}/lance/`：Lance 库
- `{root}/staging/`：内容寻址暂存

再次打开不删除已有表和快照。建表时连接参数 `new_table_data_storage_version` 为 `2.2`，`new_table_enable_v2_manifest_paths` 为 `true`。`lancedb` 0.40.0 打开 `remote` feature，未打开时该版本不能编译。

默认参数：内联文本 4 MiB，blob 内联阈值 64 KiB，独立文件阈值 16 MiB，暂存保留 24 小时。blob 阈值在建表时写入 `blobs.data`。已存在的表不改阈值。

调用入口是 `strata-client`。`strata-store` 不把 Lance 类型放到 `strata-client` 的公开签名里。

## 控制存储

`ControlStore` 是控制存储的操作界面。本阶段只连接 SQLite，不连接 PostgreSQL。出队在单个 SQLite 连接上串行执行。

控制库保存 outbox、任务、已接收的身份与状态头、暂存引用、刷写批次和建表时的表版本。`committer_lease` 表保留给多实例租约；本阶段不写租约。同一进程的写者由句柄互斥保证。两个句柄打开同一目录时，没有额外的文件锁。

已完成回执是 `state` 中 `scope=receipt` 的行。付费调用前的 pending 不在本阶段写入。

## 接收

`apply` 在一个 SQLite 事务里完成别名解析、状态版本检查、outbox、队列 `ack` 和暂存引用。失败则整笔回滚。

- 未给出 `ns` 的 `ensure` 使用 `data`。
- 身份键已存在时返回原来的 RID，不改写标签，不新建资源。墓碑身份同样返回原来的 RID。
- 别名只在当前变更集内有效，重复别名和未定义别名都拒绝。
- `expectVersion` 对比已接收版本，含尚未可见的写入。缺省不检查。`Some(0)` 表示尚无写入。成功后版本加一。
- 同一个幂等键返回首次结果，`duplicate=true`，不追加 outbox。
- `ack` 的任务必须存在且未完成。确认与 outbox 同一事务。
- 暂存引用在接收时与文件的长度、sha256 核对。
- `commit_id` 是不透明的 ULID 文本。

`created_at` 在接收时确定，重放使用同一值。facet、blob 和快照的时间在刷写时确定。

### 正文

| 正文 | 接收的 kind |
|---|---|
| `text` | `raw` `html` `xml` `md` `text` `json` `yaml` 以及 `Other` |
| `struct` | `struct` `links` `regions` `json` `yaml` 以及 `Other` |
| `staged` | `raw` `html` `xml` `md` `text` `json` `yaml` `image` `audio` `video` `file` 以及 `Other` |

`embedding` 没有正文形态和维度，拒绝写入，不建 `embeddings_*`。`Body::External` 没有内容哈希，拒绝写入。其余不匹配的组合拒绝整笔 `apply`。

`struct` 的内容哈希是 `serde_json::to_vec` 的 sha256，`body_struct` 保存同一份 UTF-8。文本的内容哈希是 UTF-8 字节的 sha256。暂存正文的内容哈希是部件的 sha256。

标签的键和值不允许 `=` 或换行。`resource_marks.labels` 的元素是 `k=v`，按第一个 `=` 拆开。同一 `label` 操作里先 `unset` 再 `set`。`ensure` 只在新建且带标签时写初始标记。

## 刷写与可见性

库内不启动刷写循环。调用方执行 `flush` 和 `recover`。`WaitFor::Visible` 在 `apply` 的事务提交后执行一次 `flush`。

一批已接收变更集：

1. 把提交号和各表当前版本写入控制库的刷写批次。
2. 追加表用 `add`。`identities` 与 `blobs` 用 `merge_insert` 的 `when_not_matched_insert_all`，连接键分别是 `identity_key` 和 `sha256`。本阶段不建标量索引，也不调用 `optimize`。
3. 向 `snapshots` 追加一行。这一行记录追加之后的表版本，包含 `snapshots` 自身。版本号不是预期的下一次版本时，刷写失败。
4. 在同一个 SQLite 事务里把这批 outbox 标为可见，并删除刷写批次。

没有快照时读取结果为空。一个变更集的行在同一 `snapshot_seq` 下一起可见。`seq` 按刷写输出的每一行递增，重放可以留下空隙。

文本超过内联上限时，字节写入 `blobs`，`body_text` 为空，`blob_ref` 为该 sha256。

facet 的内容哈希与该种类最新可见版本相同时不追加行。比较范围包含本批前面的行。

### 恢复

存在刷写批次时：

- 最新快照的 `commit_ids` 已包含这批提交：只把 outbox 标为可见，不重放。
- 否则把各表 `restore` 到批次记录的版本，批次删除，outbox 保持已接收。下一次 `flush` 重放。

没有刷写批次时，表的当前版本高于最新快照记录的版本（没有快照时高于建表版本），`restore` 到该版本。当前版本低于记录时打开失败。`restore` 会生成新的表版本；在下一次成功刷写之前，后续打开仍可能看到“版本号更高、内容与快照相同”，并再次 `restore`。

读取只打开快照记录的表版本。`_rowid` 只在这一次打开里使用。

## 暂存

文件名是 sha256。写入使用临时文件、`sync` 和改名。引用它的变更集可见之后删除；仍被其他已接收变更集引用的文件保留。没有引用且修改时间超过 `staging_ttl` 的文件在 `flush` 时删除。

## 读取

同一句柄记录见过的最大 `snapshot_seq`，后续不带 `asOf` 的读取不低于它。`asOf` 不改变这个序号。

- `AsOf::Snapshot`：该序号必须存在。
- `AsOf::Time`：取 `published_at` 不晚于该时刻的最大快照。没有则结果为空。
- `AsOf::Tag`：本阶段不建标签，返回错误。

`resolve` 只看已可见身份，墓碑身份仍返回 RID。`state` 返回已可见的最新值和版本；版本是该键的可见行数。

`get(lazy=true)` 不读 blob 字节。内联文本仍返回。溢出到 blob 的文本只返回 `blob` 引用。`slice` 只接受内联文本或 blob，区间是字节半开区间。`history` 忽略版本选择。`changes(after, limit)` 返回该序号之后、且不超过句柄已见序号的快照，不在库内启动订阅。

资源的 `updatedAt` 是该资源 facet 的最大 `createdAt`，没有 facet 时等于资源的 `createdAt`。查询不隐藏墓碑。

### 查询

每个表达式对象只有一个键。

| 键 | 形状 |
|---|---|
| `all` | 非空表达式数组，同时成立 |
| `eq` | `[字段, 值]` |
| `gte` | `[字段, 值]` |

其他键返回错误，不按未命中处理。带 `search` 的查询返回错误。

时间字段按时间点比较，其他字段按 JSON 值比较。不能比较时返回错误。

`hasFacets` 和 `expand` 只用于 `from=resources`。选择器带 `variant` 时种类和变体都要匹配。`expand.depth` 缺省为 1，`0` 不展开。

未指定 `orderBy` 时按 `seq` 升序。`cursor` 是上一页最后一行的 `seq` 十进制，下一页只含更大的 `seq`。指定了 `orderBy` 时不接受 `cursor`。返回行数等于 `limit` 时给出下一页 cursor。

查询结果不内嵌 blob 字节，只给出 `blobRef`。选中的 facet 以 `kind` 或 `kind:variant` 为键。

## 未闭合

- `embedding` 的正文形态和维度
- `Body::External` 的内容哈希
- 全文与向量检索（设计第 13 节第 8 项）
- 审计标签
- PostgreSQL 与多实例租约（设计第 13 节第 3 项）
- `optimize` 与刷写交替的可见延迟（设计第 13 节第 2 项）

`lance` 13.0.0 上，内联阈值 16 字节、独立文件阈值 64 字节时，8 字节载荷写在 `blobs.lance/data/*.lance` 内，32 字节与 96 字节载荷各对应一个同目录下的 `*.blob` 文件。相同 `sha256` 不新增行。这两个 `*.blob` 文件的名字不区分 sidecar 与独立文件。
