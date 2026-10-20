# 内置访问器

本文固定 `http`、`rss`、`api`、`paid-api`、`search`、`push`、`file`、`browser`、`bundle-list` 的行为。契约见 `docs/specs/access-adapters.md`。设计第 6 节是模型。

实现在 `crates/accessors-builtin`。版本都是 `1`。`validate` 用 JSON 反序列化，未知字段类别为 `config`，消息 `spec 无效`。

访问器不抽取标题、条目、搜索命中或业务字段。部件字节只通过 `Staging::put` 进入信封。`redact_envelope` 在返回前执行。

调用开始时检查取消标志与截止时间，翻页前再检查一次。取消的消息是 `cancelled`。截止时间已到的类别是 `transient`，消息是 `deadline`。取消时不返回已经取到的页。截止时间在两页之间到达时，返回已经取到的页。

## 共同的 HTTP 响应

出站端口返回的字节按下面的规则进入信封。

`Content-Encoding` 为 `gzip` 时解码。其他非空编码类别为 `permanent`，消息 `encoding`。解码成功后去掉 `Content-Encoding`；若有 `Content-Length`，改成解码后的字节数。

`Content-Type` 去掉参数并转成小写，缺省 `application/octet-stream`。`charset` 写入部件，不转码。

状态 `304` 时 `not_modified` 为真，不写 `document` 部件。其他状态都写一个 `document` 部件，包括 4xx 与 5xx。

`Retry-After` 是整数秒或 HTTP 日期时写入 `signals.retry_after`。`Cache-Control` 的 `max-age` 写入 `signals.cache_max_age` 和 `signals.ttl`。

`transport.protocol` 取最终 URL 的方案，缺省 `http`。`final_locator` 是最终 URL。`bytes_in` 是解码后的字节数。

提示里的 `etag`、`last_modified` 分别变成 `If-None-Match`、`If-Modified-Since`。spec 已经写了同名头时不覆盖。

`spec.session` 是会话名。`SessionPool::get` 的头追加在 spec 头之后。返回了 `cookie` 且还没有 `Cookie` 头时写入 `Cookie`。不轮换会话。

`http` 的实际渲染级别：使用了会话为 `L1`，否则 `L0`。不执行脚本。

## http

id 是 `http`。定位符是 `url`，方案必须是 `http` 或 `https`。

spec 字段：`method` 缺省 `GET`，`headers`，`body`，`session`。一次调用一个信封。

## rss

id 是 `rss`。定位符与 `http` 相同。能力包含条件请求。

spec 字段：`headers`，`session`，`pages`。`pages` 缺省 10，范围 1 到 50。

每一页一个信封，正文是该页原文。非 2xx 的页仍然返回，分页在该页停止。

下一页只看两处，HTTP `Link` 优先：

- `Link` 头里 rel 含 token `next` 的尖括号 URL。
- 没有该头时，在正文前 65536 字节内查找 `rel="next"`、`rel='next'` 或 `rel=next`，并在其前后 300 字节内取 `href`。窗口两端落在字符中间时扩到完整字符。

相对 URL 按该页最终 URL 解析。下一页与已请求的 URL 相同，或 URL 无法解析时停止，并保留已取到的页。不读取条目标题、条目链接或全文。

## WebSub

订阅和回调不在 `access` 里。组合根持有监听套接字。

`subscribe` 经出站端口向 hub 发送 `POST`，`Content-Type` 为 `application/x-www-form-urlencoded`，正文是 `hub.mode=subscribe`、`hub.topic`、`hub.callback`。响应状态不在 200 到 299 时，类别 `permanent`，消息 `hub`。

`verify` 读取查询参数 `hub.mode`、`hub.topic`、`hub.challenge`。`hub.mode` 为 `subscribe` 或 `unsubscribe` 且 `hub.topic` 与登记的 topic 相同时，响应状态 200，正文是 `hub.challenge`。否则状态 404，正文为空。

`notify` 把 POST 正文当作一份 Feed 原文，产出一个 `document` 信封。不拆条目。

## api

id 是 `api`。定位符是 `apiEndpoint`，值是 `http` 或 `https` URL。

spec 字段：`method` 缺省 `GET`，`headers`，`body`，`session`，`pages`，`pagination`。`pages` 缺省 1，范围 1 到 50。

`pagination.type`：

| 类型 | 行为 |
|---|---|
| `offset` | 查询参数 `param`。第一页取游标 `offset`，没有则取 `start`（缺省 0）。下一页加 `step`。`step` 为 0 时 `spec 无效` |
| `cursor` | 查询参数 `param`。值来自游标 `token`，没有则不带该参数。响应 JSON 在 `pointer` 处是字符串时，写入该页的 `cursor_candidate.token` 并作为下一页的值。指针缺失或值不是字符串则停止 |
| `link` | 只跟 HTTP `Link` 的 `next`。游标 `url` 存在时从该 URL 开始 |

每一页一个信封。非 2xx 的页仍然返回，分页在该页停止。指针只取下一页 token，不映射其他字段。`offset` 把下一页起点写入 `cursor_candidate.offset`。`cursor` 写入 `cursor_candidate.token`。`link` 写入 `cursor_candidate.url`。响应不是 JSON 时视为指针缺失。

## paid-api

id 是 `paid-api`。定位符是非空的 `apiEndpoint`，不要求为 `http` URL。分页规则同 `api`，传输走 `PaidGateway::call`。

`spec.service` 为非空字符串。`spec.params` 是对象，缺省 `{}`。`spec.headers` 是二元组列表，缺省空，原样写入 `PaidCall.headers`。`method` 不是 `GET` 时写入 `params.method`。请求体放进 `PaidCall.body`。分页参数并入 `params`：`offset` 与 `cursor` 写入同名查询参数的键，`link` 把当前 URL 写入 `params.url`。调用的 `endpoint` 仍是定位符。

`capabilities.paid.service` 为 `spec`，表示服务名在每次请求的 spec 里。`batchable` 为真。

每个信封的 `transport.receipt_id` 是付费响应的回执。一次调用一个信封。不把响应拆成多个信源。

请求次数上限写在信源 `paid.budget`，由调度按窗口计数。预算错误原样返回。

## search

id 是 `search`。定位符是 `query`。

`spec.endpoint` 必须含有 `{query}`，其余部分是 `http` 或 `https` URL。`{query}` 替换为 RFC 3986 非保留字符以外的百分号编码，空格是 `%20`。一次 `GET`，一个信封。

`spec.headers` 与 `spec.session` 同 `http`。

经浏览器打开结果页时，信源使用 `browser`。`search` 不调用浏览器内核。

## push

id 是 `push`。`inbound` 为真。`access` 的类别是 `config`，消息 `inbound`。

组合根持有监听套接字，并调用 `ingest`。本 crate 不绑定端口。

spec 字段：`token` 非空，`ratePerSec` 缺省 1，`burst` 缺省 3。两者都至少为 1。

令牌放在 `Authorization: Bearer` 或 `X-Push-Token`。比较不因第一个不同字节提前返回。不匹配时类别 `permanent`，消息 `unauthorized`。

同一 channel 的令牌桶按 `ratePerSec` 补充，上限 `burst`。没有令牌时类别 `transient`，消息 `rate-limit`。

`Idempotency-Key` 缺省时每次请求一个新信封。键相同且正文 sha256 相同：在 24 小时内返回已保存的信封，不写入暂存。键相同而正文不同：类别 `permanent`，消息 `idempotency`。同一键的检查、扣减令牌和写入持有同一把锁。并发的相同键只产生一个信封。保存的键超过 1024 时丢掉最旧的一条。

信封定位符是 `pushChannel`。部件是一个 `document`，媒体类型取 `Content-Type`，缺省 `application/octet-stream`。

## file

id 是 `file`。协议是 `file`。

路径来自 `spec.path`，没有则来自定位符 `url`。`file://` 去掉该前缀。其余定位符类别为 `config`，消息 `locator`。

`spec.root` 存在时，路径必须位于该目录之内。比较使用规范化后的绝对路径。`..` 不能离开 root。没有 root 时路径必须是绝对路径。打开文件后读取该描述符。有 root 时，用 `/proc/self/fd` 的规范化路径确认描述符仍在 root 内；落在 root 外则类别 `permanent`，消息 `not-found`。

文件不存在的类别是 `permanent`，消息 `not-found`。

媒体类型来自 `spec.mediaType`，缺省 `application/octet-stream`。不按内容猜测类型。

不读取对象存储。对象存储的端点和凭据未规定。

## browser

id 是 `browser`。`render` 与 `layout` 为真。`capture_network` 为 `matched-body`。`interact` 为真，表示一次 `eval` 脚本。

定位符是 `http` 或 `https` URL。

访问器持有 `BrowserEngine` 和正向代理 URL。代理 URL 为空时，类别 `config`，消息 `proxy`。

`policy.capture.network` 为 `all-bodies`，或 spec 的 `mode` 为 `cdp`，或 `engine` 为 `chromium`，或写出了 `fallback`：类别 `config`，消息 `未实测`。`engine` 不是 `moli` 也不是 `chromium`，或 `mode` 不是 `auto`、`cli`、`cdp`：消息 `spec 无效`。这些判断先于出站请求。

其余任务：

1. 经出站端口对目标 URL 发送 `GET`。这一步只做 robots、限速、熔断和 SSRF。响应体不进入信封。出站错误则不调用内核。
2. 调用 `BrowserEngine::fetch`。请求里的 `proxy` 是访问器持有的代理 URL。`block_private_networks` 为真，不随 `egress.private_networks` 关闭。

spec 接受 `engine` 缺省 `moli`，`mode` 缺省 `auto`（`auto` 与 `cli` 相同），`js` 缺省真，`frames`，`layout`，`resources`，`eval`，`session`，`wait.until`。

`wait` 里出现 `selector`、`script`、`responseUrl`、`responseUrlRegex`、`responseJson` 或 `delay` 时，消息 `未实测`。`resources` 只接受 `image`、`font`、`media`，其他取值的消息是 `未实测`。

`policy.capture.screenshot` 为 `on-failure`、`on-render` 或 `always` 时，内核请求的 `layout` 为真。缺省、`never` 或其他取值不因此打开布局。引擎没有返回截图字节时不造部件。

`by_id` 返回除 `browser` 以外的访问器。`browser` 由 `BrowserAccessor::new` 绑定内核和代理 URL。

`policy.capture.renditions` 列出的 `markdown` 或 `semantic_tree_text` 有对应字节时写成 `rendition`，`meta.derivedBy` 为 `moli`。

实际渲染级别：请求了 `layout`、`eval` 或网络追踪时为 `L3`，否则 `L2`。

### 内核端口

`BrowserEngine::fetch` 返回结构化结果。访问器把它映射为信封：

| 结果 | 部件 |
|---|---|
| `html` | `document`，`text/html` |
| `body` 且没有 `html` | `document`，媒体类型取响应声明，缺省 `application/octet-stream` |
| `final_url`、`status`、`headers`、`redirect_chain`、`title` | `metadata`（JSON），并写入 `transport` 与 `final_locator` |
| `network` | `network-trace`，仅当 `policy.capture.network` 为 `trace` 或 `matched-body` |
| `matched` | `network-response`，仅当捕获至少为 `matched-body` |
| `screenshot` | `screenshot`，`image/png` |
| `pdf` | `print`，`application/pdf` |

`transport.engine.name` 为 `moli`。`version` 用结果里的版本字符串；没有则为 `unmeasured`。

`moli_args` 生成参数：`fetch`、URL、`--dump`、`json`、`--http-proxy`、代理、`--block-private-networks`。按请求追加 `--disable-js`、`--with-frames`、`--layout`、`--eval`、`--wait-until`、`--trace-network`、`--trace-matched-response-body`、`--image`、`--font`、`--media`，以及每个头的 `-H`、`Name: Value`。

`MoliCommand` 启动给定可执行文件并解析标准输出。JSON 只读取 `final_url`、`status`、`headers`、`redirect_chain`、`title`、`html`、`body_base64`、`network`、`version`。`headers` 是 `[名称, 值]` 数组。`network` 的原始 JSON 作为追踪字节。不从 `network` 里挑选响应体。标准输出无法按这些字段解析时，类别 `permanent`，消息 `engine`。

`metadata` 与 `network-trace` 在写入前按 `raw.redact_headers` 改写头值，命中的值是 `***`。追踪里键名为 `headers` 或以 `headers` 结尾的节点，以及全部由 `[名称, 值]` 组成的数组，使用同一名单。`transport.headers` 仍由 `redact_envelope` 改写。

进程退出状态非 0 时，类别 `permanent`，消息 `engine`。

## bundle-list

id 是 `bundle-list`。定位符是 `http` 或 `https` URL。先取该页。页面不是 2xx 时原样返回。2xx 时按 `extract` 顺序在 HTML 上抽取；没有条目时，从页面里的 `script src` 取最后 `maxScripts` 个（缺省 8，范围 1 到 20），从后往前请求。非 2xx 或超过 `maxScriptBytes`（缺省 1048576，上限 2000000）的脚本跳过。第一份抽出条目的脚本停止。

`extract` 非空。`quoted-fields` 从 `marker` 到 `endMarker` 之间读取对象，`fields` 是字段名，标题和链接都非空才保留。`route-record` 读取含 `routePath` 的对象，路径以 `routePrefix` 开头；`lang` 非空时还要相等；`skip` 中的路径丢弃。相对地址相对页面最终 URL 补全。标题、摘要和日期来自对象或 `frontmatter`。

返回一个信封，媒体类型 `application/json`，正文是 `{"items":[...]}`。传输错误向上返回。

## 未实测

下列各项是要求，本仓库的测试没有对真实内核执行它们：

- 安装并运行 `moli fetch` 或 `moli serve`，确认标准输出字段与上面的解析器一致。
- `capture.network=all-bodies` 的响应体覆盖度。
- CDP 或 WebDriver 会话、进程池、健康检查和回收。
- 真实内核的截图、PDF 和布局。
- Chromium 回退。
- 内核经正向代理发出的子请求是否都被网关看到。
- `--block-cidrs` 的具体参数。

测试使用夹具引擎和替身可执行文件。替身只证明参数中含有调用方给出的 `--http-proxy`，以及标准输出能映射成信封。夹具不是对 moli 输出的测量。

## 未闭合

- 端口访问控制
- embedding 正文形态和维度
- `Body::External` 的内容哈希
- 是否改用 CUE
- 多实例租约与 PostgreSQL
- Web Bot Auth 签名与会话轮换
- 对象存储
- 合并付费响应的归属
