# 访问器与适配器

本文固定阶段 4 的访问契约、路由和纯计算适配器。设计第 6、7 节是模型，这里只写实现必须遵守、且设计没有展开的规则。

出站与付费在本阶段是端口。限速、熔断、robots 执行和预算不在 `crates/accessors` 或 `crates/adapters` 中。

## 访问器

`Accessor` 一次调用返回有序的 `RawEnvelope` 列表。页序与列表序相同。

```text
id、version、capabilities、validate(spec)
access(request, context) -> Vec<RawEnvelope>
```

`AccessRequest` 携带任务、信源、定位符、访问器 `spec`、游标、提示和截止时间。提示包含 `etag`、`last_modified`、`render_level`。`policy` 是有效策略的 JSON 对象，字段名与配置快照一致。

`AccessContext` 携带四个端口和取消标志：

| 端口 | 调用 |
|---|---|
| `EgressGateway` | `request(method, url, headers, body) -> status, headers, body, final_url` |
| `PaidGateway` | `call(service, endpoint, params, body) -> status, headers, body, receipt_id` |
| `SessionPool` | `get(profile) -> headers, cookie` |
| `Staging` | `put(bytes) -> PartRef` |

取消标志是 `Arc<AtomicBool>`。标志为真时访问器返回类别 `transient`、消息 `cancelled`。

`redact_envelope` 只改写 `transport.headers`。名称来自 `policy.raw.redact_headers`，按 ASCII 大小写不敏感匹配，值替换为 `***`。顺序和未列出的头保持不变。

内置访问器的行为见 `docs/specs/builtin-accessors.md`。

## 适配器输出

搜索适配器返回 `DiscoverOutput`：有序的 `Discovery` 和可选的 `Feedback`。内容适配器返回 `ContentOutput`：facet 草稿、链接草稿和可选的 `Feedback`。

`Discovery.identity` 是身份键字符串。绝对 URL 先按基准 URL 解析，再使用序列化结果组成 `url:` 键。解析失败的条目跳过。本阶段不删除查询参数，不删除 fragment。

`Follow::Yes` 携带访问器 id、`auto | never | always` 和优先级。条目带 `materialize` 时跟进为 `Follow::No`。

`Feedback.quality` 取 `ok`、`thin`、`blocked`、`empty`。`needs_render` 取 `L0` 到 `L4`。

适配器 trait 上没有写入方法。`deterministic == false` 的适配器拒绝执行，类别为 `config`。内置适配器的 `deterministic` 均为 `true`，版本均为 `1`。

`params_hash` 是适配器收到的 spec JSON 字节的 sha256。识别产出的 `params_hash` 是检测器 id 字节的 sha256。

## 路由

管道规则的字段是 `when`、`run`、`run_if_accepts`、`for`、`after`、`spec`。`when` 的字段是 `part_role`、`declared`、`detected`、`traits`。其他键使本次运行失败。

适配器 id 与配置模块 id 相同，例如 `adapter/feed-items`。

`for` 缺省时匹配 `Follow` 以外的任务原因。`for: follow` 只匹配 `Follow`。其他取值失败。

`when` 中出现的键全部满足才命中。媒体类型先去掉参数并转成小写再比较。`traits` 列出的特征都要存在。

匹配按下面的顺序：

1. 用部件角色、声明类型和 `for` 选出候选规则。此步不读取识别结果。
2. 候选规则里出现 `detected` 或 `traits` 时，对这份 raw 输入执行一次识别。没有这样的规则时不识别。点名 `adapter/detect` 不会再执行第二次。
3. 用识别结果收紧候选。`consumes` 里的 `media_type` 与声明类型或识别类型之一相同即可。`kind` 为空或 `raw` 才匹配原始部件。
4. `run_if_accepts` 调用 `probe`。返回 `None` 或 `accept == false` 的适配器不进入计划。只看 `accept`。

同一部件上同一适配器只保留第一次。`after` 只连接同一部件：上游不在计划中则去掉下游。依赖成环则失败。派生输入按该正文的 kind 与媒体类型匹配 `consumes`。

同一层按部件下标、再按适配器 id 依次执行。`after` 的输入是上游第一个文本或结构正文。结构正文以 JSON 字节交给下游，媒体类型为 `application/json`。

识别结果按 `part_id` 分发给后续适配器。

`policy.classify.detector` 取值 `basic`、`off` 或 `plugin:<id>`。`off` 时不识别，引用识别结果的规则不命中。未知插件使本次运行失败。

## 变更集

运行结果是一份 `ChangeSet`、跟进列表和各适配器的 `Feedback`。不写入游标、确认或健康状态。

准入在 `ensure` 之前，顺序为：允许前缀、拒绝前缀、噪声词、`publishedAfter`、首次导入的 `maxAge`、同一身份键去重、首次导入的 `limit`。前缀和噪声词作用在定位符文本、标题和摘要上：允许前缀非空时定位符文本必须以其一开头；拒绝前缀命中则去掉；噪声词按大小写不敏感的子串匹配标题或摘要。`publishedAfter` 与 `maxAge` 只在写出了发布时间时生效。`limit` 与 `maxAge` 只在首次导入时生效。墓碑不占用 `limit`。

身份键按书写顺序查看：

- 任一键是墓碑：整条发现丢弃。
- 否则任一键已存在：目标用第一条已存在的 RID。写入 `struct:discovery` 和 `discovered_by`。不写 `materialize`，不生成跟进。
- 否则：`ensure` 使用第一条键，别名 `d0`、`d1` 按保留顺序。写入 `struct:discovery` 和 `materialize`。`Follow::Yes` 生成跟进。

`discovered_by` 的 `src` 是发现目标，`dst` 是 `origin`。`origin` 缺省时用 `resource`。两者都缺省时不写这条关系。

内容适配器的 facet 写到 `resource`。本次有内容产出但 `resource` 缺省时失败。正文只使用 `text` 或 `struct`。

`struct:discovery` 的 variant 是 `discovery`。字段为 `locator`、`identity`、`hints`。`hints` 只写出有值的 `title`、`publishedAt`、`author`、`summary`、`rank`、`extra`。列表、Feed 条目和链接发现的 `rank` 从 1 起，按保留顺序编号。

草稿没有 `derived_from`、且输入带有 facet 引用时，血缘输入是该引用的 `[0, 字节长度)`。下游输入没有 facet 引用时，血缘输入为空。

## 内置检测器 `detector/basic`

类型判定只看前 65536 字节。特征与区间扫描全文，超过 1 MiB 的部分不参与。

字符集：BOM 优先，其次是 HTML 的 `charset=` 或 XML 声明的 `encoding`。

媒体类型按第一条命中：

| 条件 | 类型 | 置信度 |
|---|---|---|
| `%PDF-`、PNG、JPEG、`GIF87a`/`GIF89a`、`RIFF`+`WEBP`、`PK\x03\x04`、gzip | 对应的 PDF、PNG、JPEG、GIF、WEBP、ZIP、gzip | 0.95 |
| 第一个元素本地名 `rss` / `feed` / `html` | `application/rss+xml` / `application/atom+xml` / `text/html` | 0.95 |
| 前缀含 `<!doctype html` 或 `<html` | `text/html` | 0.95 |
| 其他以 `<` 开头的元素 | `application/xml` | 0.95 |
| 去掉空白后以 `{` 或 `[` 开头 | `application/json` | 0.9 |
| 有声明类型 | 声明类型 | 0.5 |
| 其余 | `application/octet-stream` | 0.2 |

`text/html` 附加特征，名称排序后写入：

- 去掉 `script`/`style` 及其内容、再去掉标签、折叠空白后，可见文本少于 40 个字符：`shell-page`
- 含 `cf-challenge`、`captcha`、`attention required`、`just a moment` 之一：`challenge-page`
- 含 `application/ld+json`：`has-jsonld`
- 含 `<img`：`has-images`

区间只产出 `json-ld`：从该标记之后的 `>` 到 `</script>`，偏移是原文字节下标。JSON 无法解析的脚本跳过。置信度 0.9。

`adapter/detect` 产出 `struct:detect`。顶层 `mediaType`、`charset`、`confidence`、`traits` 取 `document` 部件，没有则取第一个部件。`parts` 列出每个部件。存在区间时另产出 `regions:detect`。

## 内置适配器

| id | 角色 | 行为 |
|---|---|---|
| `adapter/feed-items` | 搜索 | RSS 2.0 与 Atom 1.0。规则见「列表与 Feed」 |
| `adapter/feed-fulltext` | 内容 | 一份 `md` 和一份 `text`。顺序是频道标题和各条目的标题与正文 |
| `adapter/html-list` | 搜索 | HTML、Markdown 与更新日志。规则见「列表与 Feed」 |
| `adapter/json-list` | 搜索 | JSON 数组映射。规则见「列表与 Feed」 |
| `adapter/readability-md` | 内容 | 去掉 `script`、`style`、`noscript`、`nav`、`footer`、`header`、`form`、`iframe`、`svg`。根节点优先 `article`，然后 `main`、`[role=main]`、`body`。产出 `md:readable` 与 `text`。可见文本为空：`empty` 且 `needs_render=L2`。少于 80 个字符：`thin` 且 `needs_render=L2`。输入特征含 `shell-page` 或 `challenge-page`：`blocked` 且 `needs_render=L2` |
| `adapter/link-discovery` | 搜索 | 收集 `a[href]`，解析为 `http`/`https`，同一身份键只保留第一条 |
| `adapter/jsonld-struct` | 内容 | 每个可解析的 `application/ld+json` 脚本进入 `struct:jsonld` 的 `items`。脚本值是数组时展开 |

选择器无法解析、必填 spec 缺失，类别为 `config`。

零条 Feed 或 JSON 列表的 `quality` 为 `empty`，不请求渲染。HTML 模式零条时 `needs_render=L2`。Markdown 与更新日志零条时不请求渲染。

## 列表与 Feed

`adapter/feed-items` 读 RSS 2.0 与 Atom 1.0。定位符取 RSS `link`，或 Atom `rel=alternate` 的 `href`，否则取第一条 `href`。身份键先 `url:`，若 `guid` 或 Atom `id` 不含空白则再加 `ext:guid:`。有 `content:encoded` 或 Atom `content` 时作为 `html` facet 放入 `materialize`，且不跟进。摘要去掉标签。无法解析的 XML 是永久错误。

可选 spec：

| 字段 | 行为 |
|---|---|
| `allowCategories` | 非空时，条目的 `category` 文本或 Atom `term` 至少有一项与名单大小写不敏感相等 |
| `denyCategories` | 命中任一项则丢弃。先于允许名单 |
| `summaryIsBody` | 为真且没有 `content` 时，把去标签后的摘要作为 `text` facet 放入 `materialize`，且不跟进 |
| `publishedAtUtcOffset` | `+08:00` 这种偏移。纯日期按 UTC 零点。没有时区的日期时间使用该偏移，缺省 `+08:00` |

YouTube、youtu.be、Vimeo 的播放页不把摘要写入正文，也不跟进。已有 `content` 仍写入 `html` facet。

`adapter/html-list` 的 `parseMode` 缺省 `html`。`html` 必填 `item` 与 `link`，可选 `title`、`summary`、`publishedAt`、`baseUrl`、`minTitle`。`publishedAt` 是选择器字符串或 `{ selector, attr }`。未写 `attr` 时依次读 `datetime`、`content`、`title` 和文字，先读到能解析的值。链接相对 `baseUrl`，缺省相对 `locator`。与列表同一路径和查询的链接跳过。同一身份键保留较长的标题。

`markdown` 读取 `[标题](地址)`。`minTitle` 缺省 1，按字符数。`docusaurus_changelog` 读取 `article` 或 `.markdown` 内带 `id` 的 `h2`/`h3`；整段标题是日期时，该日期作用于后续条目，直到下一个非日期 `h2`。`intercom_changelog` 读取 `article` 内带 `id` 的 `h2`/`h3`，标题能解析为日期时，后续兄弟节点的文字作为摘要和 `text` facet，且不跟进。日期标题的偏移缺省 `+00:00`。条目地址是列表地址加上标题 `id` 的 fragment。

`adapter/json-list` 用 `items`（JSON Pointer）或 `itemsPath`（点路径；空字符串表示根数组）定位数组。`url` 是条目上的键，相对 `locator` 补全。`urlTemplate` 用 `{路径}` 做百分号编码、`{raw:路径}` 原样插入，结果必须已是 `http` 或 `https` 绝对地址。`title`、`summary`、`author` 可以是键或键数组；`titlePaths`、`summaryPaths`、`authorPaths` 按顺序取第一个非空标量。`publishedAt` 或 `publishedAtPath` 配合 `publishedAtUnit`：`epoch_s`、`epoch_ms`、`yyyymmdd`。`externalIdPath` 追加 `ext:` 身份键。`decodeEntities` 为真时，标题和摘要把 `&amp;` `&lt;` `&gt;` `&quot;` `&#39;` 换成原字符。`summaryIsBody` 为真时，摘要作为 `text` facet 放入 `materialize`，且不跟进。路径不是数组或没有映射出条目时结果为空。

## 相同输入

同一定义适配器对相同字节、相同 spec 和相同身份视图的两次运行，变更集与跟进列表相同。运行不生成 RID。
