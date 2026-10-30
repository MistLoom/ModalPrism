---
name: modalprism
description: 查询 ModalPrism 已采集的内容与信源，发起内容投递并追踪投递记录与日志。当用户需要「看最近入库的内容」「按关键词检索」「读一篇内容」「列出信源」「把内容投给模型或接口」「查看失败原因」时使用。
---

# ModalPrism

这个 Skill 做三件事：查业务资源、发起内容投递、追踪投递结果。信源、模型、外部接口、通知渠道与设置的修改在控制台完成，不通过这个 Skill 改配置。

## 前置

collector 在运行（仓库根目录 `cargo run -p modalprism-collector`）。先确认：

```sh
node scripts/prism.mjs status
```

返回 `status` 与 `version` 才继续。

基址默认 `http://127.0.0.1:17880`，用 `--base http://host:port` 或 `MODALPRISM_BASE` 覆盖；控制台地址同样是有效入口，代理不改变响应。命令一览：`node scripts/prism.mjs help`。

## 任务配方

### 看信源

```sh
node scripts/prism.mjs sources --limit 20
```

每项含 `id`、`name`、`enabled`、`endpoint`、`schedule`、`health`、`nextRunAt` 与 `etag`。健康只用于筛选，不替代任务详情。

### 查内容

```sh
node scripts/prism.mjs content --query "agent memory" --limit 10
node scripts/prism.mjs content --source hugging-face --status active
node scripts/prism.mjs content-item r_01ARZ3NDEKTSV4RRFFQ69G5FAV
```

列表只返回业务字段：`id`（即 RID）、`title`、`sourceId`、`status`、`tags`、`publishedAt`、`createdAt` 与 `etag`。正文、版本、血缘与关系在详情与独立子资源里。

### 投递内容

```sh
# 1. 看可选目标：模型、外部 API、通知渠道，键为 kind:id
node scripts/prism.mjs targets

# 2. 发起投递，随后轮询 Operation
node scripts/prism.mjs deliver r_01ARZ...,r_02BQC... --targets model:model/atomcode,endpoint:endpoint_...
node scripts/prism.mjs op op_...
```

投递按目标顺序逐条提交，单个目标失败不回滚已成功的其他目标；结果里每个目标都带状态、耗时与输出摘要。内容页与计划表单做的是同一件事，区别只在入口。

### 查投递记录

```sh
node scripts/prism.mjs records --limit 20
node scripts/prism.mjs records --type auto --status failed
```

记录区分手动与自动：自动投递由计划表单里的投递目标驱动，`trigger.ruleName` 指出是哪条规则。

### 查失败原因

```sh
node scripts/prism.mjs records --status failed
node scripts/prism.mjs logs --correlation op_... --level error
node scripts/prism.mjs logs --correlation run_... --limit 50
```

`requestId`、`operationId`、`taskId`、`runId`、`deliveryId` 可以互相追踪。

## 直接调 HTTP

脚本未覆盖的请求走公共 HTTP，前缀 `/api/v1`：`GET /system/status`、`GET /catalog/sources`、`GET /content/items`（含 `/{id}` 与 `/{id}/body`）、`GET /delivery/targets`、`POST /delivery/jobs`、`GET /delivery/records`、`GET|POST /delivery/rules`、`GET /delivery/channels`、`GET /operations/{id}`、`GET /observability/logs`。全部方法与参数见控制台「外部调用」页。

契约约定：写请求需要 `Idempotency-Key`；资源读取带 `ETag`；列表统一返回 `{"items":[],"nextCursor":null}`；错误统一返回 `{"error":{"code","message","fieldErrors","requestId","retryable"}}`；异步动作返回 `202` 与 Operation。

## 边界

- 不通过 API 改信源、模型、外部接口、通知渠道或设置；这些在控制台的对应页面完成。
- 不把 `seq` 当作稳定标识；用 `id`（RID）、`operationId`、`runId` 与 `deliveryId`。
- 投递会产生真实的对外请求（模型计费、接口调用、消息推送）；确认目标后再发起。
- 报告结果时给出 `id`、`sourceId` 与 requestId，便于在日志页追踪。
