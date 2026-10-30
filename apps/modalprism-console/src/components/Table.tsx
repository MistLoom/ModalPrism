import { useMemo, useState, type ReactNode } from "react";

import { IconChevronDown } from "./icons";

export type SortDirection = "asc" | "desc";

export interface SortState {
  key: string;
  direction: SortDirection;
}

/**
 * 表格排序的可复用实现：列头点击切换 升序 → 降序 → 取消。
 * 排序函数保持稳定：相同键时保留原有顺序。
 */
export function useSort(initial?: SortState) {
  const [sort, setSort] = useState<SortState | null>(initial ?? null);
  const toggle = (key: string) => {
    setSort((current) => {
      if (!current || current.key !== key) return { key, direction: "asc" };
      if (current.direction === "asc") return { key, direction: "desc" };
      return null;
    });
  };
  return { sort, toggle };
}

export function sortRows<T>(rows: T[], sort: SortState | null, valueOf: (row: T, key: string) => string | number | null | undefined): T[] {
  if (!sort) return rows;
  const { key, direction } = sort;
  const factor = direction === "asc" ? 1 : -1;
  return [...rows].sort((left, right) => {
    const a = valueOf(left, key);
    const b = valueOf(right, key);
    if (a === b) return 0;
    if (a === null || a === undefined) return 1;
    if (b === null || b === undefined) return -1;
    if (typeof a === "number" && typeof b === "number") return (a - b) * factor;
    return String(a).localeCompare(String(b), "zh-CN") * factor;
  });
}

export function SortHeader({
  label,
  sortKey,
  sort,
  onSort,
  align,
  width,
}: {
  label: ReactNode;
  sortKey: string;
  sort: SortState | null;
  onSort: (key: string) => void;
  align?: "right";
  width?: number;
}) {
  const active = sort?.key === sortKey;
  return (
    <th style={{ width, textAlign: align }}>
      <button
        type="button"
        className={`sort-head${active ? " is-active" : ""}`}
        onClick={() => onSort(sortKey)}
        aria-label={`按${typeof label === "string" ? label : sortKey}排序`}
      >
        <span>{label}</span>
        <span
          className="sort-arrow"
          style={{
            transform: active && sort?.direction === "asc" ? "rotate(180deg)" : undefined,
            opacity: active ? 1 : 0.35,
          }}
        >
          <IconChevronDown size={12} />
        </span>
      </button>
    </th>
  );
}

/**
 * 高级筛选：把不常用的筛选条件收进一个可折叠面板，主工具条只留搜索。
 * `active` 用于在折叠状态下提示「有 N 项筛选生效」。
 */
export function FilterPanel({
  children,
  active = 0,
  onReset,
}: {
  children: ReactNode;
  active?: number;
  onReset?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const label = useMemo(() => (active > 0 ? `高级筛选 · ${active} 项生效` : "高级筛选"), [active]);
  return (
    <div className="filter-panel">
      <button type="button" className={`btn btn-sm${open ? " is-active" : ""}`} onClick={() => setOpen((value) => !value)} aria-expanded={open}>
        {label}
        <span className="sort-arrow" style={{ transform: open ? "rotate(180deg)" : undefined }}>
          <IconChevronDown size={12} />
        </span>
      </button>
      {open ? (
        <div className="filter-panel-body">
          {children}
          {onReset ? (
            <button type="button" className="btn btn-ghost btn-sm" onClick={onReset}>
              清除筛选
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/** 状态分段的通用渲染。 */
export function Seg<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  label?: string;
}) {
  return (
    <div className="seg" role="group" aria-label={label}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          className={value === option.value ? "is-active" : ""}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
