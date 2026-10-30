import type { ReactNode } from "react";

import { openSubject, type Subject } from "../lib/subject";

/**
 * 列表主标题按钮：标题本身就是入口，键盘可达（Enter/Space），带焦点环。
 * 页面只需要把原来的标题 `<span>` 换成它，交互口径就跟着统一了。
 */
export function RowSubject({ onClick, children, title }: {
  onClick: () => void;
  children: ReactNode;
  title?: string;
}) {
  return (
    <button type="button" className="row-subject" onClick={onClick} title={title}>
      {children}
    </button>
  );
}

/**
 * 统一的列表行：选择框 / 主体 / 元信息 / 操作。
 *
 * 主体一定是 `<button>`：可以用键盘（Enter/Space）打开，也带 `aria-current`；
 * 打开行为统一走 `openSubject`，页面不再各写一套「只有详情按钮能点」的规则。
 */
export function ListRow({ subject, title, meta, actions, selected, onSelect, onOpen, leading }: {
  subject: Subject;
  title: ReactNode;
  meta?: ReactNode;
  actions?: ReactNode;
  selected?: boolean;
  onSelect?: (checked: boolean) => void;
  /** 就地打开（抽屉）；缺省按主体类型跳页。 */
  onOpen?: (subject: Subject) => void;
  /** 行首附加内容（例如徽标）。 */
  leading?: ReactNode;
}) {
  return (
    <div className={`list-row${selected ? " is-selected" : ""}`}>
      {onSelect ? (
        <input
          type="checkbox"
          aria-label={`选择 ${subject.title ?? subject.id}`}
          checked={Boolean(selected)}
          onChange={(event) => onSelect(event.target.checked)}
        />
      ) : null}
      {leading}
      <span className="list-main">
        <button
          type="button"
          className="row-subject"
          onClick={() => openSubject(subject, onOpen)}
          aria-label={`打开 ${subject.title ?? subject.id}`}
        >
          {title}
        </button>
        {meta ? <span className="list-meta">{meta}</span> : null}
      </span>
      {actions ? <span className="row-actions">{actions}</span> : null}
    </div>
  );
}
