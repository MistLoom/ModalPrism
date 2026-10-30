import { t } from "./dictionary";

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

/** 字典未收录的状态补充文案（运行/发布/审核等界面状态，不属于任务状态域）。 */
const STATUS_FALLBACK: Record<string, string> = {
  online: "在线",
  offline: "离线",
  maintenance: "维护中",
  healthy: "正常",
  suspended: "已暂停",
  paused: "暂停",
  cancelling: "取消中",
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
  partiallyPublished: "部分成功",
  delivering: "投递中",
  delivered: "已投递",
  scheduled: "已排期",
  suspicious: "待核实",
  configurationError: "配置错误",
  draft: "草稿",
  validated: "已校验",
  sent: "已发送",
};

/** 状态文案：字典优先，补充表兜底，最后回落原值。 */
export function statusLabel(status?: string | null): string {
  if (!status) return "未知";
  return t("task.status", status, STATUS_FALLBACK[status] ?? status);
}

/** 操作/任务种类：先按任务值域翻，再按操作与事件值域翻，最后回落原值。 */
export function kindLabel(kind: string): string {
  return t("task.kind", kind, t("operation.kind", kind, kind));
}

/** 日志级别：trace/debug/info/warn/error 的中文名。 */
export function levelLabel(level?: string | null): string {
  if (!level) return "—";
  return t("log.level", level, level);
}

/** 框架监测事件的种类与相位。 */
export function monitorKindLabel(kind?: string | null): string {
  if (!kind) return "—";
  return t("monitor.kind", kind, kind);
}

export function monitorPhaseLabel(phase?: string | null): string {
  if (!phase) return "";
  return t("monitor.phase", phase, phase);
}

/** 监测事件主体：`实例.方法` 拆开后各自翻译，未收录的部分保留原值。 */
export function monitorAbilityLabel(instance?: string | null, method?: string | null): string {
  const ability = instance ? t("ability", instance, instance) : "";
  const action = method ? t("ability.method", method, method) : "";
  return [ability, action].filter(Boolean).join(" · ");
}

/** 操作过程事件（已受理、成功、请求取消等）。 */
export function operationEventLabel(kind?: string | null): string {
  if (!kind) return "—";
  return t("operation.event", kind, kind);
}

/** 观测流名称：logs / events / audit。 */
export function streamLabel(stream?: string | null): string {
  if (!stream) return "—";
  return t("log.stream", stream, stream);
}

/** 操作关联资源种类：source、contentBatch 等。 */
export function refKindLabel(kind?: string | null): string {
  if (!kind) return "—";
  return t("ref.kind", kind, kind);
}

/** 计划命中限额/停止策略的原因。 */
export function stopReasonLabel(reason?: string | null): string {
  if (!reason) return "—";
  return t("source.stopReason", reason, reason);
}

/** 内容形态的用户可见名称：先按数据面变体翻，再按数据面类型翻。 */
export function contentKindLabel(kind: string): string {
  return t("facet.variant", kind, t("facet.kind", kind, kind));
}

export function errorHint(error: unknown): string | undefined {
  if (error && typeof error === "object" && "requestId" in error) {
    const requestId = (error as { requestId?: string }).requestId;
    return requestId ? `requestId ${requestId}；可在日志页按此追踪` : undefined;
  }
  return undefined;
}
