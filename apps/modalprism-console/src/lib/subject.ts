// 列表主体的统一打开语义：任何列表的「标题」都是入口，行为只在这里定义一次。
//
// 约定：有独立页面的主体跳到页面并带 `?id=` 深链；只有抽屉的主体由调用方给 onOpen。
// 这样「有的列表能点标题、有的只能点详情按钮」的双重标准就不再成立。

import { navigate } from "./router";

export type SubjectKind =
  | "content"
  | "task"
  | "source"
  | "plan"
  | "run"
  | "delivery"
  | "notification"
  | "model"
  | "prompt"
  | "skill";

export interface Subject {
  kind: SubjectKind;
  id: string;
  title?: string;
}

/** 主体所在页面（无独立页面的主体返回 null，由抽屉承接）。 */
const PAGES: Partial<Record<SubjectKind, string>> = {
  content: "content",
  task: "tasks",
  source: "sources",
  plan: "plans",
  run: "overview",
  delivery: "delivery",
  notification: "notifications",
  model: "settings",
  prompt: "settings",
  skill: "settings",
};

/** 页面内用哪个查询参数定位同一个主体。 */
const PARAM: Partial<Record<SubjectKind, string>> = {
  content: "id",
  task: "id",
  source: "id",
  plan: "id",
  run: "run",
  delivery: "id",
  notification: "id",
  model: "id",
  prompt: "id",
  skill: "id",
};

/** 主体是否有独立页面（决定「标题可点」是否成立）。 */
export function hasPage(kind: SubjectKind): boolean {
  return Boolean(PAGES[kind]);
}

/** 统一跳转：拼目标页面与主体定位参数。 */
export function subjectHref(subject: Subject): string | null {
  const page = PAGES[subject.kind];
  if (!page) return null;
  const param = PARAM[subject.kind] ?? "id";
  return `${page}?${param}=${encodeURIComponent(subject.id)}`;
}

/** 统一打开：优先给回调（抽屉等就地交互），否则按主体类型跳页。 */
export function openSubject(subject: Subject, open?: (subject: Subject) => void): void {
  if (open) {
    open(subject);
    return;
  }
  const href = subjectHref(subject);
  if (href) navigate(href);
}

/** 从地址栏读当前深链的主体标识：`?id=xxx` 或 `?run=xxx`。 */
export function subjectFromLocation(kind: SubjectKind): string | null {
  if (typeof window === "undefined") return null;
  const query = window.location.hash.split("?")[1] ?? "";
  const param = PARAM[kind] ?? "id";
  return new URLSearchParams(query).get(param);
}
