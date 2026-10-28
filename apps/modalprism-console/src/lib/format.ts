export function relative(value?: string | null): string {
  if (!value) return "—";
  const at = Date.parse(value);
  if (Number.isNaN(at)) return value;
  const delta = Date.now() - at;
  const future = delta < 0;
  const seconds = Math.abs(delta) / 1000;
  const text = (n: number, unit: string) => `${Math.max(1, Math.round(n))} ${unit}`;
  let valueText: string;
  if (seconds < 60) valueText = text(seconds, "秒");
  else if (seconds < 3600) valueText = text(seconds / 60, "分钟");
  else if (seconds < 86400) valueText = text(seconds / 3600, "小时");
  else valueText = text(seconds / 86400, "天");
  return future ? `${valueText}后` : `${valueText}前`;
}

export function stamp(value?: string | null): string {
  if (!value) return "—";
  const at = new Date(value);
  if (Number.isNaN(at.getTime())) return value;
  return at.toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}

export function dateOnly(value?: string | null): string {
  if (!value) return "—";
  const at = new Date(value);
  if (Number.isNaN(at.getTime())) return value;
  return at.toLocaleDateString("zh-CN", { year: "numeric", month: "2-digit", day: "2-digit" });
}

export function count(value?: number | null): string {
  if (value === null || value === undefined) return "0";
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 10_000) return `${(value / 1_000).toFixed(1)}k`;
  return `${value}`;
}

export function bytes(value?: number | null): string {
  if (!value) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB"];
  let size = value;
  let index = 0;
  while (size >= 1024 && index < units.length - 1) {
    size /= 1024;
    index += 1;
  }
  return `${size.toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}

export function interval(ms?: number | null): string {
  if (!ms) return "—";
  if (ms % 3_600_000 === 0) return `${ms / 3_600_000} 小时`;
  if (ms % 60_000 === 0) return `${ms / 60_000} 分钟`;
  return `${Math.round(ms / 1000)} 秒`;
}

export function statusTone(status?: string | null): "ok" | "warn" | "bad" | "info" | "neutral" | "notice" {
  switch (status) {
    case "online":
    case "healthy":
    case "succeeded":
    case "published":
    case "delivered":
    case "approved":
    case "active":
      return "ok";
    case "degraded":
    case "partiallySucceeded":
    case "partiallyPublished":
    case "awaitingReview":
    case "queued":
    case "paused":
      return "warn";
    case "failed":
    case "rejected":
    case "validationFailed":
    case "configurationError":
    case "maintenance":
      return "bad";
    case "running":
    case "publishing":
    case "delivering":
      return "info";
    case "suspicious":
      return "notice";
    default:
      return "neutral";
  }
}

const STATUS_LABELS: Record<string, string> = {
  online: "在线",
  offline: "离线",
  degraded: "退化",
  maintenance: "维护中",
  healthy: "正常",
  failed: "失败",
  suspended: "已暂停",
  paused: "暂停",
  queued: "排队",
  running: "执行中",
  cancelling: "取消中",
  cancelled: "已取消",
  succeeded: "成功",
  partiallySucceeded: "部分成功",
  awaitingReview: "待审核",
  approved: "已批准",
  rejected: "已驳回",
  validationFailed: "校验失败",
  superseded: "已被替代",
  active: "启用",
  disabled: "停用",
  publishing: "发布中",
  published: "已发布",
  partiallyPublished: "部分发布",
  delivering: "投递中",
  delivered: "已投递",
  scheduled: "已排期",
  suspicious: "可疑",
  configurationError: "配置错误",
  draft: "草稿",
  validated: "已校验",
  sent: "已发送",
};

export function statusLabel(status?: string | null): string {
  if (!status) return "未知";
  return STATUS_LABELS[status] ?? status;
}

export function kindLabel(kind: string): string {
  return (
    {
      collection: "采集",
      production: "生产",
      publication: "发布",
      maintenance: "维护",
      sourceAnalysis: "信源分析",
      adapterDraft: "适配器草稿",
      adapterApprove: "适配器批准",
    }[kind] ?? kind
  );
}

/** 内容形态的用户可见名称；未知形态原样保留。 */
export function contentKindLabel(kind: string): string {
  return (
    {
      discovery: "采集条目",
      article: "文章",
      hotlist: "热榜",
      struct: "结构化对象",
      structured: "结构化对象",
      image: "图片",
      audio: "音频",
      video: "视频",
      document: "文档",
    }[kind] ?? kind
  );
}

export function errorHint(error: unknown): string | undefined {
  if (error && typeof error === "object" && "requestId" in error) {
    const requestId = (error as { requestId?: string }).requestId;
    return requestId ? `requestId ${requestId}；可在日志页按此追踪` : undefined;
  }
  return undefined;
}
