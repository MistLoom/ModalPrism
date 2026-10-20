# 信源目录

采集对象写在 `config/sources/`。取回字节、翻页、列表抽取、日期、字段路径、Feed 条目、链接规范化和身份键由访问器与适配器完成。站点文件只写地址、选择器、字段路径、节奏和准入。

访问器与适配器的字段见 `docs/specs/builtin-accessors.md` 和 `docs/specs/access-adapters.md`。调度与推送写入见 `docs/specs/assembly.md`。

## 能力

| 来源形态 | 访问器 | 适配器 |
|---|---|---|
| RSS / Atom | `accessor/rss` | `adapter/feed-items` |
| 网页列表、Markdown 列表、更新日志 | `accessor/http` | `adapter/html-list` |
| JSON 接口 | `accessor/api` | `adapter/json-list` |
| 带 `{query}` 的查询地址 | `accessor/search` | `adapter/json-list` |
| 按次计费的 JSON 接口 | `accessor/paid-api` | `adapter/json-list` |
| 外部推送 | `accessor/push` | `adapter/json-list` |

`html-list` 的 `parseMode` 取 `html`、`markdown`、`docusaurus_changelog`、`intercom_changelog`。需要阅读器渲染页面时，把列表地址写成 `https://r.jina.ai/` 加上目标 URL，并在该信源的访问 spec 里写鉴权头。采集不会在抓取失败后自行改走阅读器。

付费传输把 `params` 写成查询参数，不附加鉴权头。密钥放在信源 `access.spec` 的查询参数或正文里，用 `${env:NAME}` 引用环境变量。`paid.budget` 的计量单位未规定，装配不把它换成次数或金额。付费访问器上的 `paid.service` 与 `paid.budget` 只满足非空要求。

信源模块没有 `paid` 字段，因此使用 `accessor/paid-api` 的信源进入免费队列。付费队列只在信源写出 `paid` 时选用；当前模块字段表不接受该键。

`policy.robots.mode` 可以写在单个信源上。`ignore` 必须同时写 `robots_reason`，并且不写 `exempt_as`。

发现结果里的 `Follow` 不入队。列表没有自带正文、且 `summaryIsBody` 不为真时，不另发详情请求。

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

`source/html-listing` 与 `source/json-listing` 的 `enabled` 为 false。它们声明网页列表和 JSON 列表的字段，地址是 `https://example.com/` 下的示例路径，不作为线上目标。

`source/status-search` 与 `source/wechat-account` 的 `enabled` 为 false。前者请求 `https://api.socialdata.tools/twitter/search`，查询参数为 `query` 与 `type`，条目取 `tweets`，标题和摘要取 `full_text`，地址由 `user.screen_name` 与 `id_str` 组成。 `summaryIsBody` 为真，摘要写入 `text` facet。后者向 `https://www.dajiala.com/fbmain/monitor/v3/post_history` 发送 POST，条目取 `data`，时间字段 `post_time` 的单位是 `epoch_s`。两份声明都不含密钥。
