# 信源目录

采集对象写在 `config/sources/`。取回字节、翻页、列表抽取、日期、字段路径、Feed 条目、链接规范化和身份键由访问器与适配器完成。站点文件只写地址、选择器、字段路径、节奏和准入。

访问器与适配器的字段见 `docs/specs/builtin-accessors.md` 和 `docs/specs/access-adapters.md`。调度与推送写入见 `docs/specs/assembly.md`。

## 能力

| 来源形态 | 访问器 | 适配器 |
|---|---|---|
| RSS / Atom | `accessor/rss` | `adapter/feed-items` |
| 网页列表、Markdown 列表、更新日志 | `accessor/http` | `adapter/html-list` |
| 详情页字段 | `accessor/http` | `adapter/page-fields` |
| 页面内嵌脚本中的列表 | `accessor/bundle-list` | `adapter/json-list` |
| JSON 接口 | `accessor/api` | `adapter/json-list` |
| 带 `{query}` 的查询地址 | `accessor/search` | `adapter/json-list` |
| 按次计费的 JSON 接口 | `accessor/paid-api` | `adapter/json-list` |
| 外部推送 | `accessor/push` | `adapter/json-list` |

`html-list` 的 `parseMode` 取 `html`、`markdown`、`docusaurus_changelog`、`intercom_changelog`。`publishedAtRegex` 写在条目 HTML 上，取第一个捕获组，再按日期规则解析。需要阅读器渲染页面时，把列表地址写成 `https://r.jina.ai/` 加上目标 URL，并在该信源的访问 spec 里写鉴权头。采集不会在抓取失败后自行改走阅读器。

列表 spec 可以写 `detail`。`maxFetches` 为 0 到 50，`fields` 取 `publishedAt`、`title`、`summary`。请求的字段在列表里缺失，且摘要没有作为正文时，产出 `Follow::Yes`，`needs` 列出缺失字段，访问器是 `accessor/http`。`maxFetches` 大于 0 时这些跟进入队，每轮不超过该上限。已经写入 `struct:page` 且所需字段非空的资源不占名额。跟进任务不改写信源的下次时间和游标。详情页由 `adapter/page-fields` 读取，结果写入 `struct:page`。

付费请求的 `headers` 写在信源 `access.spec`，格式与 HTTP 访问器相同，原样进入 `PaidCall`。密钥用 `${env:NAME}` 写在头或正文里。`enabled: false` 的模块保留 `${env:NAME}` 字面量，加载不因变量缺失失败。启用后变量缺失则整次加载失败。`paid.budget` 的计量单位未规定，装配不把它换成次数或金额。付费访问器上的 `paid.service` 与 `paid.budget` 只满足非空要求。

信源模块没有 `paid` 字段，因此使用 `accessor/paid-api` 的信源进入免费队列。付费队列只在信源写出 `paid` 时选用；当前模块字段表不接受该键。

`policy.robots.mode` 可以写在单个信源上。`ignore` 必须同时写 `robots_reason`，并且不写 `exempt_as`。

脚本里的列表用 `accessor/bundle-list`。它先读页面，抽不出条目时再取页面里的脚本地址。`extract` 按顺序尝试 `quoted-fields` 与 `route-record`，命中的第一份脚本停止。抽出的 JSON 交给 `adapter/json-list`。

## 目录中的信源

公开 Feed 使用 `accessor/rss` 与 `adapter/feed-items`。`accessor/rss` 的 `pages` 为 1，`Accept-Encoding` 为 `gzip, identity`。首次导入保留 8 条、365 天以内的条目。

| id | 名称 | 地址 | 间隔 |
|---|---|---|---|
| `source/openai-news` | OpenAI News | `https://openai.com/news/rss.xml` | 2h |
| `source/google-deepmind` | Google DeepMind | `https://deepmind.google/blog/rss.xml` | 2h |
| `source/google-research` | Google Research | `https://research.google/blog/rss/` | 2h |
| `source/hugging-face` | Hugging Face Blog | `https://huggingface.co/blog/feed.xml` | 2h |
| `source/microsoft-research` | Microsoft Research | `https://www.microsoft.com/en-us/research/feed/` | 2h |
| `source/nvidia-blog` | NVIDIA Blog | `https://blogs.nvidia.com/feed/` | 2h |
| `source/aws-machine-learning` | AWS Machine Learning Blog | `https://aws.amazon.com/blogs/machine-learning/feed/` | 2h |
| `source/github-ai` | GitHub Blog · AI & ML | `https://github.blog/ai-and-ml/feed/` | 2h |
| `source/mistral` | Mistral AI | `https://mistral.ai/news/rss` | 2h |
| `source/berkeley-ai-research` | Berkeley AI Research | `https://bair.berkeley.edu/blog/feed.xml` | 3h |
| `source/the-verge-ai` | The Verge · AI | `https://www.theverge.com/rss/ai-artificial-intelligence/index.xml` | 1h |
| `source/techcrunch-ai` | TechCrunch · AI | `https://techcrunch.com/category/artificial-intelligence/feed/` | 1h |
| `source/ars-technica-ai` | Ars Technica · AI | `https://arstechnica.com/ai/feed/` | 1h |
| `source/mit-technology-review-ai` | MIT Technology Review · AI | `https://www.technologyreview.com/topic/artificial-intelligence/feed` | 2h |
| `source/the-decoder` | The Decoder | `https://the-decoder.com/feed/` | 1h |
| `source/simon-willison` | Simon Willison | `https://simonwillison.net/atom/everything/` | 2h |
| `source/import-ai` | Import AI | `https://importai.substack.com/feed` | 3h |
| `source/latent-space` | Latent Space | `https://www.latent.space/feed` | 3h |

`source/the-decoder` 的 `policy.robots.mode` 为 `ignore`。该站点的 robots 禁止全部订阅路径，`robots_reason` 写明采集对象就是这条订阅地址。

`source/inbound-items` 监听推送通道 `items`。令牌写在该信源的 `access.spec.token`，缺省值只用于环回监听。正文是 JSON：`items` 数组，元素含 `title`、`url`，可选 `publishedAt` 与 `summary`。

`source/deepseek-updates` 读取 `https://api-docs.deepseek.com/zh-cn/updates/`。访问器是 `accessor/http`，适配器是 `adapter/html-list`，`parseMode` 为 `docusaurus_changelog`，`publishedAtUtcOffset` 为 `+08:00`。间隔 2 小时。首次导入保留 30 条、365 天以内的条目。

`source/mimo-blog` 读取 `https://mimo.xiaomi.com/`。访问器是 `accessor/bundle-list`。先按 `sectionTitle:"Blog"` 到下一个 `sectionTitle:` 抽取 `title`、`link`、`desc`；没有结果时再读 `routePath` 以 `/blog/` 开头、`lang` 为 `en` 的记录，跳过 `/blog/`、`/blog/index`、`/blog/blog1`。脚本最多 8 个，每个不超过 1048576 字节。抽出的条目交给 `adapter/json-list`。缺标题或缺发布时间时跟进详情页，每轮最多 30 次。详情选择器是 `time.hero-date` 的 `datetime`、`h1.hero-title` 和 `p.hero-subtitle`。间隔 2 小时。首次导入保留 30 条、365 天以内的条目。

`source/status-search` 与 `source/wechat-account` 的 `enabled` 为 false。前者请求 `https://api.socialdata.tools/twitter/search`，查询参数为 `query` 与 `type`，条目取 `tweets`，标题和摘要取 `full_text`，地址由 `user.screen_name` 与 `id_str` 组成。`summaryIsBody` 为真，摘要写入 `text` facet。请求头是 `Authorization: Bearer ${env:SOCIALDATA_API_KEY}`。启用时设置该环境变量，把 `query` 里的 `from:example` 换成目标账号，再把 `enabled` 改为 true。后者向 `https://www.dajiala.com/fbmain/monitor/v3/post_history` 发送 POST，正文是 `{"ghid":"gh_example","key":"${env:DAJIALA_KEY}","verifycode":""}`。条目取 `data`，时间字段 `post_time` 的单位是 `epoch_s`。启用时设置 `DAJIALA_KEY`，把 `ghid` 换成目标账号，再把 `enabled` 改为 true。密钥留在环境里，不写入配置文件。
