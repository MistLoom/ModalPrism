import { useEffect, useState } from "react";

export type PageId =
  | "overview"
  | "projects"
  | "sources"
  | "tasks"
  | "content"
  | "production"
  | "outbound"
  | "logs"
  | "settings";

export const ROUTES: { id: PageId; label: string; kicker: string }[] = [
  { id: "overview", label: "概览", kicker: "overview" },
  { id: "projects", label: "项目", kicker: "projects" },
  { id: "sources", label: "信源", kicker: "sources" },
  { id: "tasks", label: "任务", kicker: "tasks" },
  { id: "content", label: "内容", kicker: "content" },
  { id: "production", label: "生产", kicker: "production" },
  { id: "outbound", label: "出站", kicker: "outbound" },
  { id: "logs", label: "日志", kicker: "logs" },
  { id: "settings", label: "系统设置", kicker: "settings" },
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
