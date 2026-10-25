import { useEffect, useRef } from "react";

import { CommandPalette } from "./components/CommandPalette";
import { Toasts } from "./components/Toasts";
import {
  IconAlert,
  IconChannel,
  IconChevronLeft,
  IconChevronRight,
  IconConfig,
  IconFlow,
  IconLayers,
  IconLibrary,
  IconOverview,
  IconPlan,
  IconQueue,
  IconSearch,
  IconShield,
  IconSource,
} from "./components/icons";
import { useConsole } from "./lib/console";
import { clockTime, relative } from "./lib/format";
import { navigate, ROUTES, useRoute, type RouteId } from "./lib/router";
import { ChannelsPage } from "./pages/Channels";
import { ConfigPage } from "./pages/Config";
import { GovernancePage } from "./pages/Governance";
import { LibraryPage } from "./pages/Library";
import { OverviewPage } from "./pages/Overview";
import { PlansPage } from "./pages/Plans";
import { SourcesPage } from "./pages/Sources";
import { StrataPage } from "./pages/Strata";
import { TasksPage } from "./pages/Tasks";
import { WorkflowsPage } from "./pages/Workflows";

const NAV_ICONS: Record<RouteId, React.ReactNode> = {
  overview: <IconOverview />,
  sources: <IconSource />,
  plans: <IconPlan />,
  tasks: <IconQueue />,
  strata: <IconLayers />,
  library: <IconLibrary />,
  workflows: <IconFlow />,
  governance: <IconShield />,
  channels: <IconChannel />,
  config: <IconConfig />,
};

const GROUPS: { label: string; routes: RouteId[] }[] = [
  { label: "观测 · observe", routes: ["overview", "sources", "plans", "tasks", "strata", "library"] },
  { label: "加工 · process", routes: ["workflows"] },
  { label: "系统 · system", routes: ["governance", "channels", "config"] },
];

export default function App() {
  const { route } = useRoute();
  const { sources, state, busy, now, timeline, cursor, setCursor, liveCursor, setPalette, snapshot, connection } =
    useConsole();
  const view = useRef<HTMLElement | null>(null);
  const alerts = sources.filter((source) => source.enabled && (source.alert || source.health !== "ok")).length;
  const generation = snapshot?.catalog.generation ?? null;

  useEffect(() => {
    view.current?.scrollTo({ top: 0 });
  }, [route]);

  return (
    <div className="shell">
      <aside className="rail" aria-label="主导航">
        <div className="brand">
          <svg className="brand-mark" viewBox="0 0 32 32" aria-hidden="true">
            <path d="M16 3.5 28 26H4L16 3.5Z" fill="none" stroke="currentColor" strokeWidth="1.2" />
            <path
              d="M16 3.5v22.5M10 14.8h12M7 21h18"
              fill="none"
              stroke="currentColor"
              strokeWidth=".8"
              opacity=".45"
            />
            <path d="M16 3.5 22 26" stroke="var(--cinnabar)" strokeWidth="1.3" fill="none" />
          </svg>
          <div className="brand-text">
            <span className="brand-name">ModalPrism</span>
            <span className="brand-sub">采集控制台</span>
          </div>
        </div>

        <nav className="nav">
          {GROUPS.map((group) => (
            <div className="nav-group" key={group.label}>
              <span className="nav-label">{group.label}</span>
              {group.routes.map((id) => {
                const item = ROUTES.find((entry) => entry.id === id)!;
                const badge =
                  id === "sources" && alerts > 0 ? (
                    <span className="nav-badge is-alert" title={`${alerts} 个信源需要留意`}>
                      {alerts}
                    </span>
                  ) : id === "sources" ? (
                    <span className="nav-badge">{sources.length}</span>
                  ) : null;
                return (
                  <button
                    key={id}
                    type="button"
                    className={`nav-item${route === id ? " is-active" : ""}`}
                    onClick={() => navigate(id)}
                    aria-current={route === id ? "page" : undefined}
                    title={item.label}
                  >
                    <span className="nav-icon">{NAV_ICONS[id]}</span>
                    <span className="nav-text">{item.label}</span>
                    {badge}
                  </button>
                );
              })}
            </div>
          ))}
        </nav>

        <div className="rail-foot">
          <div className="glyph" aria-hidden="true">
            <span />
            <span />
            <span />
            <span />
            <span />
          </div>
          <div className="foot-row">
            <span>配置代际</span>
            <strong>G-{generation ?? "—"}</strong>
          </div>
          <div className="foot-row">
            <span>快照游标</span>
            <strong>#{liveCursor ?? "—"}</strong>
          </div>
          <div className="foot-row">
            <span>采集进程</span>
            <strong>{connection?.online ? connection.endpoint.replace(/^https?:\/\//, "") : "未连接"}</strong>
          </div>
          <p className="foot-note">控制台 · 写操作直接写回配置目录，热加载生效</p>
        </div>
      </aside>

      <div className="main">
        <header className="topbar">
          <div className="crumb">
            <span>控制台</span>
            <span className="crumb-sep" aria-hidden="true">
              /
            </span>
            <strong>{ROUTES.find((item) => item.id === route)?.label}</strong>
          </div>
          <button
            type="button"
            className="palette-trigger"
            onClick={() => setPalette(true)}
            aria-haspopup="dialog"
            aria-label="打开命令面板"
          >
            <IconSearch size={13} />
            <span className="trigger-text">搜索信源、页面或动作…</span>
            <kbd>Ctrl K</kbd>
          </button>
          <div className="top-meta">
            <span
              className={`live${state === "offline" ? " is-offline" : state === "error" ? " is-error" : ""}${busy ? " is-busy" : ""}`}
            >
              <span className="live-dot" aria-hidden="true" />
              {state === "offline" ? "已断开" : state === "error" ? "读取异常" : busy ? "同步中" : "实时"}
            </span>
            <time className="clock">{clockTime(new Date(now))}</time>
            <button
              type="button"
              className="gen-chip"
              onClick={() => navigate("config")}
              title="查看配置"
            >
              G-{generation ?? "—"}
            </button>
          </div>
        </header>

        <main className="view" ref={view} tabIndex={-1}>
          {state === "offline" || state === "error" ? <OfflineBanner /> : null}
          {route === "overview" ? <OverviewPage /> : null}
          {route === "sources" ? <SourcesPage /> : null}
          {route === "plans" ? <PlansPage /> : null}
          {route === "tasks" ? <TasksPage /> : null}
          {route === "strata" ? <StrataPage /> : null}
          {route === "library" ? <LibraryPage /> : null}
          {route === "workflows" ? <WorkflowsPage /> : null}
          {route === "governance" ? <GovernancePage /> : null}
          {route === "channels" ? <ChannelsPage /> : null}
          {route === "config" ? <ConfigPage /> : null}
        </main>
      </div>

      <footer className="snapbar" aria-label="快照游标">
        <div className="snap-left">
          <span className="snap-label">SNAPSHOT</span>
          <strong className="snap-id">#{cursor ?? liveCursor ?? "—"}</strong>
          <span className="snap-time">
            {cursor !== null
              ? (timeline.find((item) => item.snapshot === cursor)?.publishedAt
                  ? relative(timeline.find((item) => item.snapshot === cursor)!.publishedAt)
                  : "历史")
              : liveCursor !== null
                ? relative(timeline[timeline.length - 1]?.publishedAt ?? null)
                : "尚未提交"}
          </span>
          <span className={`snap-mode${cursor !== null ? " is-historic" : ""}`}>
            {cursor !== null ? "历史" : "实时"}
          </span>
        </div>
        <div className="snap-track-wrap">
          <button
            type="button"
            className="snap-step"
            aria-label="更早的快照"
            disabled={!timeline.length || cursor === timeline[0]?.snapshot}
            onClick={() => step(-1)}
          >
            <IconChevronLeft size={13} />
          </button>
          <div className="snap-track" role="group" aria-label="选择快照">
            {timeline.length === 0 ? (
              <span className="snap-hint">还没有可回放的提交</span>
            ) : (
              timeline.map((commit) => (
                <button
                  key={commit.snapshot}
                  type="button"
                  className={`snap-dot${
                    (cursor ?? liveCursor) === commit.snapshot ? " is-current" : ""
                  }${commit.snapshot === liveCursor ? " is-live" : " is-past"}`}
                  title={`快照 #${commit.snapshot} · ${commit.commitIds.length} 个变更集 · ${relative(commit.publishedAt)}`}
                  aria-label={`快照 ${commit.snapshot}`}
                  onClick={() => setCursor(commit.snapshot === liveCursor ? null : commit.snapshot)}
                />
              ))
            )}
          </div>
          <button
            type="button"
            className="snap-step"
            aria-label="更晚的快照"
            disabled={!timeline.length || (cursor ?? liveCursor) === liveCursor}
            onClick={() => step(1)}
          >
            <IconChevronRight size={13} />
          </button>
        </div>
        <div className="snap-right">
          {cursor !== null ? (
            <button type="button" className="btn btn-quiet btn-sm" onClick={() => setCursor(null)}>
              回到实时
            </button>
          ) : null}
          <span className="snap-hint mono">读取按快照进行</span>
        </div>
      </footer>

      <CommandPalette />
      <Toasts />
    </div>
  );

  function step(direction: number) {
    if (!timeline.length) {
      return;
    }
    const current = cursor ?? liveCursor ?? timeline[timeline.length - 1].snapshot;
    const index = timeline.findIndex((item) => item.snapshot === current);
    const next = timeline[Math.max(0, Math.min(timeline.length - 1, index + direction))];
    if (next) {
      setCursor(next.snapshot === liveCursor ? null : next.snapshot);
    }
  }
}

function OfflineBanner() {
  const { state, error, connection, refresh, busy } = useConsole();
  return (
    <div className="note is-bad" style={{ marginBottom: 18 }}>
      <IconAlert size={15} />
      <div style={{ display: "grid", gap: 4 }}>
        <strong>{state === "offline" ? "采集进程没有响应" : "读取数据时出错"}</strong>
        <span>
          {state === "offline"
            ? `控制台正在等待 ${connection?.endpoint ?? "运维端口"}。先启动 modalprism-collector，再点右侧按钮重试。界面会保留上一次的数据结构，但不会展示陈旧数值。`
            : (error ?? "未知错误")}
        </span>
        <span style={{ display: "flex", gap: 8, marginTop: 4 }}>
          <button type="button" className="btn btn-sm" onClick={refresh} disabled={busy}>
            {busy ? "正在重试…" : "重新连接"}
          </button>
          <button type="button" className="btn btn-sm btn-quiet" onClick={() => navigate("config")}>
            查看配置说明
          </button>
        </span>
      </div>
    </div>
  );
}
