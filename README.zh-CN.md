# ModalPrism

Rust 编写的AI原生的多信源数据采集系统 支持向量存储与多模态数据：把 RSS、网页、动态页面、API、推送等各类来源的数据统一采集并原样保存，支持异构多模态内容；数据只增不改、可重放。

项目基于[LEAF](https://github.com/MistLoom/LEAF/)框架打造：LEAF 是以 Rust 构建、为 AI 生态打造的轻量化 AI 开发框架，一套 Ability 同时服务 Agent、MCP、CLI、Skill、Tools 与 HTTP 等协议。

[![许可证](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-ed9e28?logo=rust&logoColor=white)](#快速开始)
[![存储](https://img.shields.io/badge/存储-SQLite%20%E2%86%92%20PostgreSQL-336791)](#架构)

[English](README.md) · **简体中文**

![ModalPrism 控制台 · 运行总览](assets/overview.png)

<p align="center"><sub>控制台 · 概览：信源、待处理、任务队列与已入库内容一眼看完，并带实时运行监测。</sub></p>

## 要点

- **一键启动，轻量化运行**：单二进制、零外部服务依赖，首次运行自动初始化存储；一条命令即完成采集、投递与查询服务的启动。
- **内置常用信源，计划开箱可用**：随仓库提供 39 条内置信源与 3 个默认关闭的示例采集计划（2 个多信源计划 + 1 个查询型计划），覆盖 AI、金融、医疗、科技四类权威站点，并按「AI 厂商与实验室、AI 研究与论文、AI 媒体与观点、金融监管与市场、医疗与期刊、科技媒体、自有与接入」七个内置分类归档；未启用的计划不会自动采集；示例计划只演示节律与额度，其余信源在计划页自建计划后才会采集。RSS/Atom、网页列表与详情页、JS 渲染页面、开放 API、按次计费的付费 API、搜索引擎、外部推送、本地文件一个进程全部接入。
- **采完即得，不用二次开发**：过滤、全文、向量检索融合在同一查询接口，业务代码不写 SQL；文本、结构化数据、图片、音视频统一为可寻址资源，重复内容自动去重。
- **多协议接入同一能力面**：HTTP API、MCP 工具、SKILL 脚本共享信源、内容与配置能力；内容投递、通知管理与运行监测通过 HTTP API 提供，支持矩阵见「公共 API」。
- **可视化控制台**：概览、项目、信源、计划、任务、内容、内容投递、通知管理、外部调用、日志、调试工具、系统设置十二个一级页面；建源表单、可排序可筛选的表格、信源自定义分类（左侧分类栏即管理范围，内置分类开箱分好七个领域，可批量移入与移出）、计划内投递目标、手动与自动投递、逐平台通知配置都在界面里完成，工作台实时显示运行监测。
- **原始数据逐字节留存，只增不改、可重放**：策略变更或识别升级后可对已抓数据重新解析，而不是重新爬一遍。
- **全链路可追溯**：每一层产出都记录生产者与所用配置，数据可审计、可复现。
- **对外请求集中治理**：限速、熔断、robots 合规、私网地址防护；付费调用有预算控制与重复请求合并。
- **配置即代码，原子热加载**：严格校验的 YAML 保存即生效；信源取什么、计划多久跑一次、新内容投给谁，都在声明里写清楚。

## 控制台一览

点击任意截图可看大图。

<table>
  <tr>
    <td width="50%" valign="top"><a href="assets/sources.png"><img src="assets/sources.png" alt="信源页"></a><br><b>信源</b> — 声明按可编辑分类归档，每行给出健康状态、关联计划、今日产出与下次运行；从模板起步，或直接在页面上生成适配器。</td>
    <td width="50%" valign="top"><a href="assets/project.png"><img src="assets/project.png" alt="项目页"></a><br><b>项目</b> — 项目是数据归属单元：信源、计划、内容与任务都按项目统计，切换项目即切换数据范围。</td>
  </tr>
  <tr>
    <td width="50%" valign="top"><a href="assets/tasks.png"><img src="assets/tasks.png" alt="任务页"></a><br><b>任务</b> — 即时、付费、渲染、推送四条队列之上是一份统一的执行记录，可按状态、类型、信源类型与计划筛选，再重排或取消。</td>
    <td width="50%" valign="top"><a href="assets/content.png"><img src="assets/content.png" alt="内容页与详情抽屉"></a><br><b>内容</b> — 每条内容保留原始字节与派生 facet（网页正文、结构化对象），带版本与关联，原文一键打开。</td>
  </tr>
  <tr>
    <td width="50%" valign="top"><a href="assets/sources_record.png"><img src="assets/sources_record.png" alt="采集记录抽屉"></a><br><b>采集记录</b> — 一次运行看全：本次产出了什么、重复采集跳过了什么，并可一键重采。</td>
    <td width="50%" valign="top"><a href="assets/notification.png"><img src="assets/notification.png" alt="通知管理页"></a><br><b>通知管理</b> — 飞书、企业微信、钉钉、Slack、Discord、Telegram 与通用 Webhook，逐事件勾选要推送的事件，保存前可先做连通检测。</td>
  </tr>
</table>

## 三十秒上手

三步从冷启动走到有数据可查：

1. 启动采集进程——首次运行自动初始化存储；内置信源与采集计划开箱可用，计划默认关闭，确认节律后在「计划」页启用（命令见[快速开始](#快速开始)）
2. 打开控制台——浏览器访问 `http://127.0.0.1:5178`
3. 直接查询已入库的内容：

```sh
curl -s "http://127.0.0.1:17880/api/v1/content/items?query=agent&limit=1"
```

```json
{
  "items": [
    {
      "id": "r_01ARZ3NDEKTSV4RRFFQ69G5FAV",
      "title": "New methods for training reliable reasoning models",
      "sourceId": "source/openai-news",
      "status": "active",
      "tags": ["ai", "research"],
      "publishedAt": "2026-09-14T04:00:00Z"
    }
  ],
  "nextCursor": "c…f"
}
```

## 定位

Firecrawl、Crawl4AI、RSSHub 等解决的是“抽一下”或“发个订阅源”；ModalPrism 覆盖调度、访问、存储、投递、查询的完整链路，并把采到的一切沉淀为可长期使用的数据资产。

| | ModalPrism | Firecrawl / Crawl4AI | RSSHub | Colly / Crawlee |
|---|---|---|---|---|
| 核心环节 | 采集 + 资源存储 + 查询 | 网页 → Markdown / 结构化抽取 | 为站点生成 RSS | 代码抓取框架 |
| 原始数据 | 原样保存、只增不改、可重放 | 归约为 Markdown/JSON | 输出订阅源 | 内存态、瞬时 |
| 来源追溯 | 逐层记录产出来源与配置 | — | — | — |
| 存储 | 统一资源库，SQLite → PostgreSQL | 自带落地目标 | — | 自带落地目标 |
| 查询 | 过滤 + 全文 + 向量 | 搜索/抽取接口 | RSS 订阅 | — |
| 控制台 | 十二页可视化控制台 | 托管面板 / 无 | 无 | 无 |
| 付费调用治理 | 预算控制、幂等去重 | 额度制（SaaS） | — | 自行实现 |
| 扩展信源 | 配置声明，或控制台模板新建 | 模板 / 代码 | 代码路由 | 代码 |
| Agent 接口 | MCP 只读工具 · HTTP · SKILL 包 | API/SDK | 订阅源 | — |

## 特性

### 一套模型接入异构信源
信源、访问器、适配器全部由 YAML 声明，仓库内置 39 条信源配置与 3 个默认关闭的示例采集计划，覆盖 AI、金融、医疗、科技四类权威站点。访问器负责取回，适配器负责解读，可自由组合；同一份原始数据可以由多个适配器解读为不同用途的资源。

### 原始数据只增不改，可重放
访问器不解释内容。保存下来的原始数据逐字节保持原样，任何抓取都可以重放。识别与分类结果以字节区间引用原文，不修改原文；facet 以不可变版本追加；每一层派生记录来源，可追溯到产出它的模块、版本与配置。

### 统一资源库 Strata
文本、结构化对象、图片、音视频统一为一个可寻址资源（RID），每个资源可有多种 facet（`raw`、`html`、`md`、`struct:article`、`image:cover` 等）。大对象内容寻址、自动去重，支持按区间读取。适配器没有写权限，所有写入经声明式变更集提交；游标、数据与任务确认在同一次提交中生效或都不生效。

### 过滤、全文、向量在同一查询中
类型化查询 DSL 融合元数据过滤、全文检索与向量检索，按一致快照读取，业务层不写 SQL。缺省部署只依赖单文件 SQLite，需要时切换 PostgreSQL。

### 对外请求集中治理
所有对外请求——包括无头浏览器发起的子请求——都经由同一道网关发出：按域限速、自适应节流、主机熔断、会话与代理绑定、SSRF 防护、RFC 9309 `robots.txt` 处理。付费调用另有一层治理：逻辑请求键、预算窗口、幂等回执。

### 声明式配置，原子热加载
信源、访问器、适配器与策略预设全部是校验严格的 YAML 文件——未知字段校验报错，配置不存数据库。热加载按版本生成快照（一次配置变更对应一份不可变版本，接口字段为 `generation`）并原子切换，在途任务保持派发时的那一份。

### 建源不用改代码
新增一个站点不需要提交代码：从模板（RSS/Atom、网页列表、搜索查询、JSON 接口）起步，在控制台填地址与选择器，保存即按同一套加载器校验并热加载。信源、访问器、适配器都支持完整的新建、改写、启停、删除与批量操作；每份声明带「还有几个信源在用它」的引用计数，删除前能看清影响面。

### 内容投递
采集到的内容可以直接投出去，不需要再搭一层加工作业：目标分三类——已登记的模型、外部 API、通知渠道（Webhook / 飞书）。手动投递在内容页与投递页完成；自动投递在计划表单里勾选目标，保存后新内容会按目标顺序逐条提交，每个目标的结果与耗时都留在投递记录里。模型调用支持 `api` / `local` / `cli` 三种方式，密钥只存环境变量名。详见「内容投递」与「通知」。

### 面向 Agent 的只读表面
已采集数据通过 MCP 暴露：类型化查询、健康状态、信源清单、单页抓取与配置溯源，供任意支持 MCP 的客户端接入；抽象类型（`Article`、`HotList`）提供业务视图。写入面被网关硬性收窄（白名单见「公共 API」节）；只读进程连数据目录也只读打开。

## 架构

```text
配置(YAML) ─► 调度器 ─► 任务队列(Strata) ─► 任务执行
                                             │ 访问器（经对外请求治理 / 付费治理）
                                             ▼
                                        原始信封 ─► 适配器路由 ─► 搜索适配器 ─► 发现新条目
                                                   └► 内容适配器 ─► facet
                                             │ 组装为变更集
                                             ▼
                          Strata（关系库 + 内容寻址 Blob）─► 抽象类型 / 下游消费者
```

| 组件 | 职责 |
|---|---|
| 调度器 | 按信源配置、节奏与健康状态生成访问任务 |
| 访问器 | 按协议取回原始数据，输出原始信封；不解析内容 |
| 对外请求治理 | 限速、熔断、robots、SSRF 防护、会话与代理 |
| 适配器 | 搜索适配器找出新条目，内容适配器产出 facet；均无写权限 |
| Strata | 资源、facet、关系、身份、血缘、状态与任务队列的存取 |
| 抽象类型 | 基于 Strata 的业务视图（Article、HotList 等），自行维护 schema 与状态 |

边界约束：只有 Strata 访问存储；访问器的对外请求只能经由统一网关发出；运行期状态存放在 Strata，与只读配置分离。

### 运行形态

| 进程 | 说明 | 对外表面 |
|---|---|---|
| `modalprism-collector` | 一条命令初始化存储、暴露领域 API 并运行采集与投递循环 | 环回 `/api/v1`（`:17880`）、正向代理 `:17881`、推送 `:17882` |
| `modalprism-console`（Node） | 托管 SPA，把 `/api/v1` 逐字节透明代理 | HTTP，`127.0.0.1:17890` |
| `modalprism-agent` | 只读表面（查询 / 健康 / 信源清单 / 配置溯源 / 类型视图） | MCP stdio |

## 快速开始

### 构建

```sh
cargo build --workspace
```

以上为完整构建，无需额外参数，也无需系统安装 `protoc`（构建脚本自带）。对象存储缺省走外部 RustFS 发行包：先在配置路径与 `PATH` 里找二进制，都没有才下载预编译发行包；完全离线时可配置为快速失败。另支持切换为文件存储或外部 S3 兼容服务，默认不需要。

### 启动采集

在仓库根目录执行；首次运行自动初始化存储，内置信源与采集计划开箱可用，计划默认关闭，确认后在计划页启用：

```sh
cargo run -p modalprism-collector
```

### 启动控制台

前端为 Vite + React + TypeScript（pnpm 管理）：

```sh
cd apps/modalprism-console && pnpm install && pnpm dev
```

浏览器访问 `http://127.0.0.1:5178`，接口直连 collector。路由为哈希模式：`#/overview`、`#/projects`、`#/sources`、`#/plans`、`#/tasks`、`#/content`、`#/delivery`、`#/notifications`、`#/external`、`#/logs`、`#/tools`、`#/settings`。

### 启动只读 MCP 进程

```sh
cargo run -p modalprism-agent
```

数据与配置目录与 collector 相同，接入方式见「公共 API」。

## 信源与计划声明

信源是 `config/sources/*.yaml` 中的声明式入口，说明访问什么、怎么解析；执行节奏与数量上限写在配套的采集计划里。一条内置信源：

```yaml
id: source/openai-news
kind: source
name: OpenAI News
sourceType: feed
labels:
  tier: T1
  topic: 官方
access:
  accessor: accessor/rss
  locator:
    url: https://openai.com/news/rss.xml
pipeline:
  - run: [adapter/feed-items]
project: proj_default
shared: true
admission: {}
```

计划是 `config/plans/*.yaml`，一个计划可以覆盖多个信源，负责节奏、数量上限与停止条件：

```yaml
id: plan/openai-news
kind: plan
name: OpenAI News 采集计划
trigger:
  kind: recurring
  interval: 2h
targets:
  - source/openai-news
quota:
  perRunItems: 8
  maxAge: 365d
```

## 内容投递

采集到的内容直接投出去，中间不再有加工环节。目标分三类：已登记的模型、外部 API、通知渠道（Webhook / 飞书）。

- **手动投递**：在「内容」页选中内容后点「投递」，弹窗里只选目标——内容已经定了，不再重选一遍；「内容投递」页从内容库检索后批量投递，流程相同。
- **自动投递**：打开计划表单，在「投递目标」里勾选，可多选，留空表示不自动投递，保存即生效。此后该计划每次采到新内容，按目标顺序逐条提交。
- **投递记录**：逐目标记录成功、失败、耗时与输出摘要，区分手动与自动，单次投递可回看完整轨迹。

模型写在 `config/models/` 下，也可以在「系统设置 → 用户设置」里建。最小的一条：

```yaml
# config/models/digest.yaml
name: 摘要模型
mode: api
endpoint: https://api.deepseek.com/v1
model: deepseek-chat
keyEnv: DEEPSEEK_API_KEY
```

三种调用方式：`api` 走远端 OpenAI 兼容接口（密钥只写环境变量名），`local` 走本机或局域网端点（Ollama、vLLM、LM Studio），`cli` 把提示词写进子进程标准输入（`claude -p`、`llm`）。

接口上，`GET /delivery/targets` 列出可选目标；`GET|POST /delivery/rules` 与 `PATCH|DELETE /delivery/rules/{ruleId}` 维护投递规则；`POST /delivery/jobs` 发起投递；`GET /delivery/records` 查记录。外部 API 端点走 `/delivery/endpoints`，通知渠道走 `/delivery/channels`。完整清单见「公共 API」。

## 通知

通知仅覆盖系统事件，与内容投递分开：通知中心把事件留在本机，按分类开关并支持浏览器提醒；通知推送把事件发到飞书、企业微信、钉钉、Slack、Discord、Telegram 或任意 Webhook，逐个渠道勾选需要的事件（计划启动、采集完成、内容入库、信源异常等），保存前可先做连通检测。通知历史与推送历史都在控制台可查。内容投递用的通知渠道在「内容投递」页维护，两边互不影响。

飞书接入：群机器人 → 自定义机器人 → 复制 Webhook；安全设置选「自定义关键词」并把关键词填成 `ModalPrism`（不支持签名校验）。在控制台「通知 → 通知推送」里粘贴后点「连通检测」，通过即可保存。

## 公共 API

collector 对外只有一套契约，控制台原样代理。三个接入面共享信源、内容与配置能力；内容投递、通知管理与运行监测只通过 HTTP API 提供。控制台「外部调用」页按协议分组列出全部可用方法（HTTP 再按功能分组，可搜索、可复制 curl），并附快速指引与调用约定；清单由代码生成，与路由表逐条比对，接口增减会立即暴露：

| 功能 | HTTP API | MCP 工具 | SKILL 脚本 |
|---|:---:|:---:|:---:|
| 系统 · 查看运行状态与健康 | ✓ | ✓ | ✓ |
| 信源 · 查看清单与模板 | ✓ | ✓ | ✓ |
| 信源 · 新建 / 改写 / 启停 | ✓ | ✓（白名单） | — |
| 信源 · 删除、探针等完整运维 | ✓ | — | — |
| 内容 · 关键词 / 全文 / 语义检索 | ✓ | ✓ | ✓ |
| 内容 · 读取正文与历史版本 | ✓ | ✓ | ✓ |
| 内容 · 追溯来源与关系 | ✓ | ✓ | — |
| 业务视图 · 文章 / 热榜 | ✓ | ✓ | — |
| 即时采集 · 关键词搜索 | ✓ | ✓ | — |
| 即时采集 · URL 取正文 | ✓ | ✓ | — |
| 即时采集 · 结构化抽取 | ✓ | ✓ | — |
| 内容投递 · 手动投递、计划投递规则与记录 | ✓ | — | — |
| 内容投递 · 目标清单、外部端点与通知渠道 | ✓ | — | — |
| 外部调用 · 协议清单、完整方法与快速指引 | ✓ | — | — |
| 通知管理 · 本机通知中心与推送平台 | ✓ | — | — |
| 监测 · 查看调用链与运行统计 | ✓ | — | — |
| 配置 · 溯源、对比与校验 | ✓ | ✓ | — |
| 运维 · 查询日志与事件 | ✓ | — | ✓ |

> ✓ 表示支持；MCP 的信源写操作仅开放可恢复的白名单方法（见下）。

### HTTP

```sh
curl -s http://127.0.0.1:17880/api/v1/system/status
curl -s "http://127.0.0.1:17880/api/v1/catalog/sources?limit=20"
curl -s "http://127.0.0.1:17880/api/v1/content/items?query=agent&limit=10"
curl -s http://127.0.0.1:17880/api/v1/operations/op_01...
curl -s -X POST http://127.0.0.1:17880/api/v1/collection/runs \
  -H 'idempotency-key: manual-1' -H 'content-type: application/json' \
  -d '{"sourceId":"openai-news","mode":"normal"}'
```

异步动作返回 `202`、Operation 正文与 `Location`。同一表面覆盖 catalog、collection、content、delivery、notifications、search、settings、operations、observability 与 monitor；不提供通用 Ability invoke，也不提供原始配置文件的直接读写。

项目是一等资源：采集与投递资源归属项目；`POST /fetch` 与 `POST /parse` 把 URL 变为正文与结构化抽取。

### MCP

collector 之外另起一个只读进程（启动见「快速开始」），供任意支持 MCP 的客户端查询已采集数据。工具按「能力实例.方法」命名，例如 `strata_read.query`（检索）、`type_article.project`（文章视图）、`catalog_admin.templates`（信源模板）。写侧只开一份可恢复写白名单（信源/访问器/适配器声明的新建、改写与启停）；破坏性删除与系统设置不在工具列表里。

### SKILL 包

`skills/modalprism/` 是一份 Agent Skill：调用顺序、字段说明与常见任务配方，附一个零依赖的 HTTP 客户端脚本，没有 MCP 客户端也能用：

```sh
node skills/modalprism/scripts/prism.mjs status
node skills/modalprism/scripts/prism.mjs content --query agent --limit 10
node skills/modalprism/scripts/prism.mjs sources
```

## 仓库结构

```text
apps/modalprism-collector/ 采集与投递进程
apps/modalprism-console/   控制台（Web SPA + Node 外壳）
apps/modalprism-agent/     只读 MCP 进程
apps/modalprism-domain/    领域库（资源存储、配置、类型与 API）
frame/                     协议端口与宿主框架（HTTP / CLI / MCP / gRPC / fun）
SDK/                       多语言能力调用 SDK（Rust / TypeScript / Python / Cangjie）
abilitys/                  共享能力包
config/                    系统配置、访问器、适配器、信源、计划与策略预设（YAML）
skills/                    Agent Skill 包
```

## 许可证

Apache-2.0，见 [LICENSE](LICENSE)。
