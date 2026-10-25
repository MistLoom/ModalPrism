// 配置字段的语义化名称与说明。
// 键是 config_inspect.explain 返回的点分路径；命中顺序：完整路径 → 任意后缀 → 回退最后一段。
// 说明文案写给第一次接触声明文件的人，不出现内部术语。

export interface FieldMeta {
  label: string;
  hint?: string;
}

const PATHS: Record<string, FieldMeta> = {
  // ── 通用模块字段 ─────────────────────────────────────
  id: { label: "模块标识", hint: "配置内的唯一 ID，如 source/openai-news、accessor/http" },
  kind: { label: "模块类型", hint: "source（信源）/ accessor（通道）/ adapter（解析器）/ type（内容类型）/ preset（策略预设）之一" },
  name: { label: "显示名称", hint: "模块在界面上的名字，不填则用 id" },
  description: { label: "说明", hint: "给人看的描述，会出现在信源与配置抽屉里" },
  enabled: { label: "是否启用", hint: "关闭后不再参与采集与调度" },
  labels: { label: "标签", hint: "用于筛选与归组的键值对，如 tier=t1" },
  "labels.tier": { label: "分级标签", hint: "信源优先级约定值，如 T1、T2" },
  "labels.topic": { label: "主题标签", hint: "信源主题约定值，如 官方、研究、媒体" },
  requires: { label: "依赖声明", hint: "运行前必须可用的其它模块" },
  extends: { label: "继承预设", hint: "在列出的预设基础上叠加本文件的政策覆盖" },

  // ── 信源 ────────────────────────────────────────────
  "schedule.interval": { label: "采集节律", hint: "两次采集之间的最小间隔，如 90s、30m" },
  "schedule.cadence": { label: "采集节律", hint: "两次采集之间的最小间隔" },
  "access.accessor": { label: "访问通道", hint: "用哪个采集通道出站访问" },
  "access.locator": { label: "定位信息", hint: "告诉通道去哪里取内容" },
  "access.locator.url": { label: "访问地址", hint: "采集目标的 URL" },
  "access.locator.channel": { label: "推送通道", hint: "外部内容推送到这里的通道名" },
  "access.locator.endpoint": { label: "接口端点", hint: "付费或私有接口的完整地址" },
  "access.spec.service": { label: "服务商标识", hint: "付费或私有接口的提供方代号" },
  "access.spec.method": { label: "请求方法", hint: "调用接口用的 HTTP 方法，如 GET、POST" },
  "access.spec.headers": { label: "请求头", hint: "调用接口附加的 HTTP 头，可引用环境变量" },
  "access.spec.body": { label: "请求体", hint: "POST 请求的内容，可引用环境变量" },
  "access.spec.token": { label: "推送令牌", hint: "外部推送接入时校验身份的令牌" },
  "access.spec.ratePerSec": { label: "推送限速", hint: "每秒最多接受多少条外部推送" },
  "access.spec.burst": { label: "推送突发上限", hint: "瞬时允许超过限速的最大缓冲条数" },
  "access.spec.maxScripts": { label: "脚本解析上限", hint: "页面内嵌数据脚本最多解析多少个" },
  "access.spec.maxScriptBytes": { label: "脚本大小上限", hint: "单个内嵌脚本超过该大小就跳过" },
  "access.spec.extract": { label: "脚本抽取规则", hint: "从页面内嵌数据里提取条目列表的规则" },
  adapter: { label: "解析器", hint: "把原始响应变成结构化字段的规则" },
  pipeline: { label: "解析管道", hint: "采集到的原始数据依次交给哪些解析器处理" },
  "pipeline.run": { label: "步骤解析器", hint: "该步骤使用的解析器列表" },
  "pipeline.for": { label: "步骤场景", hint: "follow 表示抓完列表后继续跟进详情页" },
  "pipeline.spec": { label: "步骤参数", hint: "传给解析器的具体参数" },
  admission: { label: "准入规则", hint: "控制哪些条目允许进入内容库" },
  "admission.allowUrlPrefixes": { label: "准入地址前缀", hint: "只有以这些前缀开头的链接才被采集" },
  "admission.initialBackfill.limit": { label: "首次回填条数", hint: "新信源接入时最多回采多少条历史内容" },
  "admission.initialBackfill.maxAge": { label: "回溯时间窗", hint: "首次回填只采这个时限内的历史内容" },
    "admission.perRun.limit": { label: "每轮上限", hint: "每一轮采集最多入库多少条，留空表示不限制" },
    "admission.daily.limit": { label: "每日上限", hint: "当天累计到这个条数后，当天不再派发" },
  "paid.budget.requests": { label: "付费请求配额", hint: "一个统计窗口内最多发起多少次付费请求" },
  "paid.budget.window": { label: "配额窗口", hint: "付费配额的统计周期，如 1d" },
  "adapter.adapter": { label: "解析器", hint: "把原始响应变成结构化字段的规则" },
  type: { label: "内容类型", hint: "采集结果按这个类型入库" },
  schema: { label: "字段结构", hint: "该类型入库时必须满足的字段约定" },
  identity: { label: "归并键", hint: "判断两条内容是否同一条的字段" },
  "spec.keepSnapshots": { label: "快照保留数", hint: "热榜类内容一天内保留多少个快照" },
  "spec.selector.hasFacets": { label: "类型选择器", hint: "带哪些数据分层的资源才算这个类型" },

  // ── 采集通道 ────────────────────────────────────────
  implementation: { label: "实现方式", hint: "内置实现标识，如 builtin:rss" },
  limits: { label: "访问限制", hint: "超时、并发与重试" },
  "limits.timeout": { label: "单次超时", hint: "一次请求允许的最长时间" },
  "limits.concurrency": { label: "并发数", hint: "同时发起的请求数上限" },
  "limits.retry": { label: "重试策略", hint: "失败后的自动重试" },
  "limits.retry.attempts": { label: "重试次数", hint: "失败后自动重试的次数" },
  "limits.retry.backoff": { label: "退避方式", hint: "重试间隔的增长方式，如 exponential" },
  "limits.retry.base": { label: "退避基准", hint: "第一次重试前等待多久" },
  "limits.retry.max": { label: "退避上限", hint: "重试间隔最长不超过多少" },
  "egress.profile": { label: "出站策略", hint: "直连、经代理，或跟随系统设置" },
  "spec.pages": { label: "翻页数", hint: "一次采集最多翻多少页列表" },
  "spec.headers": { label: "默认请求头", hint: "该通道请求时附加的 HTTP 头" },
  "paid.service": { label: "付费服务", hint: "通道挂接的付费服务代号" },

  // ── 解析器声明与参数 ─────────────────────────────
  role: { label: "解析角色", hint: "search 表示发现新条目，content 表示补全正文" },
  consumes: { label: "输入声明", hint: "解析器接受哪些原始数据形态" },
  produces: { label: "产出声明", hint: "解析器输出的数据形态" },
  "adapter/json-list.items": { label: "条目路径", hint: "响应里列表数据所在的位置，如 /data" },
  "adapter/json-list.url": { label: "链接字段", hint: "条目原文链接取自哪个字段" },
  "adapter/json-list.urlTemplate": { label: "链接模板", hint: "用条目字段拼出原文链接，{名} 表示取字段" },
  "adapter/json-list.title": { label: "标题字段", hint: "条目标题取自哪个字段" },
  "adapter/json-list.titlePaths": { label: "标题候选", hint: "依次尝试这些字段，取第一个非空值作标题" },
  "adapter/json-list.summary": { label: "摘要字段", hint: "条目摘要取自哪个字段" },
  "adapter/json-list.summaryPaths": { label: "摘要候选", hint: "依次尝试这些字段，取第一个非空值作摘要" },
  "adapter/json-list.summaryIsBody": { label: "摘要即正文", hint: "没有独立正文时，用摘要充当正文" },
  "adapter/json-list.authorPaths": { label: "作者候选", hint: "依次尝试这些字段解析作者" },
  "adapter/json-list.publishedAt": { label: "发布时间字段", hint: "条目发布时间取自哪个字段" },
  "adapter/json-list.publishedAtPath": { label: "发布时间路径", hint: "按嵌套路径取发布时间" },
  "adapter/json-list.publishedAtUnit": { label: "时间单位", hint: "epoch_s 表示秒级时间戳，epoch_ms 表示毫秒" },
  "adapter/json-list.publishedAtUtcOffset": { label: "时区偏移", hint: "无时区标记的时间按此时区理解" },
  "adapter/json-list.externalIdPath": { label: "外部标识路径", hint: "站点侧唯一 ID 的取值路径，用于判重" },
  "adapter/json-list.decodeEntities": { label: "还原转义", hint: "把 &amp; 之类的 HTML 转义还原成原字符" },
  "adapter/json-list.detail": { label: "详情补齐", hint: "列表缺少的字段再到详情页抓取补齐" },
  "adapter/json-list.detail.maxFetches": { label: "补齐条数上限", hint: "一次采集最多跟进多少个详情页" },
  "adapter/json-list.detail.fields": { label: "补齐字段", hint: "需要从详情页补齐的字段清单" },
  "adapter/html-list.parseMode": { label: "解析模式", hint: "针对特定站点形态的预设解析方式" },
  "adapter/page-fields.publishedAt": { label: "发布时间选择器", hint: "详情页里发布时间元素的 CSS 选择器" },
  "adapter/page-fields.publishedAtAttr": { label: "时间属性", hint: "从元素哪个属性取值，如 datetime" },
  "adapter/page-fields.publishedAtUtcOffset": { label: "时区偏移", hint: "无时区标记的时间按此时区理解" },
  "adapter/page-fields.title": { label: "标题选择器", hint: "详情页标题元素的 CSS 选择器" },
  "adapter/page-fields.summary": { label: "摘要选择器", hint: "详情页摘要元素的 CSS 选择器" },

  // ── 系统策略（policy.*）──────────────────────────────
  "policy.preset": { label: "策略预设", hint: "系统采用哪套策略预设，如 preset/standard" },
  "policy.raw.store": { label: "原始响应", hint: "是否留下原始字节，便于回放与排查" },
  "policy.raw.retain": { label: "原始保留期", hint: "原始字节保留多久后清理" },
  "policy.raw.max_part_size": { label: "原始分片上限", hint: "原始响应超过该大小时拆成分片存储" },
  "policy.raw.redact_headers": { label: "脱敏请求头", hint: "留存原始响应时抹掉这些敏感头" },
  "policy.media.mode": { label: "媒体文件", hint: "图片等媒体是下载、还是只留链接" },
  "policy.media.kinds": { label: "媒体类别", hint: "允许处理的媒体类型，如 image、video" },
  "policy.media.max_size": { label: "单个媒体上限", hint: "超过该大小不再下载" },
  "policy.versions.facet_keep.raw": { label: "原始层保留", hint: "原始正文层保留多少个历史版本" },
  "policy.versions.facet_keep.md": { label: "可读层保留", hint: "Markdown 可读层保留多少个历史版本" },
  "policy.versions.facet_keep.default": { label: "其余层保留", hint: "未单独指定的层保留多少个历史版本" },
  "policy.versions.snapshot_retain": { label: "快照保留期", hint: "历史快照保留多久" },
    "policy.retention.tasks": { label: "任务保留期", hint: "终态任务超过这个时长后由保留清理删除" },
  "policy.versions.audit_tag": { label: "审计打标", hint: "按什么频率给历史版本打审计标记" },
  "policy.commit.flush_interval": { label: "提交间隔", hint: "暂存区多久批量落一次库" },
  "policy.commit.flush_max_rows": { label: "单批落库上限", hint: "一次批量落库最多写入多少行" },
  "policy.state.snapshot_interval": { label: "状态快照间隔", hint: "运行状态多久固化一次" },
  "policy.robots.mode": { label: "robots 策略", hint: "是否遵守站点爬虫守则" },
  "policy.robots.cache_ttl": { label: "守则缓存", hint: "robots 文件缓存多久后重取" },
  "policy.robots.on_unreachable": { label: "守则读不到时", hint: "站点守则无法访问时的处置" },
  "policy.robots.crawl_delay": { label: "抓取间隔", hint: "遵守守则里的 Crawl-delay 声明" },
  "policy.render.max_level": { label: "渲染层级", hint: "什么情况下启用浏览器渲染" },
  "policy.render.fallback_engine": { label: "备用渲染引擎", hint: "轻量渲染失败时改用的引擎，如 chromium" },
  "policy.render.downgrade_probe": { label: "降级探测期", hint: "持续多久渲染失败后降级到更低层级" },
  "policy.capture.screenshot": { label: "失败留证", hint: "采集失败时是否保留截图" },
  "policy.capture.network": { label: "网络留证", hint: "采集时是否保存请求与响应的网络记录" },
  "policy.capture.renditions": { label: "衍生快照", hint: "除正文外额外保留的呈现形态，如 markdown" },
  "policy.identity.merge_primary": { label: "同一条内容的判定", hint: "用站点 ID 还是 URL 判断重复" },
  "policy.identity.fragment": { label: "页面锚点", hint: "仅 # 锚点不同是否算同一条" },
  "policy.identity.strip_params": { label: "忽略参数", hint: "归并时忽略哪些 URL 参数" },
  "policy.dedupe.near_duplicate": { label: "近似去重", hint: "正文高度相似的内容是否合并" },
  "policy.reprocess.on_adapter_change": { label: "解析规则变更后", hint: "解析规则变化时是否重算旧内容" },
  "policy.backfill.initial.limit": { label: "首次回填条数", hint: "新信源第一次最多回采多少条" },
  "policy.backfill.initial.max_age": { label: "回溯时间窗", hint: "首次回填只采这个时限内的历史内容" },
  "policy.classify.detector": { label: "内容识别器", hint: "判断内容属于哪种类型所用的探测方式" },
  "policy.paid.unknown_receipt": { label: "付费回执不确定时", hint: "付费接口返回不明确时的处置" },
  "policy.health.expected_period": { label: "预期产出周期", hint: "多久没有新产出算异常" },
  "policy.health.failure_threshold": { label: "连续失败阈值", hint: "连续失败多少次进入告警" },
  "policy.health.yield_drop_ratio": { label: "产出下降告警线", hint: "相比历史下降到多少告警" },
};

/** 语义化名称与说明；未收录的字段回退为路径最后一段。 */
export function fieldMeta(path: string): FieldMeta {
  const direct = PATHS[path];
  if (direct) {
    return direct;
  }
  const segments = path.split(".");
  for (let start = 1; start < segments.length; start += 1) {
    const candidate = segments.slice(start).join(".");
    const hit = PATHS[candidate];
    if (hit) {
      return hit;
    }
  }
  return { label: segments[segments.length - 1] ?? path };
}
