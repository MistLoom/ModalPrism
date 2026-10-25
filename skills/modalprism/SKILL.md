---
name: modalprism
description: 查询 ModalPrism 已采集的内容并触发内容加工。当用户需要「看看某个信源最近的文章」「按关键词检索已采集内容」「读一篇已入库文章的正文」「列出信源与工作流」「立刻加工一批内容」时使用。走公共 HTTP（默认 127.0.0.1:17880）或附带脚本，不直接读数据库。
---

# ModalPrism

ModalPrism 把各站点的内容采进统一资源库。这个 Skill 只做两件事：查已采集的内容，触发一次加工。它不负责新增信源、改适配器、改工作流——那些属于配置写入，要用控制台。

## 前置

采集进程在跑（`cargo run -p modalprism-collector`）。先确认：

```sh
node scripts/prism.mjs health
```

返回 `{"status":"ok","service":"modalprism","generation":N}` 才继续。`generation` 每次配置改动会 +1，可以用来判断配置是否已生效。

基址默认 `http://127.0.0.1:17880`，用 `--base http://host:port` 或 `MODALPRISM_BASE` 覆盖。

## 任务配方

### 看某个信源最近的文章

```sh
node scripts/prism.mjs sources                     # 先拿到信源的 id / slug
node scripts/prism.mjs articles --source hugging-face --limit 5
```

`--source` 收 slug（`hugging-face`）或完整 id（`source/hugging-face`），两种都行。

每条内容顶层固定有 `rid`、`seq`、`createdAt`、`sourceId`，其余字段摊平自 `struct:article`（常见 `title`、`url`、`summary`，随适配器而异）。**不要假设某个字段一定存在**，缺了就用 `article` 命令看完整投影。

### 按关键词检索

```sh
node scripts/prism.mjs articles --q "agent memory" --limit 10
```

检索按相关度返回，不支持翻页：带 `--q` 时忽略游标。

### 读一篇内容

```sh
node scripts/prism.mjs article r_01ARZ3NDEKTSV4RRFFQ69G5FAV
```

返回 `{rid, struct, readable, cover}`：`struct` 是结构化字段，`readable` 是可读正文（可能为 `null`），`cover` 是封面图的摘要信息。

RID 只能从列表或运行记录里拿，不要臆造：格式对了但不存在会返回 404。

### 看加工配置与历史

```sh
node scripts/prism.mjs workflows
node scripts/prism.mjs runs --limit 10
```

`workflows` 返回 `{workflows, models}`，每项含 `slug`、`name`、`enabled`、`trigger`、`steps`、`outputs`、`notify`。`runs` 是最近的运行记录，含 `status`、`durationMs` 与每步输出。

### 触发加工

```sh
node scripts/prism.mjs run daily-brief r_01ARZ3NDEKTSV4RRFFQ69G5FAV   # 对一篇跑一条工作流
node scripts/prism.mjs process                                        # 把游标之后的新内容都加工一遍
```

`run` 不看 `trigger`，适合补跑与试跑；`process` 是幂等的「推进一次」，没有新内容时返回 `scanned: 0`。

## 直接调 HTTP

脚本只是薄封装，任何语言都可以直接调：

| 方法 | 路径 | 说明 |
|---|---|---|
| `GET` | `/api/v1/health` | 存活与配置代际 |
| `GET` | `/api/v1/articles?limit=&source=&q=` | 内容列表 |
| `GET` | `/api/v1/articles/{rid}` | 单篇投影 |
| `GET` | `/api/v1/sources` | 信源、访问器、适配器 |
| `GET` | `/api/v1/workflows` | 工作流与模型 |
| `GET` | `/api/v1/runs?limit=` | 运行记录 |
| `POST` | `/api/v1/workflows/{slug}/run` | 正文 `{"rid":"r_…"}` |
| `POST` | `/api/v1/process` | 推进一次加工 |

`limit` 默认 20、上限 50。出错时正文是 `{"error":{"code":"…","message":"…"}}`：`invalid_input` 改参数，`not_found` 换 rid 或 slug，`service_unavailable` 说明进程或存储没起来，`bad_gateway` 是内部能力报错（看采集进程日志）。

## 有 MCP 客户端时

`cargo run -p modalprism-agent -- --data var --config config` 起只读 MCP 服务，工具名形如 `strata_read.query`、`workflow.catalog`、`catalog_admin.list`。查询能力比 HTTP 面更全（历史、血缘、切片、类型化查询）；写入仍然只有 HTTP 的 `run` 与 `process`。

## 边界

- 只读与加工：新增/修改信源、适配器、工作流一律走控制台（`#/sources`、`#/channels`、`#/workflows`），不要试图通过 API 改配置。
- 不缓存、不聚合：返回的就是库里的投影，取多了用 `--limit` 收窄而不是翻页。
- 报告结果时给出 `rid` 与 `sourceId`，不要把 `seq` 当成稳定标识（重放会变）。
