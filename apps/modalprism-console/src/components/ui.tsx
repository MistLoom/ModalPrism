import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

import { IconAlert, IconCheck, IconClose } from "./icons";
import { ApiError, runOperation } from "../lib/api";
import type { Operation } from "../lib/types";
import { statusLabel, statusTone } from "../lib/format";

export function Badge({ tone = "neutral", children }: { tone?: string; children: ReactNode }) {
  return <span className={`badge ${tone}`}>{children}</span>;
}

export function StatusBadge({ status }: { status?: string | null }) {
  return <Badge tone={statusTone(status)}>{statusLabel(status)}</Badge>;
}

export function Card({
  title,
  sub,
  action,
  children,
  flush,
}: {
  title?: ReactNode;
  sub?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  flush?: boolean;
}) {
  return (
    <section className="card">
      {title ? (
        <header className="card-head">
          <div className="card-head-text">
            <div className="card-title">{title}</div>
            {sub ? <div className="card-sub">{sub}</div> : null}
          </div>
          {action}
        </header>
      ) : null}
      <div className={`card-body${flush ? " flush" : ""}`}>{children}</div>
    </section>
  );
}

export function Empty({ title, text, action }: { title: string; text?: string; action?: ReactNode }) {
  return (
    <div className="empty">
      <strong>{title}</strong>
      {text ? <span>{text}</span> : null}
      {action}
    </div>
  );
}

export function Loading({ lines = 4 }: { lines?: number }) {
  return (
    <div className="skeleton" aria-label="加载中">
      {Array.from({ length: lines }, (_, index) => (
        <span key={index} style={{ width: `${92 - index * 9}%` }} />
      ))}
    </div>
  );
}

export function Problem({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  const message = error instanceof Error ? error.message : String(error);
  const requestId = error instanceof ApiError ? error.requestId : "";
  return (
    <div className="alert bad" role="alert">
      <IconAlert size={16} />
      <div className="stack" style={{ gap: 3 }}>
        <strong>{message}</strong>
        {requestId ? (
          <span className="small">
            requestId <code>{requestId}</code>
            <button type="button" className="btn btn-ghost btn-sm" onClick={() => { window.location.hash = `#/logs?correlationId=${encodeURIComponent(requestId)}`; }}>
              在日志页追踪
            </button>
          </span>
        ) : null}
        {onRetry ? (
          <span>
            <button type="button" className="btn btn-sm" onClick={onRetry}>
              重试
            </button>
          </span>
        ) : null}
      </div>
    </div>
  );
}

export function Field({
  label,
  hint,
  error,
  children,
  htmlFor,
}: {
  label: ReactNode;
  hint?: ReactNode;
  error?: string;
  children: ReactNode;
  htmlFor?: string;
}) {
  return (
    <div className="field">
      <label className="field-label" htmlFor={htmlFor}>
        {label}
      </label>
      {children}
      {hint ? <span className="field-hint">{hint}</span> : null}
      {error ? <span className="field-error">{error}</span> : null}
    </div>
  );
}

export function Tabs<T extends string>({
  value,
  items,
  onChange,
}: {
  value: T;
  items: { id: T; label: string; count?: number }[];
  onChange: (value: T) => void;
}) {
  return (
    <div className="tabs" role="tablist">
      {items.map((item) => (
        <button
          key={item.id}
          type="button"
          role="tab"
          aria-selected={value === item.id}
          className={`tab${value === item.id ? " is-active" : ""}`}
          onClick={() => onChange(item.id)}
        >
          {item.label}
          {item.count === undefined ? null : <span className="muted"> {item.count}</span>}
        </button>
      ))}
    </div>
  );
}

function useOverlay(onClose: () => void, open: boolean) {
  const ref = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement as HTMLElement | null;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
      if (event.key === "Tab" && ref.current) {
        const nodes = ref.current.querySelectorAll<HTMLElement>(
          'a[href], button:not([disabled]), textarea, input, select, [tabindex]:not([-1])',
        );
        if (nodes.length === 0) return;
        const first = nodes[0];
        const last = nodes[nodes.length - 1];
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first.focus();
        }
      }
    };
    document.addEventListener("keydown", onKey);
    document.body.style.overflow = "hidden";
    window.setTimeout(() => {
      const target = ref.current?.querySelector<HTMLElement>("input, textarea, select, button");
      target?.focus();
    }, 30);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.body.style.overflow = "";
      previous?.focus();
    };
  }, [onClose, open]);
  return ref;
}

export function Drawer({
  title,
  sub,
  onClose,
  footer,
  children,
  wide,
}: {
  title: ReactNode;
  sub?: ReactNode;
  onClose: () => void;
  footer?: ReactNode;
  children: ReactNode;
  wide?: boolean;
}) {
  const ref = useOverlay(onClose, true);
  // 挂到 body：顶栏/卡片带 backdrop-filter 或 transform 时，fixed 浮层会被困在祖先盒子里。
  return createPortal(
    <>
      <div className="scrim" onClick={onClose} aria-hidden="true" />
      <aside ref={ref} className={`drawer${wide ? " wide" : ""}`} role="dialog" aria-modal="true" aria-label={typeof title === "string" ? title : "详情"}>
        <header className="drawer-head">
          <div className="fill">
            <h2>{title}</h2>
            {sub ? <div className="card-sub">{sub}</div> : null}
          </div>
          <button type="button" className="icon-btn" onClick={onClose} aria-label="关闭">
            <IconClose size={17} />
          </button>
        </header>
        <div className="drawer-body">{children}</div>
        {footer ? <footer className="drawer-foot">{footer}</footer> : null}
      </aside>
    </>,
    document.body,
  );
}

export function Modal({
  title,
  sub,
  onClose,
  footer,
  children,
  wide,
}: {
  title: ReactNode;
  sub?: ReactNode;
  onClose: () => void;
  footer?: ReactNode;
  children: ReactNode;
  wide?: boolean;
}) {
  const ref = useOverlay(onClose, true);
  return createPortal(
    <>
      <div className="scrim" onClick={onClose} aria-hidden="true" />
      <div ref={ref} className={`modal${wide ? " wide" : ""}`} role="dialog" aria-modal="true" aria-label={typeof title === "string" ? title : "对话框"}>
        <header className="modal-head">
          <div className="fill">
            <h2>{title}</h2>
            {sub ? <div className="card-sub">{sub}</div> : null}
          </div>
          <button type="button" className="icon-btn" onClick={onClose} aria-label="关闭">
            <IconClose size={17} />
          </button>
        </header>
        <div className="modal-body">{children}</div>
        {footer ? <footer className="modal-foot">{footer}</footer> : null}
      </div>
    </>,
    document.body,
  );
}

/// 常见字段名的中文标注：详情类页面用结构化字段而非原始 JSON 暴露技术键。
const FIELD_LABELS: Record<string, string> = {
  id: "标识",
  ref: "引用",
  rid: "内容",
  kind: "类型",
  name: "名称",
  title: "标题",
  text: "文本",
  body: "正文",
  summary: "摘要",
  description: "说明",
  url: "地址",
  uri: "地址",
  channel: "频道",
  query: "查询",
  cursor: "游标",
  mode: "模式",
  status: "状态",
  state: "状态",
  error: "错误",
  reason: "原因",
  message: "消息",
  type: "类型",
  variant: "变体",
  version: "版本",
  tags: "标签",
  labels: "标签",
  sourceId: "来源",
  sourceSlug: "来源标识",
  sourceName: "来源名称",
  taskId: "任务",
  planId: "计划",
  runId: "运行",
  contentId: "内容",
  operationId: "操作",
  requestId: "请求",
  attemptId: "尝试",
  deliveryId: "投递",
  targetId: "目标",
  targetKind: "目标类型",
  latencyMs: "耗时（毫秒）",
  createdAt: "创建时间",
  updatedAt: "更新时间",
  publishedAt: "发布时间",
  retryAt: "重试时间",
  lastOkAt: "最近成功",
  count: "数量",
  total: "总数",
  items: "条目",
  nodes: "节点",
  edges: "边",
  from: "起点",
  to: "终点",
  relation: "关系",
  direction: "方向",
  attrs: "属性",
  output: "输出",
  locator: "定位",
  params: "参数",
  headers: "请求头",
};

function fieldLabel(key: string): string {
  if (FIELD_LABELS[key]) return FIELD_LABELS[key];
  // 兜底：驼峰拆成可读词组，避免原样抛出技术键。
  return key.replace(/([a-z0-9])([A-Z])/g, "$1 $2").replace(/^./, (char) => char.toUpperCase());
}

const URL_PATTERN = /^https?:\/\//i;

/// 结构化值：对象按字段名值、数组按列表、长文本按段落、链接可点；不向用户抛原始 JSON。
export function StructuredValue({ value, depth = 0 }: { value: unknown; depth?: number }) {
  if (value === null || value === undefined) return <span className="muted">—</span>;
  if (typeof value === "boolean") return <span>{value ? "是" : "否"}</span>;
  if (typeof value === "number") return <span className="mono">{value.toLocaleString("zh-CN")}</span>;
  if (typeof value === "string") {
    const text = value.trim();
    if (!text) return <span className="muted">—</span>;
    if (URL_PATTERN.test(text)) {
      return <a className="value-link" href={text} target="_blank" rel="noreferrer">{text}</a>;
    }
    if (text.length > 120 || text.includes("\n")) {
      return (
        <div className="value-text">
          {text.split(/\n{2,}/).map((paragraph, index) => <p key={index}>{paragraph}</p>)}
        </div>
      );
    }
    return <span>{text}</span>;
  }
  if (Array.isArray(value)) {
    if (value.length === 0) return <span className="muted">无</span>;
    if (value.every((item) => item === null || typeof item !== "object")) {
      return (
        <span className="row wrap" style={{ gap: 6 }}>
          {value.map((item, index) => <span className="tag" key={index}>{inlineValue(item)}</span>)}
        </span>
      );
    }
    return (
      <div className="value-list">
        {value.map((item, index) => (
          <div className="value-item" key={index}>
            <StructuredValue value={item} depth={depth + 1} />
          </div>
        ))}
      </div>
    );
  }
  const entries = Object.entries(value as Record<string, unknown>).filter(([, item]) => item !== undefined);
  if (entries.length === 0) return <span className="muted">无</span>;
  return (
    <dl className={`kv${depth > 0 ? " is-nested" : ""}`}>
      {entries.map(([key, item]) => (
        <div key={key} style={{ display: "contents" }}>
          <dt>{fieldLabel(key)}</dt>
          <dd><StructuredValue value={item} depth={depth + 1} /></dd>
        </div>
      ))}
    </dl>
  );
}

/** 单行结构化摘要：用于差异对比、表格内联展示等不能展开的场景。 */
export function inlineValue(value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (typeof value === "string") return value || "（空）";
  if (typeof value === "boolean") return value ? "是" : "否";
  if (typeof value === "number") return String(value);
  if (Array.isArray(value)) return value.length ? value.map(inlineValue).join("、") : "无";
  const entries = Object.entries(value as Record<string, unknown>).filter(([, item]) => item !== undefined);
  if (entries.length === 0) return "无";
  return entries.map(([key, item]) => `${fieldLabel(key)} ${inlineValue(item)}`).join("；");
}

export function Pager({ next, onNext, onReset }: { next: string | null; onNext: () => void; onReset: () => void }) {
  return (
    <div className="pager">
      <span>{next ? "还有更多记录" : "已到末页"}</span>
      <div className="row">
        <button type="button" className="btn btn-sm" onClick={onReset}>
          回到开头
        </button>
        <button type="button" className="btn btn-sm" onClick={onNext} disabled={!next}>
          下一页
        </button>
      </div>
    </div>
  );
}

interface Toast {
  id: number;
  tone: "ok" | "bad" | "warn" | "info";
  title: string;
  detail?: string;
}

interface Tracked {
  key: string;
  title: string;
  operation: Operation;
}

interface UiState {
  notify: (tone: Toast["tone"], title: string, detail?: string) => void;
  run: (operation: Operation, title: string, options?: { silentSuccess?: boolean }) => Promise<Operation>;
}

const UiContext = createContext<UiState | null>(null);

export function UiProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [tracked, setTracked] = useState<Tracked[]>([]);
  const next = useRef(1);

  const dismiss = useCallback((id: number) => {
    setToasts((current) => current.filter((toast) => toast.id !== id));
  }, []);

  const notify = useCallback(
    (tone: Toast["tone"], title: string, detail?: string) => {
      const id = next.current++;
      setToasts((current) => [...current.slice(-3), { id, tone, title, detail }]);
      window.setTimeout(() => dismiss(id), tone === "bad" ? 9000 : 5000);
    },
    [dismiss],
  );

  const run = useCallback(
    async (operation: Operation, title: string, options?: { silentSuccess?: boolean }) => {
      const key = operation.operationId;
      setTracked((current) => [...current.filter((item) => item.key !== key), { key, title, operation }]);
      try {
        const finished = await runOperation(operation, (value) => {
          setTracked((current) => current.map((item) => (item.key === key ? { ...item, operation: value } : item)));
        });
        if (finished.status === "partiallySucceeded") {
          notify("warn", `${title}：部分成功`, "失败项已在详情中列出。");
        } else if (finished.resultSummary?.status === "failed") {
          notify("warn", `${title}：已完成，但结果失败`, "请在对应页面查看失败项与错误。");
        } else if (finished.resultSummary?.status === "partiallyPublished") {
          notify("warn", `${title}：部分完成`, "部分渠道或步骤失败，已保留成功项。");
        } else if (!options?.silentSuccess) {
          notify("ok", `${title}：完成`);
        }
        return finished;
      } catch (error) {
        notify("bad", `${title}：失败`, error instanceof Error ? error.message : undefined);
        throw error;
      } finally {
        window.setTimeout(() => {
          setTracked((current) => current.filter((item) => item.key !== key));
        }, 2200);
      }
    },
    [notify],
  );

  const value = useMemo(() => ({ notify, run }), [notify, run]);
  return (
    <UiContext.Provider value={value}>
      {children}
      {tracked.length > 0 ? (
        <div className="op-dock" aria-live="polite">
          {tracked.map((item) => (
            <div key={item.key} className="stack" style={{ gap: 5 }}>
              <div className="row between">
                <strong className="small">{item.title}</strong>
                <span className="small muted">{statusLabel(item.operation.status)}</span>
              </div>
              <div className="progress">
                <span style={{ width: `${Math.round(Math.max(4, item.operation.progress * 100))}%` }} />
              </div>
            </div>
          ))}
        </div>
      ) : null}
      <div className="toasts" aria-live="polite">
        {toasts.map((toast) => (
          <div key={toast.id} className={`toast ${toast.tone}`}>
            <span style={{ color: toast.tone === "bad" ? "var(--danger)" : toast.tone === "ok" ? "var(--good)" : "var(--accent)" }}>
              {toast.tone === "ok" ? <IconCheck size={16} /> : <IconAlert size={16} />}
            </span>
            <div className="fill">
              <strong>{toast.title}</strong>
              {toast.detail ? <div className="small muted">{toast.detail}</div> : null}
            </div>
            <button type="button" className="icon-btn" onClick={() => dismiss(toast.id)} aria-label="关闭通知">
              <IconClose size={14} />
            </button>
          </div>
        ))}
      </div>
    </UiContext.Provider>
  );
}

export function useUi(): UiState {
  const value = useContext(UiContext);
  if (!value) throw new Error("UiProvider 未挂载");
  return value;
}
