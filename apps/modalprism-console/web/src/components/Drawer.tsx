import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

import { IconClose, IconTop } from "./icons";

export function Drawer({
  open,
  kicker,
  title,
  onClose,
  children,
  footer,
}: {
  open: boolean;
  kicker?: string;
  title: ReactNode;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
}) {
  const panel = useRef<HTMLDivElement | null>(null);
  const body = useRef<HTMLDivElement | null>(null);
  const restore = useRef<HTMLElement | null>(null);
  const [scrolled, setScrolled] = useState(false);

  // onClose 只保存在 ref 里：父组件每轮轮询都会传入新的闭包，
  // 若让它进入依赖，下面的 effect 会反复重跑，把抽屉滚动位置重置回顶部造成闪烁。
  const onCloseRef = useRef(onClose);
  useEffect(() => {
    onCloseRef.current = onClose;
  }, [onClose]);

  // 只在 open 变化时执行一次：滚动归零、焦点迁移、锁页面滚动、挂键盘监听。
  useEffect(() => {
    if (!open) {
      return;
    }
    body.current?.scrollTo({ top: 0 });
    setScrolled(false);
    restore.current = document.activeElement as HTMLElement | null;
    panel.current?.focus();
    const previous = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        onCloseRef.current();
      }
      if (event.key === "Tab" && panel.current) {
        const focusable = panel.current.querySelectorAll<HTMLElement>(
          'button:not([disabled]), [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
        );
        if (!focusable.length) {
          return;
        }
        const first = focusable[0];
        const last = focusable[focusable.length - 1];
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first.focus();
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      document.body.style.overflow = previous;
      restore.current?.focus();
    };
  }, [open]);

  const onScroll = () => {
    if (body.current) {
      setScrolled(body.current.scrollTop > 240);
    }
  };

  if (!open) {
    return null;
  }

  // portal 到 body：避免页面入场动画的 transform 祖先劫持 fixed 定位。
  return createPortal(
    <div className="layer">
      <div className="layer-scrim" onClick={onClose} aria-hidden="true" />
      <div
        className={`drawer${footer ? " has-foot" : ""}`}
        role="dialog"
        aria-modal="true"
        aria-label={typeof title === "string" ? title : "详情"}
        tabIndex={-1}
        ref={panel}
      >
        <header className="drawer-head">
          <div className="drawer-head-text">
            {kicker ? <span className="drawer-kicker">{kicker}</span> : null}
            <h2 className="drawer-title">{title}</h2>
          </div>
          <button className="btn btn-quiet btn-icon" type="button" onClick={onClose} aria-label="关闭">
            <IconClose />
          </button>
        </header>
        <div className="drawer-body" ref={body} onScroll={onScroll}>
          {children}
        </div>
        {footer ? <footer className="drawer-foot">{footer}</footer> : null}
        {scrolled ? (
          <button
            type="button"
            className="drawer-top"
            aria-label="回到顶部"
            onClick={() => body.current?.scrollTo({ top: 0, behavior: "smooth" })}
          >
            <IconTop size={14} />
          </button>
        ) : null}
      </div>
    </div>,
    document.body,
  );
}
