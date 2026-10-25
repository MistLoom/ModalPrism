import { useEffect, useMemo, useRef, useState } from "react";

import { useConsole } from "../lib/console";
import { navigate, ROUTES } from "../lib/router";
import {
  IconAlert,
  IconChannel,
  IconCommand,
  IconConfig,
  IconFlow,
  IconLibrary,
  IconOverview,
  IconPlay,
  IconRefresh,
  IconSearch,
  IconSource,
} from "./icons";
import type { Source } from "../lib/types";

interface Item {
  key: string;
  group: string;
  label: string;
  hint?: string;
  icon: React.ReactNode;
  run: () => void;
}

const ICONS: Record<string, React.ReactNode> = {
  overview: <IconOverview size={14} />,
  sources: <IconSource size={14} />,
  library: <IconLibrary size={14} />,
  workflows: <IconFlow size={14} />,
  channels: <IconChannel size={14} />,
  config: <IconConfig size={14} />,
};

export function CommandPalette() {
  const { palette, setPalette, sources, refresh, notify, cursor, setCursor } = useConsole();
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (palette) {
      setQuery("");
      setActive(0);
      window.setTimeout(() => input.current?.focus(), 20);
      const previous = document.body.style.overflow;
      document.body.style.overflow = "hidden";
      return () => {
        document.body.style.overflow = previous;
      };
    }
    return undefined;
  }, [palette]);

  const items = useMemo<Item[]>(() => {
    const nav: Item[] = ROUTES.map((route) => ({
      key: `nav-${route.id}`,
      group: "前往",
      label: route.label,
      hint: route.hint,
      icon: ICONS[route.id],
      run: () => navigate(route.id),
    }));

    const actions: Item[] = [
      {
        key: "act-refresh",
        group: "动作",
        label: "刷新观测数据",
        hint: "重新读取信源状态",
        icon: <IconRefresh size={14} />,
        run: () => {
          refresh();
          notify("info", "正在重新读取");
        },
      },
      {
        key: "act-live",
        group: "动作",
        label: cursor === null ? "当前已是实时视图" : "回到实时视图",
        hint: "退出历史快照",
        icon: <IconPlay size={14} />,
        run: () => setCursor(null),
      },
      {
        key: "act-config",
        group: "动作",
        label: "校验配置草稿",
        hint: "在配置页执行",
        icon: <IconConfig size={14} />,
        run: () => navigate("config"),
      },
    ];

    const sourceItems: Item[] = sources.map((source: Source) => ({
      key: `src-${source.id}`,
      group: "信源",
      label: source.name,
      hint: source.health === "ok" ? source.accessorLabel : "需要留意",
      icon: source.alert ? <IconAlert size={14} /> : <IconSource size={14} />,
      run: () => navigate(`sources/${encodeURIComponent(source.id)}`),
    }));

    const all = [...nav, ...actions, ...sourceItems];
    const text = query.trim().toLowerCase();
    if (!text) {
      return all;
    }
    return all.filter(
      (item) =>
        item.label.toLowerCase().includes(text) ||
        (item.hint ?? "").toLowerCase().includes(text) ||
        item.key.toLowerCase().includes(text),
    );
  }, [query, sources, refresh, notify, cursor, setCursor]);

  useEffect(() => {
    setActive((index) => Math.min(index, Math.max(0, items.length - 1)));
  }, [items.length]);

  useEffect(() => {
    if (!palette) {
      return;
    }
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setPalette(false);
      }
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setActive((index) => Math.min(index + 1, Math.max(0, items.length - 1)));
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        setActive((index) => Math.max(0, index - 1));
      }
      if (event.key === "Enter") {
        event.preventDefault();
        const item = items[active];
        if (item) {
          item.run();
          setPalette(false);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [palette, items, active, setPalette]);

  useEffect(() => {
    const row = listRef.current?.querySelector<HTMLElement>(`[data-index="${active}"]`);
    row?.scrollIntoView({ block: "nearest" });
  }, [active]);

  if (!palette) {
    return null;
  }

  let lastGroup = "";

  return (
    <div className="palette-layer">
      <div className="layer-scrim" onClick={() => setPalette(false)} aria-hidden="true" />
      <section className="palette" role="dialog" aria-modal="true" aria-label="命令面板">
        <div className="palette-input-row">
          <span style={{ color: "var(--faint)" }}>
            <IconCommand size={15} />
          </span>
          <input
            ref={input}
            className="palette-input"
            value={query}
            placeholder="搜索信源、页面或动作…"
            onChange={(event) => {
              setQuery(event.target.value);
              setActive(0);
            }}
            aria-label="命令面板搜索"
            autoComplete="off"
            spellCheck={false}
          />
          <kbd>ESC</kbd>
        </div>
        <div className="palette-results" ref={listRef} role="listbox" aria-label="结果">
          {items.length === 0 ? (
            <div className="empty" style={{ padding: "26px 12px" }}>
              <p className="empty-title">没有匹配项</p>
              <p className="empty-text">
                试试信源名称，或者输入「配置」「通道」这样的页面名。
              </p>
            </div>
          ) : (
            items.map((item, index) => {
              const showGroup = item.group !== lastGroup;
              lastGroup = item.group;
              return (
                <div key={item.key}>
                  {showGroup ? <div className="palette-group">{item.group}</div> : null}
                  <button
                    type="button"
                    role="option"
                    aria-selected={index === active}
                    data-index={index}
                    className={`palette-item${index === active ? " is-active" : ""}`}
                    onMouseEnter={() => setActive(index)}
                    onClick={() => {
                      item.run();
                      setPalette(false);
                    }}
                  >
                    <span className="item-icon">{item.icon}</span>
                    <span className="item-text">{item.label}</span>
                    {item.hint ? <span className="item-hint">{item.hint}</span> : null}
                  </button>
                </div>
              );
            })
          )}
        </div>
        <div className="palette-foot">
          <span>
            <kbd>↑</kbd>
            <kbd>↓</kbd> 选择
          </span>
          <span>
            <kbd>↵</kbd> 执行
          </span>
          <span>
            <kbd>Ctrl</kbd> <kbd>K</kbd> 唤起
          </span>
          <span style={{ marginLeft: "auto" }}>
            <IconSearch size={11} />
          </span>
        </div>
      </section>
    </div>
  );
}
