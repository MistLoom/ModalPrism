// 极简哈希路由：静态托管下不需要服务器改写就能深链。
import { useEffect, useState } from "react";

export type RouteId =
  | "overview"
  | "sources"
  | "tasks"
  | "strata"
  | "library"
  | "governance"
  | "channels"
  | "config";

export type RouteGroup = "观测" | "系统";

export const ROUTES: { id: RouteId; label: string; hint: string; group: RouteGroup }[] = [
  { id: "overview", label: "总览", hint: "信源健康与最近提交", group: "观测" },
  { id: "sources", label: "信源", hint: "信源清单与调度时间", group: "观测" },
  { id: "tasks", label: "任务", hint: "任务队列、重试与调度", group: "观测" },
  { id: "strata", label: "Strata", hint: "内容分层、版本与血缘", group: "观测" },
  { id: "library", label: "内容库", hint: "已入库的内容与快照", group: "观测" },
  { id: "governance", label: "治理", hint: "限速、熔断与站点守则", group: "系统" },
  { id: "channels", label: "采集通道", hint: "通道配置与出站策略", group: "系统" },
  { id: "config", label: "系统配置", hint: "声明文件、校验与生效", group: "系统" },
];

export const ROUTE_GROUPS: RouteGroup[] = ["观测", "系统"];

export function routeLabel(id: RouteId): string {
  return ROUTES.find((item) => item.id === id)?.label ?? id;
}

export function routeOf(hash: string): RouteId {
  const path = hash.replace(/^#\/?/, "");
  const head = path.split("/")[0];
  return (ROUTES.find((item) => item.id === head)?.id ?? "overview") as RouteId;
}

export function paramOf(hash: string): string | null {
  const path = hash.replace(/^#\/?/, "");
  const parts = path.split("/");
  if (parts.length < 2 || !parts[1]) {
    return null;
  }
  try {
    return decodeURIComponent(parts.slice(1).join("/"));
  } catch {
    return parts.slice(1).join("/");
  }
}

export function useRoute(): { route: RouteId; param: string | null } {
  const [hash, setHash] = useState(() => window.location.hash);
  useEffect(() => {
    const onChange = () => setHash(window.location.hash);
    window.addEventListener("hashchange", onChange);
    return () => window.removeEventListener("hashchange", onChange);
  }, []);
  return { route: routeOf(hash), param: paramOf(hash) };
}

export function navigate(path: string): void {
  const next = path.startsWith("#") ? path : `#/${path.replace(/^\//, "")}`;
  if (window.location.hash === next) {
    window.dispatchEvent(new HashChangeEvent("hashchange"));
    return;
  }
  window.location.hash = next;
}
