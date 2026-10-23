# ModalPrism

Rust 编写的多信源数据采集系统：把 RSS、网页、动态页面、API、推送等各类来源的数据统一采集并原样保存，支持异构多模态内容；数据只增不改、可重放，采完即可查询。

[![许可证](https://img.shields.io/badge/license-Apache--2.0-blue)](#许可证)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-ed9e28?logo=rust&logoColor=white)](#快速开始)
[![存储](https://img.shields.io/badge/存储-SQLite%20%E2%86%92%20PostgreSQL-336791)](#架构)

[English](README.md) · **简体中文**

## 要点

- **一个进程接入全部信源类型**：RSS/Atom、网页列表与详情页、JS 渲染页面、开放 API、按次计费的付费 API、搜索引擎、外部推送、本地文件；内置 23 个信源配置，新增信源只需一个 YAML 文件。
- **原始数据逐字节留存，只增不改、可重放**：策略变更或识别升级后可对已抓数据重新加工，而不是重新爬一遍。
- **全链路血缘**：每一层加工记录产出者与所用配置，数据可审计、来源可追溯。
- **即采即查**：文本、结构化数据、图片、音视频统一为可寻址资源，重复内容自动去重；本地单文件即可运行，需要时切换 PostgreSQL。
- **过滤、全文、向量检索融合在同一查询接口**，业务代码不写 SQL。
- **出站流量集中治理**：限速、熔断、robots 合规、私网地址防护；付费调用有预算控制与重复请求合并。
- **配置热加载**：严格校验的 YAML，变更原子生效。
- **MCP 只读表面**：Agent 直接对已采集数据做查询、健康检查与配置溯源。

## 定位

Firecrawl、Crawl4AI、RSSHub 等解决的是“抽一下”或“发个订阅源”；ModalPrism 覆盖调度、访问、加工、存储、查询的完整链路，并把采到的一切沉淀为可长期使用的数据资产。

| | ModalPrism | Firecrawl / Crawl4AI | RSSHub | Colly / Crawlee |
|---|---|---|---|---|
| 核心环节 | 采集 + 资源存储 + 查询 | 网页 → Markdown / 结构化抽取 | 为站点生成 RSS | 代码抓取框架 |
| 原始数据 | 原样保存、只增不改、可重放 | 归约为 Markdown/JSON | 输出订阅源 | 内存态、瞬时 |
| 血缘 | 每层记录产出模块/版本/配置 | — | — | — |
| 存储 | 统一资源 + facet，SQLite → PostgreSQL | 自带落地目标 | — | 自带落地目标 |
| 查询 | 过滤 + 全文 + 向量 DSL | 搜索/抽取接口 | RSS 订阅 | — |
| 付费调用治理 | 预算、逻辑请求键、幂等回执 | 额度制（SaaS） | — | 自行实现 |
| 扩展信源 | YAML 声明 | 模板 / 代码 | 代码路由 | 代码 |
| Agent 接口 | MCP 只读表面 | API/SDK | 订阅源 | — |

## 特性

### 一套模型接入异构信源
信源、访问器、适配器全部由 YAML 声明，仓库内置 23 个信源配置（以 AI/科技类站点为主）。访问器负责取回，适配器负责解读，可自由组合；同一份原始数据可以由多个适配器加工成不同用途的资源。

### 原始数据只增不改，可重放
访问器不解释内容。保存下来的原始数据逐字节保持原样，任何抓取都可以重放。识别与分类结果以字节区间引用原文，不修改原文；facet 以不可变版本追加；每一层派生记录血缘，可追溯到产出它的模块、版本与配置。

### 统一资源库 Strata
文本、结构化对象、图片、音视频统一为一个可寻址资源（RID），每个资源可有多种 facet（`raw`、`html`、`md`、`struct:article`、`image:cover` 等）。大对象内容寻址、自动去重，支持按区间读取。适配器没有写权限，所有写入经声明式变更集提交；游标、数据与任务确认在同一次提交中生效或都不生效。

### 过滤、全文、向量在同一查询中
类型化查询 DSL 融合元数据过滤、全文检索与向量检索，按一致快照读取，业务层不写 SQL。缺省部署只依赖单文件 SQLite，需要时切换 PostgreSQL。

### 出站集中治理
所有对外请求——包括无头浏览器发起的子请求——都经由出站治理发出：按域限速、自适应节流、主机熔断、会话与代理绑定、SSRF 防护、RFC 9309 `robots.txt` 处理。付费调用另有一层治理：逻辑请求键、预算窗口、幂等回执。

### 声明式配置，原子热加载
信源、访问器、适配器与策略预设全部是校验严格的 YAML 文件——未知字段校验报错，配置不存数据库。热加载按代际生成不可变快照并原子切换，在途任务保持派发时的代际。

### 面向 Agent 的只读表面
已采集数据通过 MCP 暴露：类型化查询、健康状态与配置溯源，供任意支持 MCP 的客户端接入；抽象类型（`Article`、`HotList`）提供业务视图。

## 架构

```text
配置(YAML) ─► 调度器 ─► 任务队列(Strata) ─► 任务执行
                                             │ 访问器（经出站治理 / 付费治理）
                                             ▼
                                        原始信封 ─► 适配器路由 ─► 搜索适配器 ─► 发现记录
                                                   └► 内容适配器 ─► facet
                                             │ 组装为变更集
                                             ▼
                          Strata（关系库 + 内容寻址 Blob）─► 抽象类型 / 下游消费者
```

| 组件 | 职责 |
|---|---|
| 调度器 | 按信源配置、节奏与健康状态生成访问任务 |
| 访问器 | 按协议取回原始数据，输出原始信封；不解析内容 |
| 出站治理 | 限速、熔断、robots、SSRF 防护、会话与代理 |
| 适配器 | 搜索适配器产出发现记录，内容适配器产出 facet；均无写权限 |
| Strata | 资源、facet、关系、身份、血缘、状态与任务队列的存取 |
| 抽象类型 | 基于 Strata 的业务视图（Article、HotList 等），自行维护 schema 与状态 |

边界约束：只有 Strata 访问存储；访问器的对外请求只能经由出站治理发出；运行期状态存放在 Strata，与只读配置分离。

### 运行形态

| 进程 | 说明 | 对外表面 |
|---|---|---|
| `modalprism-collector` | 一条命令初始化存储并进入采集循环 | 环回运维 HTTP `:17880`、正向代理 `:17881`、推送 `:17882` |
| `modalprism-console` | Rust BFF 同源聚合运维面并托管前端 SPA | HTTP，`127.0.0.1:17890` |
| `modalprism-agent` | 只读表面（查询 / 健康 / 配置溯源 / 类型视图） | MCP stdio，工具名 `instance.method` |

## 快速开始

### 构建

构建需要系统已安装 `protoc`（Protocol Buffers 编译器），`frame/port-grpc` 的构建脚本会调用它。Debian 与 Ubuntu 上的包名是 `protobuf-compiler`。

```sh
cargo build --workspace
```

以上为完整构建，无需额外参数。对象存储缺省走外部 RustFS 发行包（feature `rustfs-bin`）：先在配置路径、`<data>/tools/rustfs/<version>/`、`PATH` 里找二进制，都没有才下载预编译发行包，不需要编译 rustfs。完全离线时可把配置项 `strata.gateway.rustfs.offline` 写成 `true`，缺二进制会立即报错而不是等网络。

可选的后端替换（默认不需要；仅当明确要换存储形态时才开启）：

```sh
cargo build -p modalprism-collector --no-default-features   # 文件 BlobStore，无对象存储
cargo build -p modalprism-collector --features s3           # 外部 S3 兼容服务
cargo build -p modalprism-collector --features embedded-s3  # 不建议：进程内 RustFS（需编译 rustfs，构建耗时很长）
```

### 启动采集

在仓库根目录执行（默认 `--data var --config config`）：

```sh
cargo run -p modalprism-collector
```

监听地址可分别指定：`--http 127.0.0.1:17880 --proxy 127.0.0.1:17881 --push 127.0.0.1:17882`。收到 Ctrl-C 或 SIGTERM 时释放任务租约后退出。

### 启动控制台

前端为 Vite + React + TypeScript（pnpm 管理），先构建产物再启动 BFF：

```sh
cd apps/modalprism-console/web && pnpm install && pnpm build
cargo run -p modalprism-console -- --ops http://127.0.0.1:17880 --bind 127.0.0.1:17890
```

浏览器访问 `http://127.0.0.1:17890`，页面路由为哈希模式（`#/overview`、`#/sources`、`#/library`、`#/channels`、`#/config`）。

## 信源声明

信源是 `config/sources/*.yaml` 中的声明式入口，说明访问什么、使用哪些适配器、按什么节奏、采用哪套策略：

```yaml
id: source/openai-news
kind: source
name: "OpenAI News"
labels:
  tier: T1
access:
  accessor: accessor/rss
  locator:
    url: https://openai.com/news/rss.xml
pipeline:
  - run: [adapter/feed-items]
schedule:
  interval: 2h
admission:
  initialBackfill:
    limit: 8
    maxAge: 365d
```

配置合并顺序：通用配置 → 类型基础配置 → 策略预设 → Profile → 模块实例 → 环境覆盖，最后经系统级锁定检查得到不可变快照。

## 只读 MCP 表面

供外部 Agent 以 MCP 协议查询已采集数据：

```sh
cargo run -p modalprism-agent -- --data var --config config
```

## 仓库结构

```text
frame/                     LEAF 框架运行时与协议端口（随仓库提供，上游钉死提交）
SDK/                       LEAF 能力 SDK（Rust / Python / TypeScript）
apps/ability-core/         LEAF 能力宿主
apps/modalprism-domain/    strata-model · strata-sqlite-vec · strata-gateway · strata-client · config · types · surface
apps/modalprism-collector/ 采集进程（ability/ 为进程内能力；subsystems/ 为访问器、适配器、出站治理等）
apps/modalprism-agent/     只读 MCP 进程
apps/modalprism-console/   控制台（Rust BFF + web/ 前端 SPA）
abilitys/                  共享能力：strata_read · config_inspect · health · type_article · type_hotlist
config/                    系统配置、访问器、适配器、信源、策略预设与 schema（YAML / JSON）
docs/                      设计文档与实现规格
manual/                    LEAF 框架手册
```

依赖边界：领域库不依赖 `frame-core` / `ability-core` / `ability-support` 与任一 port；ability crate 只依赖 `ability-support`；组合根依赖 `frame-core` 与 `ability-core`。

## 开发与验证

测试与 clippy 门禁只覆盖本项目 crate；`frame/`、`SDK/` 与 `apps/ability-core` 为随仓库提供的 LEAF 框架代码，按上游状态构建。

```sh
cargo test --workspace --exclude 'frame-*' --exclude 'ability-support*' --exclude ai-protocol --exclude ability-core
cargo clippy --workspace --all-targets --exclude 'frame-*' --exclude 'ability-support*' --exclude ai-protocol --exclude ability-core -- -D warnings
```

工作区禁止 `unsafe_code`；release 配置 `lto = true`、`codegen-units = 1`。前端在 `apps/modalprism-console/web` 下以 pnpm 构建。

## 许可证

Apache-2.0
