# 调度与治理

本文固定出站网关、付费网关、节奏、健康、Planner 和 Runner。设计第 6.7、6.8、9 节是模型，这里只写实现必须遵守、且设计没有展开的规则。

组合根的循环不在这些 crate 里。`tick`、`run`、正向代理的 `serve` 由调用方触发。内置访问器不在这里实现。端口上不增加鉴权。

## 控制热数据

出站、付费、节奏和健康的热数据放在控制库 `hot`。进程内用互斥锁包住同一键的读改写。

| kind | key | 值 |
|---|---|---|
| `egress` | `state` | 令牌桶、熔断、robots 缓存、豁免审计 |
| `paid` | `state` | 占位回执与预算计数 |
| `planner` | `state` | 信源节奏、健康、日产出、快照水位 |

任务队列仍用 `tasks`。调度读游标用 `accepted_state("cursor", source_id)`。对外读取用 `state`，只看已可见快照。

## 出站

`Gateway::request` 实现 `EgressGateway`。对外 HTTP 只经过这个入口或正向代理。

顺序：解析 URL、每一跳 SSRF、robots、熔断、令牌桶，然后交给传输。传输、解析和拨号都由调用方注入。

只接受 `http` 和 `https`。URL 带 userinfo 时拒绝，类别 `permanent`，消息 `ssrf`。相对重定向按当前 URL 解析。最多 5 次重定向，用尽后类别 `transient`，消息 `redirect`。`303` 改为 `GET` 并清空正文。带 `Authorization` 或 `Cookie` 的请求只跟随同源重定向；跨源时类别 `permanent`，消息 `redirect-origin`。同源是 scheme、主机和端口都相同，缺省端口按 scheme 补成 80 或 443。

### SSRF

每一跳都解析主机的全部地址。任一地址落在私网或保留地址内，且 `private_networks` 为 `deny` 时，类别 `permanent`，消息 `ssrf`。`allow` 时这些地址可以连接。空解析结果的消息是 `dns`，类别沿用解析器返回的类别；解析器未给出时为 `permanent`。

拒绝的地址：

- IPv4：`0.0.0.0/8`、`10.0.0.0/8`、`100.64.0.0/10`、`127.0.0.0/8`、`169.254.0.0/16`、`172.16.0.0/12`、`192.0.0.0/24`、`192.0.2.0/24`、`192.168.0.0/16`、`198.18.0.0/15`、`198.51.100.0/24`、`203.0.113.0/24`、`224.0.0.0/4`、`240.0.0.0/4`
- IPv6：未指定、环回、`fe80::/10`、`fc00::/7`、多播、`2001:db8::/32`
- IPv4 映射地址按内嵌 IPv4 判断

连接按解析顺序使用第一个通过检查的地址。全部拨号失败时类别 `transient`，消息 `connect`。

### 注册域与令牌桶

令牌桶的键默认是注册域（eTLD+1，公共后缀表）。没有注册域时用小写主机。IP 主机用字面量。`host_rates` 里写出的主机单独使用自己的桶，键是小写主机。

缺省速率每秒 1 个、突发 3、并发 2。令牌按毫秒补充，上限是突发。取到 1 个令牌且在途数低于并发才发送。在途数在响应或传输失败后减 1。

截止时间已到仍没有令牌，或并发已满，类别 `transient`，消息 `rate-limit`。并发已满且未到截止时间时等待 50 毫秒再试。

`429` 或 `503` 把该桶速率减半，下限每秒 0.1 个。`2xx` 且本次耗时低于 1 秒时，速率向配置值回升每秒 0.1 个，不超过配置速率。

`Retry-After` 接受增量秒数或 HTTP 日期。合法时写入该桶的 `not_before`。未到该时刻的请求不发送，类别 `transient`，消息 `retry-after`。

### 熔断

熔断键是小写主机，不含端口。传输失败、状态码 `429` 和 `5xx` 记为失败，其余响应记为成功。连续失败达到 5，或最近 20 次里失败不少于 10 次，打开熔断。冷却从 30 秒起，每次再次打开翻倍，上限 30 分钟。打开期间不发送、不增加失败计数，类别 `transient`，消息 `circuit-open`。冷却结束后的成功把连续失败和打开次数清零，冷却回到 30 秒。

### robots

缓存键是 `scheme://host:port`，端口总是写出来。正文缓存 `robots.cache_ttl`。获取 `robots.txt` 时走 SSRF、熔断和令牌桶，不检查 robots。响应为 301、302、303、307、308 且带 `Location` 时按普通重定向跟随，最多 5 跳。跟随结束后状态仍是这些码，或跳数用尽时，按不可达处理，空正文不作为允许。

`ignore` 不检查。`obey` 始终检查。`obey-with-exemptions` 且本次请求带了豁免类别时不检查，并向 `audits` 追加 `origin`、`category`、`reason`、`at`。

分组按空行切开。`User-agent` 与请求 `User-Agent` 的第一个产品标记做 ASCII 大小写不敏感匹配；没有产品标记时只用 `*`。命中多个分组时取产品标记更长的一组。都未命中则允许。

`Allow` 与 `Disallow` 按路径前缀匹配。`*` 匹配任意长度，规则末尾的 `$` 要求路径在此结束。命中的规则取路径模式更长的一条；长度相同用文件中更早的一条。空的 `Disallow` 表示允许。

不允许时类别 `permanent`，消息 `robots`。Runner 确认该任务，不计入健康失败，`next_at` 按正常节奏。

`on_unreachable`：`rfc9309` 在 robots 响应为 4xx 时允许，5xx 或传输失败时全部禁止；`allow` 允许；`disallow` 全部禁止。不可达结果同样缓存 `cache_ttl`。

`Crawl-delay` 取所命中分组里最后一条，单位是秒。`honor` 使用该值，`ignore` 不用，`cap` 使用该值与上限的较小者。同一 origin 在间隔未到时不发送，类别 `transient`，消息 `crawl-delay`。

请求未带 `User-Agent` 时写入策略里的 `user_agent`。写出了 `contact` 且没有 `From` 时写入 `From`。不附加 Web Bot Auth 签名。

### 正向代理

`serve` 在调用方给定的监听套接字上接受连接。

- `CONNECT host:port`：按主机做 SSRF、熔断和令牌桶，不检查 robots。通过后回复 `200`，双向复制字节。
- 绝对形式的 `GET` 或 `HEAD`：与库调用相同，但不检查 robots。

其他请求回复 `405`。拒绝时回复 `403`。单条请求头不超过 16 KiB。

## 付费

`Gateway::call` 实现 `PaidGateway`。

逻辑键是 `sha256` 的十六进制，输入为紧凑 JSON：`service`、`endpoint`、规范化 `params`、`window`。对象的键按字典序递归排序。数组保持原顺序。`window` 是 Unix 秒向下取整到窗口长度后的起点。窗口长度缺省 24 小时。

同一个键在窗口内只真实发送一次。已有 `ready` 或 `completed` 时直接返回保存的响应，不增加预算。已有未过期的 `pending` 时不发送，类别 `transient`，消息 `in-flight`。`pending` 超过 60 秒仍无响应则改为 `unknown`。

发送前在同一次读改写里检查预算并写入 `pending`，然后才调用传输。预算按服务计数，窗口为 UTC 的分钟、小时、日。某一级达到上限时不写入占位，类别 `budget`，消息 `预算不足`。`retry_at` 是所有已满窗口终点里最晚的一个。未给出的级别不限制。重放不计数。真实发送后预算不退回。

传输返回 HTTP 响应时，占位改为 `ready` 并保存状态、头和正文。传输错误改为 `unknown`。

`unknown_receipt` 为 `manual` 时，后续调用类别 `permanent`，消息 `unknown-receipt`，不发送。`retry-once` 在 `unknown` 满 1 小时后放行一次；未满时类别 `transient`，消息 `unknown-wait`，Runner 把任务延后到冷却结束且不计入健康。这次仍未知则改为 `manual`。`unknown-receipt` 不计入健康失败。Runner 确认任务，`next_at` 按正常节奏。

完成的回执由 Runner 写入变更集：`setState`，`scope=receipt`，键是 `receipt_id`，值含 `status=completed`、`service`、`logicalKey`。`logicalKey` 是付费网关按服务、endpoint、规范参数和窗口算出的逻辑键。控制库里的行在 `apply` 被接受后标为 `completed`。

`batchKey` 只影响 Planner 的成组入队。一次付费调用产出一个信封。把合并响应拆回多个信源的归属规则未规定，访问器不拆分响应。

## 节奏

```text
y_per_day = yield_7d / 7
adaptive  = ceiling                         当 yield_7d = 0
adaptive  = 1440 / (y_per_day × 3) 分钟     其他
signal    = max(ttl, retry_after, cache_max_age, expires)   缺省项不参与
floor_eff = max(floor, 1h)                 付费访问器
floor_eff = floor                           其他
interval  = clamp(max(adaptive, signal, floor_eff), floor_eff, ceiling)
```

`adaptive` 为假时，`interval` 用信源的 `interval`，再夹在 `floor` 与 `ceiling` 之间（两者都写出时）。

7 日产出只计调用方记入的首次发现。回填由调用方不调用 `note_yield`。产出按 UTC 日存在 planner 状态里。日期变化时重算 `interval`，不改尚未到期的 `next_at`。成功后用本次信号立刻重算，并把 `next_at` 写成现在加新间隔。

## 健康

主体是 `source/<id>`、`accessor/<id>`、`adapter/<id>`。

| 状态 | 何时写入 |
|---|---|
| `ok` | 成功且质量为 `ok`，同时不满足 suspect 条件 |
| `degraded` | 瞬时错误 |
| `failing` | 永久错误，且该错误计入健康 |
| `suspect` | 质量为 `thin`、`blocked`、`empty`；或选择器未命中；或距上次计入产出的时间超过 `expected_period`；或 7 日产出 / 前 28 日日均 < `yield_drop_ratio`；或最近拦截样本不少于 5 且比例 ≥ 0.5 |
| `misconfigured` | 配置错误 |
| `paused` | 调用方写入，调度不改它 |

`consecutive_failures` 按 `transient`、`permanent`、`config` 分开计数。计入健康的连续失败达到 `failure_threshold` 时 `alert` 为真。预算、`robots`、`unknown-receipt`、`unknown-wait`、`shutdown` 不增加计数，也不改变状态。

前 28 日产出为 0 时不算骤降。`expected_period` 没有历史产出时不触发。`degraded-yield` 不退避。

永久错误把该信源间隔设为 `ceiling`，`next_at` 为 24 小时后，任务留在队列里等待探测。配置错误把 `next_at` 清空。信源配置代际变化后，`misconfigured` 恢复为 `ok`，`next_at` 为现在。

`snapshot` 在距上次提交已满 `snapshot_interval` 时，为每条健康和节奏生成 `setState`（scope 为 `health` 或 `cadence`）。`commit_snapshot` 之后才推进水位。

## Planner 与 Runner

`tick(sources, capacities, now)` 选出 `enabled` 且 `next_at` 为空或 `next_at <= now` 的信源。同一 `batchKey` 任一成员到期时，该组未在途的成员一起入队。排序是优先级降序、`next_at` 升序。

优先级：`manual` 与 `replay` 为 30，`follow`、`detail`、`render_upgrade` 为 20，`schedule` 为 10。

每个队列本次新入队数不超过 `容量 - 未过期租约数`。租约时长是信源 `timeout` 的 1.5 倍。入队时把 `next_at` 设为 `now + 租约时长`。`tick` 随即 `lease` 并返回领到的任务。未完成的同一 `(source, locator)` 不会有第二行；定位符键是 `Locator` 的 JSON。该行处于 `queued` 且 `retry_at` 为空或不晚于现在，或租约已到期时，`tick` 重新领取原任务。`retry_at` 仍在未来或租约未到期时不领取。重新领取不增加付费预算计数。

写出 `paid` 的信源进入付费队列。`paid.budget.requests` 是每个 `paid.budget.window` 内允许派出的新计划任务数，计量单位是请求次数。窗口起点是 Unix 毫秒向下对齐到 `window`。派出时 `budgetUsed` 加一。次数达到上限后，本窗口余下的扫描跳过该信源，`skipReason` 为 `budget`，`next_at` 写到下一窗口起点。进入下一窗口后 `budgetUsed` 归零，`skipReason` 清空。`budget` 不写入健康失败。

`run` 用 accepted 游标。游标是对象且 `configHash` 与当前信源配置哈希不同时，交给处理函数的游标为 null。处理函数成功后，Runner 把数据操作、游标 `setState`、完成回执 `setState` 和 `ack` 放进同一个变更集再 `apply`。游标对象写入当前 `configHash`。任一环节失败时不调用 `apply`，游标保持原值。

错误类别缺省按瞬时。瞬时错误在任务内立即重试 1 次，且不超过截止时间；这次重试不增加 `attempt`。仍失败则 `nack`。`nack` 把 `attempt` 加 1。外层延迟是 `max(Retry-After, min(6h, interval × 2^attempt))`。

`shutdown` 调用 `release_leases`，并把这些任务对应信源的 `next_at` 设为现在。不记失败，不增加 `attempt`。`release_leases` 在同一事务里把任务改回 `queued`，列上的 `lease_until` 置空，payload 里的 `leaseUntil` 写成释放时刻。下一次 `tick` 重新领取这些任务。

## 未闭合

- 端口访问控制
- embedding 正文形态和维度
- `Body::External` 的内容哈希
- 是否改用 CUE
- 多实例租约与 PostgreSQL
- Web Bot Auth 签名与会话轮换
