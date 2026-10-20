# Web 控制台

采集进程在环回地址上提供 Web 控制台，用于观察采集状态、浏览已沉积数据、管理出站治理，并在确认后修改 YAML 配置。控制台通过 `/console/api/v1/*` 访问后端，不暴露 `/v1/abilities` 协议。

## 访问方式

启动 `modalprism-collector` 后打开 `http://<http 监听>/console/`。编译采集进程时会自动构建并嵌入 `apps/console-web` 产物；也可设置 `MODALPRISM_CONSOLE_DIST` 指向已有 `dist` 目录。若嵌入产物缺失，访问 `/console/` 会返回说明页而非 404。

| 端口 | 默认 | 用途 |
|------|------|------|
| ops（`--http`） | `127.0.0.1:17880` | `/health`、`/console/`、`/console/api/v1/*` |
| proxy | `127.0.0.1:17881` | 出站代理 |
| push | `127.0.0.1:17882` | 推送入口 |

仅允许环回地址绑定。控制台与 ops 端口相同，**不另加令牌**；写接口通过 Host/Origin（仅环回）与 JSON `Content-Type` 或 `X-ModalPrism-Console` 请求头降低跨站与 DNS 重绑定风险。

## API 概要

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/meta` | 版本、配置代际、是否只读（`asOfSnapshot` 查询参数） |
| GET | `/timeline` | 可见沉积点列表（用于历史浏览） |
| GET | `/overview` | 信源与健康摘要、最近更新 |
| GET | `/sources`、`/sources/{id}` | 信源列表与详情（业务字段） |
| POST | `/sources/{id}/trigger` | 立即调度（`confirm: true`） |
| PATCH | `/sources/{id}` | 写 YAML `enabled` 并校验、加载 |
| GET | `/records` | 列表：筛选、搜索、分页、`asOfSnapshot` |
| GET | `/records/{id}` | 详情（实时；历史模式返回错误提示） |
| GET | `/governance/egress` | 域熔断状态 |
| POST | `/governance/egress/{host}/reset-circuit` | 重置熔断 |
| GET | `/config/summary` | 代际与磁盘 diff 计数 |
| GET | `/config/files` | 白名单文件列表 |
| GET/PUT | `/config/files/{path}` | 读/写 YAML（PUT 需 `confirm`，写前备份） |
| POST | `/config/file-validate` | 校验草稿（不写盘） |
| POST | `/config/file-rollback` | 回滚到写入前备份并加载 |
| POST | `/config/validate`、`/config/reload` | 全目录校验、重新加载 |

配置写入白名单：`sources/`、`presets/`、`accessors/`、`adapters/`、`types/`、`system.yaml`。拒绝 `..`、绝对路径与解析后越出配置根目录的路径（含符号链接）。

备份文件位于数据目录 `.console-config-backups/`。

## 前端

- TypeScript + Vite；默认浅色主题，可选深色与跟随系统（CSS 变量，目标 WCAG AA）。
- 视图：总览、信源、数据记录（筛选/详情抽屉）、治理、配置（YAML 编辑器、差异提示、校验、保存、回滚）。
- 侧栏可选择**历史沉积点**：记录列表按快照只读；写操作与配置编辑在只读模式下禁用。
- 命令面板（Ctrl+K）、确认对话框、空/加载/错误状态；时间按浏览器本地时区显示。

## 测试

- `cargo test -p console-api`：端点、白名单、CSRF、写入回滚。
- `cd apps/console-web && npm run build && npm test`：类型检查与单元测试。
- `npm run e2e`（Playwright）：空目录启动采集进程，验证启停信源、配置编辑回滚与立即采集。
