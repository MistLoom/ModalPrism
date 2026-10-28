---
name: modalprism
description: 查询 ModalPrism 的内容、信源、生产运行与日志，并触发一次生产运行或审核候选。当用户需要「看最近入库的内容」「按关键词检索」「读一篇内容」「列出信源与工作流」「围绕素材生成内容」「查看失败原因」时使用。走公共 HTTP（默认 127.0.0.1:17880）或附带脚本，不直接读数据库。
---

# ModalPrism

ModalPrism 把信源采集、内容生产、发布与治理收敛到同一个领域 API。这个 Skill 只做三件事：查业务资源、冻结输入并创建生产运行、审核候选。信源、适配器、工作流版本、提示词、Skill 与设置都在控制台修改，不通过这个脚本改配置。

## 前置

采集进程在跑（`cargo run -p modalprism-collector`）。先确认：

```sh
node scripts/prism.mjs status
```

返回 `status`、`version` 与 `generation` 才继续。`generation` 每次设置应用会递增，可用来判断配置是否生效。

基址默认 `http://127.0.0.1:17880`，用 `--base http://host:port` 或 `MODALPRISM_BASE` 覆盖；也可以指向 Node 控制台 `http://127.0.0.1:17890`，代理不改变响应。

## 任务配方

### 看信源

```sh
node scripts/prism.mjs sources --limit 20
```

每项含 `id`、`name`、`enabled`、`accessMethodId`、`adapterIds`、`endpoint`、`schedule`、`health`、`nextRunAt` 与 `etag`。健康只用于筛选，不替代任务详情。

### 查内容

```sh
node scripts/prism.mjs content --query "agent memory" --limit 10
node scripts/prism.mjs content --source hugging-face --status active
node scripts/prism.mjs content-item r_01ARZ3NDEKTSV4RRFFQ69G5FAV
```

列表只返回业务字段：`id`（即 RID）、`title`、`sourceId`、`status`、`tags`、`publishedAt`、`createdAt` 与 `etag`。正文、版本、血缘与关系在详情与独立子资源里。

### 生产一个候选

```sh
# 1. 冻结输入（文本；也可以是 content / contentList / querySnapshot / url / structured / file）
node scripts/prism.mjs input-text "今日素材" "需要改写的原文…"

# 2. 选一个工作流版本
node scripts/prism.mjs workflows
node scripts/prism.mjs versions wf_...

# 3. 创建运行，随后轮询 Operation
node scripts/prism.mjs run iset_... wfver_...
node scripts/prism.mjs op op_...
```

运行结束后结果里带 `candidateId`。候选默认 `awaitingReview`；未审核的候选不会进入草稿或发布。

### 审核候选

```sh
node scripts/prism.mjs candidates --status awaitingReview
node scripts/prism.mjs review cand_... approve --comment "可以采用"
node scripts/prism.mjs review cand_... reject --comment "事实不足"
```

审核需要资源的 `etag`；脚本会先读列表再带 `If-Match`，冲突时返回 412，必须重新读取后重试。

### 查失败原因

```sh
node scripts/prism.mjs runs --status failed
node scripts/prism.mjs logs --correlation op_... --level error
node scripts/prism.mjs logs --correlation run_... --limit 50
```

`requestId`、`operationId`、`taskId`、`runId` 可以互相追踪。日志与事件分开保存；审计不随普通日志清理。

## 直接调 HTTP

| 方法 | 路径 | 说明 |
|---|---|---|
| `GET` | `/api/v1/system/status` | 服务状态与配置代际 |
| `GET` | `/api/v1/catalog/sources` | 信源领域资源 |
| `GET` | `/api/v1/content/items?query=&sourceId=&status=` | 内容列表 |
| `GET` | `/api/v1/content/items/{id}`、`/body` | 内容详情与正文 |
| `GET` | `/api/v1/production/workflows`、`/{id}/versions` | 工作流与不可变版本 |
| `GET` | `/api/v1/production/inputSets` | 冻结输入集 |
| `POST` | `/api/v1/production/inputSets` | 冻结新输入集合 |
| `POST` | `/api/v1/production/runs` | `{"inputSetId":"…","workflowVersionId":"…","mode":"manual"}` |
| `GET` | `/api/v1/production/candidates` | 候选 |
| `POST` | `/api/v1/production/candidates/{id}/reviews` | `{"decision":"approve"}` + `If-Match` |
| `GET` | `/api/v1/operations/{id}` | 异步动作状态与结果摘要 |
| `GET` | `/api/v1/observability/logs` | 日志与关联筛选 |

写请求需要 `Idempotency-Key`；所有资源读取带 `ETag`。列表统一返回 `{"items":[],"nextCursor":null}`；错误统一返回 `{"error":{"code","message","fieldErrors","requestId","retryable"}}`。

## 有 MCP 客户端时

`cargo run -p modalprism-agent -- --data var --config config` 起只读 MCP 服务，工具名形如 `strata_read.query`、`type_article.project`、`catalog_admin.list`、`workflow.catalog`。MCP 只读；写入仍走公共 HTTP。

## 边界

- 不通过 API 改信源、适配器、工作流、提示词或设置；这些在控制台的对应页面完成。
- 不把 `seq` 当作稳定标识；用 `id`（RID）、`operationId`、`runId` 与 `candidateId`。
- 生成结果永远是候选；批准不触发发布，发布是另一个显式动作。
- 报告结果时给出 `id`、`sourceId` 与 requestId，便于在日志页追踪。
