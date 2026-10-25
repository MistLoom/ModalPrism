// 多选：一组标识的选中状态。列表页各持有一份，互不影响。
import { useMemo, useState } from "react";

export interface Selection<T extends string> {
  set: Set<T>;
  size: number;
  has: (id: T) => boolean;
  toggle: (id: T) => void;
  clear: () => void;
  /** 选中的标识，顺序即点选顺序。 */
  list: () => T[];
}

export function useSelection<T extends string>(): Selection<T> {
  const [set, setSet] = useState<Set<T>>(new Set());
  return useMemo(
    () => ({
      set,
      size: set.size,
      has: (id: T) => set.has(id),
      toggle: (id: T) =>
        setSet((current) => {
          const next = new Set(current);
          if (next.has(id)) {
            next.delete(id);
          } else {
            next.add(id);
          }
          return next;
        }),
      clear: () => setSet(new Set()),
      list: () => [...set],
    }),
    [set],
  );
}
