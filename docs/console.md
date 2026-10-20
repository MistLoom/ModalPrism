# Web 控制台

ModalPrism 采集进程在环回地址上提供 Web 控制台，用于观察采集状态、浏览已沉积的数据、查看出站治理，并在确认后修改 YAML 配置。控制台不暴露 `/v1/abilities` 调用协议，也不把 Strata 内部术语作为一级界面语言。

## 与运维端口的关系

| 监听 | 默认地址 | 用途 |
|------|----------|------|
| ops（HTTP） | `127.0.0.1:17880` | `/health`、`/v1/abilities/*`（运维与自动化）、`/console/`（本控制台静态资源）、`/console/api/v1/*`（控制台 API） |
| proxy | `127.0.0.1:17881` | 出站正向代理 |
| push | `127.0.0.1:17882` | `POST /<channel>` 推送入口 |

启动 `modalprism-collector` 后，在浏览器打开 `http://127.0.0.1:17880/console/`。仅允许环回地址绑定。

## 设计稿对照

设计稿为静态原型（假数据、固定端口 `7420`、虚构信源与 RID）。下表以当前代码为准。

| 设计稿元素 | 分类 | 控制台实现 |
|------------|------|------------|
| 总览：任务数、信源健康分布、最近活动 | 可直接对应 | 队列待办计数、`health` + `planner` 状态、最近 `strata_read.changes` 与配置代际 |
| 总览：成本/趋势曲线（¥、sparkline） | 需新建或删减 | 不展示虚构成本；可选展示队列深度与失败计数（无货币维度） |
| 信源列表与详情抽屉 | 可直接对应 | 已生效配置中的 `kind: source` + `planner.source_status` + 健康 facet |
| 信源：手动暂停/恢复 | 需新建 | 通过写 `enabled: false/true` 到对应 YAML 后 `config.load` |
| 信源：立即采集 | 可直接对应 | `planner.trigger` |
| Strata：资源 RID、facet 版本树 | 部分对应 | 业务层「数据记录」列表；进阶区展示内部 ID 与 facet 种类 |
| Strata：embedding / 多版本 diff 演示 | 项目不符 | 仅展示项目中实际存在的 facet 种类；无 embedding 则不显示 |
| 治理：按域熔断表与重置 | 部分对应 | 读取 egress 持久化中的 `circuits`；提供重置单域熔断（非变更集） |
| 治理：会话池、身份轮换叙事 | 需新建或删减 | 不展示设计稿中的会话池 fiction；robots 与限速以 egress 审计与策略字段为准 |
| 配置：代际 G-xx | 可直接对应 | `config.generation` |
| 配置：待提交变更集、多代 diff 时间线 | 部分对应 | `config_inspect.diff` + 磁盘 YAML 编辑；无 Git 式变更集哈希链 |
| 配置：在线 YAML 编辑并提交 | 需新建 | 按文件路径写回 + `validate` + `load`；失败则保持旧快照 |
| 快照条：#1842 时间轴 scrubber | 部分对应 | 展示最新可见快照序号与 `changes`；按历史快照只读浏览为后续能力 |
| 命令面板 Ctrl+K | 可直接对应 | 导航、触发采集、打开信源、校验配置 |
| 熔断重置二次确认 | 可直接对应 | 确认对话框后调用 API |
| 历史快照只读 | 需新建 | 首版仅「实时」；历史模式占位并禁用写操作 |
| 环回 `127.0.0.1:7420` | 项目不符 | 使用采集进程实际 `--http` 地址 |
| 「变更集提交」按钮文案 | 项目不符 | 配置写操作为「校验并加载」；采集写入由 committer 自动 flush |

## API 契约（`/console/api/v1`）

所有响应 JSON。错误体：`{ "error": { "code": "...", "message": "..." } }`。

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/meta` | 版本、配置代际、最新快照序号、监听地址 |
| GET | `/overview` | 队列统计、信源健康摘要、最近变更 |
| GET | `/sources` | 信源列表（业务字段） |
| GET | `/sources/{id}` | 单信源详情 |
| POST | `/sources/{id}/trigger` | 立即调度一轮采集 |
| PATCH | `/sources/{id}` | 体 `{ "enabled": bool }`，写 YAML 并 reload |
| GET | `/records` | 查询参数 `limit`、`cursor`；数据记录列表 |
| GET | `/records/{id}` | 单记录详情（含 facet 摘要） |
| GET | `/governance/egress` | 域级限速与熔断状态 |
| POST | `/governance/egress/{host}/reset-circuit` | 重置该主机熔断状态 |
| GET | `/config/summary` | 代际、模块数量、与磁盘 diff 摘要 |
| POST | `/config/validate` | 校验配置目录 |
| POST | `/config/reload` | 重新加载（等同 `config.load`） |
| GET | `/config/files/{path}` | 读取配置树下文本文件（相对路径，禁止 `..`） |
| PUT | `/config/files/{path}` | 写回文件；不自动 reload |

写操作除 PATCH/PUT 外均要求 JSON 体 `{ "confirm": true }`（熔断重置、trigger 可选二次确认由 UI 完成）。

## 前端结构

- 技术栈：TypeScript、Vite、原生组件（无重型框架），样式来自设计令牌（CSS 变量）。
- 主题：默认浅色；可选深色与 `prefers-color-scheme`。
- 路由：`overview`、`sources`、`records`（对应设计稿 Strata）、`governance`、`config`。
- 空状态与错误：按 API 错误码与信源 `skipReason`（预算、robots、disabled）展示说明。

## 测试

- `console-api`：单元与集成测试（环回服务 + 临时目录配置）。
- `console-web`：`tsc`、`vite build`；Vitest 覆盖命令面板与主题切换。
- 端到端：空数据目录启动采集进程，执行 `cd apps/console-web && npm run build && npm run screenshots -- http://127.0.0.1:<http端口>`，在 `CONSOLE_ARTIFACTS`（默认 `/opt/cursor/artifacts/console`）生成各视图浅色/深色截图。

## 待决事项

- 控制台是否需要鉴权（当前与 ops 端口相同，无 TLS/令牌）。
- 历史快照只读浏览的优先级与 Strata `observed` 语义的产品化文案。
- 配置 PUT 是否允许任意路径，或限制在 `sources/`、`presets/` 等白名单。
