import { useEffect, useState } from "react";

export type PageId =
  | "overview"
  | "projects"
  | "sources"
  | "plans"
  | "tasks"
  | "content"
  | "delivery"
  | "notifications"
  | "logs"
  | "external"
  | "tools"
  | "settings";

export interface RouteDef {
  id: PageId;
  label: string;
  kicker: string;
  group: "workspace" | "diagnostics";
}

// 一级页面：日常在做的事（工作区）与排查/接入（诊断）分两组。
export const ROUTES: RouteDef[] = [
  { id: "overview", label: "概览", kicker: "overview", group: "workspace" },
  { id: "projects", label: "项目", kicker: "projects", group: "workspace" },
  { id: "sources", label: "信源", kicker: "sources", group: "workspace" },
  { id: "plans", label: "计划", kicker: "plans", group: "workspace" },
  { id: "tasks", label: "任务", kicker: "tasks", group: "workspace" },
  { id: "content", label: "内容", kicker: "content", group: "workspace" },
  { id: "delivery", label: "内容投递", kicker: "delivery", group: "workspace" },
  { id: "notifications", label: "通知管理", kicker: "notifications", group: "workspace" },
  { id: "external", label: "外部调用", kicker: "external", group: "diagnostics" },
  { id: "logs", label: "日志", kicker: "logs", group: "diagnostics" },
  { id: "tools", label: "调试工具", kicker: "tools", group: "diagnostics" },
  { id: "settings", label: "系统设置", kicker: "settings", group: "diagnostics" },
];

export interface RouteState {
  page: PageId;
  params: URLSearchParams;
  hash: string;
}

export function parseHash(hash: string): RouteState {
  const text = hash.replace(/^#\/?/, "");
  const [path, search = ""] = text.split("?");
  const head = path.split("/")[0] as PageId;
  const page = ROUTES.some((route) => route.id === head) ? head : "overview";
  return { page, params: new URLSearchParams(search), hash };
}

export function useRoute(): RouteState {
  const [state, setState] = useState(() => parseHash(window.location.hash));
  useEffect(() => {
    const update = () => setState(parseHash(window.location.hash));
    window.addEventListener("hashchange", update);
    return () => window.removeEventListener("hashchange", update);
  }, []);
  return state;
}

export function navigate(path: string): void {
  const next = path.startsWith("#") ? path : `#/${path.replace(/^\//, "")}`;
  if (window.location.hash === next) {
    window.dispatchEvent(new HashChangeEvent("hashchange"));
    return;
  }
  window.location.hash = next;
}

export function pageLabel(page: PageId): string {
  return ROUTES.find((route) => route.id === page)?.label ?? page;
}

export function pageKicker(page: PageId): string {
  return ROUTES.find((route) => route.id === page)?.kicker ?? page;
}
