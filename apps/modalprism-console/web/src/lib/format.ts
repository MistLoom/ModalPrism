// 所有「把机器数据说成人话」的转换集中在这里。

const MINUTE = 60_000;
const HOUR = 3_600_000;
const DAY = 86_400_000;

/** 稀疏相对时间：刚刚 / 12 分钟前 / 3 小时前 / 2 天前 / 10月18日 */
export function relative(iso: string | null | undefined, now = Date.now()): string {
  if (!iso) {
    return "—";
  }
  const at = Date.parse(iso);
  if (Number.isNaN(at)) {
    return "—";
  }
  const delta = now - at;
  if (delta < -MINUTE) {
    return until(iso, now);
  }
  if (delta < MINUTE) {
    return "刚刚";
  }
  if (delta < HOUR) {
    return `${Math.floor(delta / MINUTE)} 分钟前`;
  }
  if (delta < DAY) {
    return `${Math.floor(delta / HOUR)} 小时前`;
  }
  if (delta < DAY * 7) {
    return `${Math.floor(delta / DAY)} 天前`;
  }
  return shortDate(iso);
}

/** 未来的时间点说成等待时长：4 分钟后 / 2 小时后 */
export function until(iso: string | null | undefined, now = Date.now()): string {
  if (!iso) {
    return "未排期";
  }
  const at = Date.parse(iso);
  if (Number.isNaN(at)) {
    return "未排期";
  }
  const delta = at - now;
  if (delta <= 0) {
    return "等待出发";
  }
  if (delta < MINUTE) {
    return "不到 1 分钟";
  }
  if (delta < HOUR) {
    return `${Math.round(delta / MINUTE)} 分钟后`;
  }
  if (delta < DAY) {
    return `${Math.round(delta / HOUR)} 小时后`;
  }
  return `${Math.round(delta / DAY)} 天后`;
}

export function shortDate(iso: string | null | undefined): string {
  if (!iso) {
    return "—";
  }
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) {
    return "—";
  }
  return `${date.getMonth() + 1}月${date.getDate()}日`;
}

export function clockTime(date = new Date()): string {
  return date.toLocaleTimeString("zh-CN", { hour12: false });
}

export function stamp(iso: string | null | undefined): string {
  if (!iso) {
    return "—";
  }
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) {
    return "—";
  }
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/** 节律：90s 说「每 1 分钟」这样更接近人的直觉 */
export function cadence(ms: number | null | undefined): string {
  if (!ms || ms <= 0) {
    return "不定";
  }
  if (ms < MINUTE) {
    return `每 ${Math.round(ms / 1000)} 秒`;
  }
  if (ms < HOUR) {
    return `每 ${round(ms / MINUTE)} 分钟`;
  }
  if (ms < DAY) {
    return `每 ${round(ms / HOUR)} 小时`;
  }
  return `每 ${round(ms / DAY)} 天`;
}

function round(value: number): string {
  const rounded = Math.round(value * 10) / 10;
  return Number.isInteger(rounded) ? String(rounded) : rounded.toFixed(1);
}

export function count(value: number | null | undefined): string {
  if (value === null || value === undefined) {
    return "0";
  }
  if (value < 1000) {
    return String(value);
  }
  if (value < 100_000) {
    return `${(value / 1000).toFixed(1)}k`;
  }
  return `${Math.round(value / 1000)}k`;
}

export function bytes(value: number | null | undefined): string {
  if (!value) {
    return "0 B";
  }
  const units = ["B", "KB", "MB", "GB"];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${unit === 0 ? size : size.toFixed(1)} ${units[unit]}`;
}

export interface HealthTone {
  tone: "ok" | "warn" | "bad" | "idle";
  label: string;
  detail: string;
}

/** 状态词用日常语言：正常 / 需要留意 / 已停用 / 待首次采集 */
export function healthOf(health: string, enabled = true): HealthTone {
  if (!enabled) {
    return { tone: "idle", label: "已停用", detail: "该信源在配置里被关闭，不会自动采集" };
  }
  switch (health) {
    case "ok":
      return { tone: "ok", label: "正常", detail: "最近一次采集成功" };
    case "warn":
    case "alert":
      return { tone: "warn", label: "需要留意", detail: "产出或连续失败达到告警线" };
    case "paused":
      return { tone: "idle", label: "已暂停", detail: "连续失败后暂停，恢复可手动触发一次采集" };
    case "misconfigured":
      return { tone: "bad", label: "配置有误", detail: "配置无法生成采集计划，需要修改配置" };
    case "error":
      return { tone: "bad", label: "失败", detail: "最近一次采集失败" };
    case "idle":
    case "":
      return { tone: "idle", label: "待首次采集", detail: "还没有采集记录" };
    default:
      return { tone: "warn", label: health, detail: "见最近一次错误" };
  }
}

/** 常见失败的日常说法。未知错误原样保留，但不再额外加术语。 */
export function explainError(raw: string | null | undefined): string | null {
  if (!raw) {
    return null;
  }
  const table: [RegExp, string][] = [
    [/deadline|budget|timeout/i, "本次采集超出时间预算，任务会在下一轮重试"],
    [/robots/i, "目标站点不允许抓取，或站点守则拉取失败"],
    [/dns/i, "域名解析失败，可能是网络或站点暂时不可用"],
    [/transport|connection|connect/i, "连接站点失败，可能是网络波动或站点拒绝"],
    [/scheme/i, "地址格式不被接受"],
    [/parse|adapter|selector/i, "页面结构变了，解析规则没有匹配到内容"],
    [/schema|invalid/i, "抓到的字段不符合内容类型定义"],
    [/config/i, "配置问题，需要在配置页修正"],
  ];
  for (const [pattern, text] of table) {
    if (pattern.test(raw)) {
      return text;
    }
  }
  return null;
}

export function profileLabel(profile: string): string {
  const table: Record<string, string> = {
    direct: "直连",
    proxy: "经代理",
    auto: "自动选择",
    system: "系统设置",
  };
  return table[profile] ?? profile;
}

export function kindLabel(kind: string): string {
  const table: Record<string, string> = {
    system: "系统策略",
    source: "信源",
    accessor: "采集通道",
    adapter: "解析器",
    type: "内容类型",
    preset: "预设",
  };
  return table[kind] ?? kind;
}

/** 网页 HTML 转成可读纯文本：段落断行、去标签、还原常见实体。 */
export function asReadable(html: string): string {
  return html
    .replace(/<\/(p|div|li|h[1-6]|tr)>/gi, "\n\n")
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/<[^>]+>/g, "")
    .replace(/&nbsp;/g, " ")
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}
