// 批量条：选中多行后出现，操作结果由页面统一提示。
import type { ReactNode } from "react";

export interface BatchAction {
  key: string;
  label: string;
  icon?: ReactNode;
  /** 破坏性操作用红色描边。 */
  danger?: boolean;
  /** 有值时先弹二次确认，文案就是它。 */
  confirm?: string;
  onClick: () => void;
}

export function BatchBar({
  count,
  busy,
  actions,
  onClear,
}: {
  count: number;
  busy: boolean;
  actions: BatchAction[];
  onClear: () => void;
}) {
  if (count === 0) {
    return null;
  }
  return (
    <div className="batch-bar" role="region" aria-label="批量操作">
      <span className="batch-count">
        已选 <strong>{count}</strong> 项
      </span>
      {actions.map((action) => (
        <button
          key={action.key}
          type="button"
          className={action.danger ? "btn btn-sm btn-danger" : "btn btn-sm"}
          disabled={busy}
          onClick={() => {
            if (action.confirm && !window.confirm(action.confirm)) {
              return;
            }
            action.onClick();
          }}
        >
          {action.icon} {action.label}
        </button>
      ))}
      <span className="spacer" />
      {busy ? <span className="faint">正在写入…</span> : null}
      <button type="button" className="btn btn-sm btn-quiet" disabled={busy} onClick={onClear}>
        取消选择
      </button>
    </div>
  );
}
