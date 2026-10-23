import type { ReactNode } from "react";

import { IconChevronLeft, IconChevronRight } from "./icons";
import { fieldMeta } from "../lib/fields";
import { count, healthOf, relative, until } from "../lib/format";
import type { HealthState } from "../lib/types";

export function Pill({
  tone = "idle",
  dot = false,
  children,
}: {
  tone?: "ok" | "warn" | "bad" | "idle" | "info";
  dot?: boolean;
  children: ReactNode;
}) {
  return (
    <span className={`pill pill-${tone}`}>
      {dot ? <span className="dot" /> : null}
      {children}
    </span>
  );
}

export function StatusPill({ health, enabled = true }: { health: HealthState; enabled?: boolean }) {
  const tone = healthOf(health, enabled);
  return (
    <Pill tone={tone.tone === "bad" ? "bad" : tone.tone === "warn" ? "warn" : tone.tone === "ok" ? "ok" : "idle"} dot>
      {tone.label}
    </Pill>
  );
}

export function Stat({
  label,
  value,
  unit,
  foot,
  alert = false,
}: {
  label: string;
  value: ReactNode;
  unit?: string;
  foot?: ReactNode;
  alert?: boolean;
}) {
  return (
    <div className={`stat${alert ? " is-alert" : ""}`}>
      <span className="stat-label">{label}</span>
      <span className="stat-value">
        {value}
        {unit ? <small>{unit}</small> : null}
      </span>
      {foot ? <span className="stat-foot">{foot}</span> : null}
    </div>
  );
}

export function Card({
  title,
  sub,
  action,
  children,
  foot,
  tight = false,
}: {
  title?: ReactNode;
  sub?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  foot?: ReactNode;
  tight?: boolean;
}) {
  return (
    <section className="card">
      {title ? (
        <header className="card-head">
          <div>
            <div className="card-title">{title}</div>
            {sub ? <div className="card-sub">{sub}</div> : null}
          </div>
          {action}
        </header>
      ) : null}
      <div className={`card-body${tight ? " tight" : ""}`}>{children}</div>
      {foot ? <div className="card-foot">{foot}</div> : null}
    </section>
  );
}

export function Empty({
  title,
  text,
  action,
}: {
  title: string;
  text?: string;
  action?: ReactNode;
}) {
  return (
    <div className="empty">
      <p className="empty-title">{title}</p>
      {text ? <p className="empty-text">{text}</p> : null}
      {action}
    </div>
  );
}

export function Skeleton({ lines = 3 }: { lines?: number }) {
  return (
    <div className="skeleton-lines" aria-hidden="true">
      {Array.from({ length: lines }).map((_, index) => (
        <span
          key={index}
          className="skeleton"
          style={{ width: `${96 - index * 9}%` }}
        />
      ))}
    </div>
  );
}

export function NextRun({ at, now }: { at: string | null; now: number }) {
  if (!at) {
    return <span className="faint">未排期</span>;
  }
  const due = Date.parse(at) <= now;
  return (
    <span className={due ? "" : "muted"} title={at}>
      {due ? "等待出发" : until(at, now)}
    </span>
  );
}

export function LastRun({ at }: { at: string | null }) {
  return (
    <span className="muted" title={at ?? undefined}>
      {relative(at)}
    </span>
  );
}

export function YieldBar({ value, max }: { value: number; max: number }) {
  const ratio = max > 0 ? Math.max(0.04, Math.min(1, value / max)) : 0;
  return (
    <span className="yield-cell">
      <span className="yield-number">{count(value)}</span>
      <span className="yield-bar" aria-hidden="true">
        <i style={{ width: `${Math.round(ratio * 100)}%` }} />
      </span>
    </span>
  );
}

export function SearchInput({
  value,
  onChange,
  placeholder,
  label,
}: {
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
  label: string;
}) {
  return (
    <label className="field-input">
      <svg viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="currentColor" strokeWidth="1.4" aria-hidden="true">
        <circle cx="7.2" cy="7.2" r="4.2" />
        <path d="m10.4 10.4 3 3" strokeLinecap="round" />
      </svg>
      <span className="sr-only">{label}</span>
      <input
        type="search"
        value={value}
        placeholder={placeholder}
        onChange={(event) => onChange(event.target.value)}
        spellCheck={false}
      />
    </label>
  );
}

export function Seg<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  label: string;
}) {
  return (
    <div className="seg" role="group" aria-label={label}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          className={option.value === value ? "is-active" : undefined}
          aria-pressed={option.value === value}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

export function Definitions({ items }: { items: [string, ReactNode][] }) {
  return (
    <dl className="defs">
      {items.map(([term, value], index) => (
        <div key={`${term}-${index}`} style={{ display: "contents" }}>
          <dt>{term}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

export function SectionTitle({ title, note }: { title: string; note?: ReactNode }) {
  return (
    <div className="section-head">
      <span className="section-title">{title}</span>
      {note ? <span className="section-note">{note}</span> : null}
    </div>
  );
}

/** 字段名：语义化的中文名称为主行，声明文件里的原始路径作为副行，说明放在悬停提示里。 */
export function FieldName({ path }: { path: string }) {
  const meta = fieldMeta(path);
  const rawKey = path.split(".").pop() ?? path;
  const known = meta.label !== rawKey;
  const tip = meta.hint ? `${meta.label} · ${meta.hint}` : path;
  return (
    <span className="origin-name" title={tip}>
      {known ? <span className="origin-label">{meta.label}</span> : null}
      <span className="origin-path">{path}</span>
    </span>
  );
}

/** 折叠面板：标题行整行可点，展开后内容替换在下方。用于默认不呈现的高级设置。 */
export function Collapse({
  title,
  note,
  badge,
  open,
  onToggle,
  children,
}: {
  title: ReactNode;
  note?: ReactNode;
  badge?: ReactNode;
  open: boolean;
  onToggle: (open: boolean) => void;
  children: ReactNode;
}) {
  return (
    <section className={`collapse${open ? " is-open" : ""}`}>
      <button
        type="button"
        className="collapse-head"
        aria-expanded={open}
        onClick={() => onToggle(!open)}
      >
        <span className="collapse-caret" aria-hidden="true">
          {open ? <IconChevronRight size={12} style={{ transform: "rotate(90deg)" }} /> : <IconChevronRight size={12} />}
        </span>
        <span className="collapse-title">{title}</span>
        {badge}
        {note ? <span className="collapse-note">{note}</span> : null}
      </button>
      {open ? <div className="collapse-body">{children}</div> : null}
    </section>
  );
}

/** 分页：keyset 翻页的控制器。不知道总行数时用页码计靠「还有更多」而不是总数。 */
export function Pagination({
  page,
  hasPrev,
  hasNext,
  onChange,
  onFirst,
}: {
  page: number;
  hasPrev: boolean;
  hasNext: boolean;
  onChange: (page: number) => void;
  onFirst?: () => void;
}) {
  return (
    <div className="pagination" role="navigation" aria-label="分页">
      <button
        type="button"
        className="btn btn-sm page-btn"
        disabled={!hasPrev}
        onClick={() => onChange(page - 1)}
      >
        <IconChevronLeft size={12} /> 上一页
      </button>
      {onFirst ? (
        <button
          type="button"
          className="btn btn-sm page-btn"
          disabled={page === 0}
          onClick={onFirst}
        >
          回到最新
        </button>
      ) : null}
      <span className="page-num faint">第 {page} 页</span>
      <button
        type="button"
        className="btn btn-sm page-btn"
        disabled={!hasNext}
        onClick={() => onChange(page + 1)}
      >
        下一页 <IconChevronRight size={12} />
      </button>
    </div>
  );
}

/** 筛选下拉：原生 select 包一层与搜索框一致的样式。 */
export function FilterSelect({
  value,
  onChange,
  options,
  label,
  disabled = false,
  title,
}: {
  value: string;
  onChange: (value: string) => void;
  options: { value: string; label: string }[];
  label: string;
  disabled?: boolean;
  title?: string;
}) {
  return (
    <label className={`field-select${disabled ? " is-disabled" : ""}`} title={title}>
      <span className="sr-only">{label}</span>
      <select
        value={value}
        disabled={disabled}
        aria-label={label}
        onChange={(event) => onChange(event.target.value)}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </label>
  );
}
