import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  IconAlert,
  IconBell,
  IconChevronRight,
  IconContent,
  IconDelivery,
  IconLogs,
  IconMenu,
  IconOverview,
  IconPlus,
  IconPlug,
  IconSearch,
  IconSettings,
  IconSources,
  IconTasks,
} from "./components/icons";
import { Field, Loading, Modal, useUi } from "./components/ui";
import { createProject, systemOverview, systemStatus } from "./lib/api";
import { ProjectProvider, useCurrentProject } from "./lib/project";
import { navigate, pageLabel, ROUTES, useRoute, type PageId } from "./lib/router";
import { ContentPage } from "./pages/Content";
import { DeliveryPage } from "./pages/Delivery";
import { ExternalPage } from "./pages/External";
import { LogsPage } from "./pages/Logs";
import { NotificationsPage, useDesktopNotifications } from "./pages/Notifications";
import { OverviewPage } from "./pages/Overview";
import { PlansPage } from "./pages/Plans";
import { ProjectsPage } from "./pages/Projects";
import { SettingsPage } from "./pages/Settings";
import { SourcesPage } from "./pages/Sources";
import { TasksPage } from "./pages/Tasks";
import { ToolsPage } from "./pages/Tools";

const NAV_ICONS: Record<PageId, React.ReactNode> = {
  overview: <IconOverview />,
  projects: <IconSources />,
  sources: <IconSources />,
  plans: <IconTasks />,
  tasks: <IconTasks />,
  content: <IconContent />,
  delivery: <IconDelivery />,
  notifications: <IconBell />,
  external: <IconPlug />,
  logs: <IconLogs />,
  tools: <IconSearch />,
  settings: <IconSettings />,
};

interface Badges {
  sources: number;
  attention: number;
  tasks: number;
}

export default function App() {
  return (
    <ProjectProvider>
      <Shell />
    </ProjectProvider>
  );
}

/// 外壳：项目上下文就绪后再渲染，切项目时以 key 重挂载页面，确保各页数据随之刷新。
function Shell() {
  const { page } = useRoute();
  const { currentProjectId } = useCurrentProject();
  useDesktopNotifications();
  const [online, setOnline] = useState<boolean | null>(null);
  const [generation, setGeneration] = useState<number | null>(null);
  const [badges, setBadges] = useState<Badges>({ sources: 0, attention: 0, tasks: 0 });
  const [palette, setPalette] = useState(false);
  const [menu, setMenu] = useState(false);
  const view = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    let alive = true;
    const load = async () => {
      try {
        const status = await systemStatus();
        if (!alive) return;
        setOnline(true);
        setGeneration(status.generation);
      } catch {
        if (alive) setOnline(false);
        return;
      }
      try {
        const overview = await systemOverview();
        if (!alive) return;
        setBadges({
          sources: overview.counts.sources,
          attention: overview.counts.attention,
          tasks: overview.counts.tasksQueued + overview.counts.tasksRunning,
        });
      } catch {
        // 概览失败不改变在线状态。
      }
    };
    void load();
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void load();
    }, 15_000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [currentProjectId]);

  useEffect(() => {
    view.current?.scrollTo({ top: 0 });
    setMenu(false);
  }, [page]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPalette((open) => !open);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="shell">
      <aside className={`sidebar${menu ? " is-open" : ""}`} aria-label="主导航">
        <div className="brand">
          <svg className="brand-mark" viewBox="0 0 32 32" aria-hidden="true">
            <path d="M16 3.5 28 26H4L16 3.5Z" fill="none" stroke="currentColor" strokeWidth={1.2} />
            <path d="M16 3.5v22.5M10 14.8h12M7 21h18" fill="none" stroke="currentColor" strokeWidth={0.8} opacity={0.4} />
            <path d="M16 3.5 22 26" stroke="var(--accent)" strokeWidth={1.3} fill="none" />
          </svg>
          <div className="brand-text">
            <span className="brand-name">ModalPrism</span>
            <span className="brand-sub">Content desk</span>
          </div>
        </div>
        <nav className="nav">
          <span className="nav-label">工作区</span>
          {ROUTES.filter((route) => route.group === "workspace").map((route) => (
            <NavItem
              key={route.id}
              id={route.id}
              active={page === route.id}
              badge={
                route.id === "sources" && badges.sources ? (
                  <span className={`nav-badge${badges.attention ? " is-alert" : ""}`}>{badges.attention || badges.sources}</span>
                ) : route.id === "tasks" && badges.tasks ? (
                  <span className="nav-badge">{badges.tasks}</span>
                ) : null
              }
            />
          ))}
          <span className="nav-label">诊断与接入</span>
          {ROUTES.filter((route) => route.group === "diagnostics").map((route) => (
            <NavItem key={route.id} id={route.id} active={page === route.id} />
          ))}
        </nav>
        <div className="rail-foot">
          <span>
            配置代际 <strong>{generation === null ? "—" : `G-${generation}`}</strong>
          </span>
          <span>仅本机访问 · 无远程认证</span>
        </div>
      </aside>

      <div className="main">
        <header className="topbar">
          <button type="button" className="icon-btn mobile-toggle" onClick={() => setMenu((open) => !open)} aria-label="打开导航">
            <IconMenu />
          </button>
          <div className="crumbs">
            <span>控制台</span>
            <IconChevronRight size={12} />
            <strong>{pageLabel(page)}</strong>
          </div>
          <button type="button" className="search-trigger" onClick={() => setPalette(true)}>
            <IconSearch size={15} />
            <span>搜索信源、内容、任务、运行与动作</span>
            <kbd>Ctrl K</kbd>
          </button>
          <div className="topbar-meta">
            <CurrentProjectPicker />
            <QuickProjectButton />
            <span className="conn">
              <span className={`dot ${online === false ? "bad" : online ? "ok" : "warn"}`} />
              {online === false ? "collector 不可达" : online ? "在线" : "连接中"}
            </span>
            <span className="hide-small mono">G-{generation ?? "—"}</span>
          </div>
        </header>
        <div className="view" ref={view}>
          {online === false ? (
            <div className="offline-bar" role="status">
              <IconAlert size={16} />
              <span>
                collector 不可达。只读外壳仍可打开，但不会伪造业务数据；确认 <code>modalprism-collector</code> 已启动后会自动恢复。
              </span>
            </div>
          ) : null}
          <Page key={currentProjectId ?? "pending"} page={page} online={online} />
        </div>
      </div>

      {palette ? <Palette onClose={() => setPalette(false)} /> : null}
    </div>
  );
}

/// 顶栏当前项目选择器：全局项目上下文落点，切换后各页按项目过滤。
function CurrentProjectPicker() {
  const { projects, currentProject, setCurrentProjectId, loading } = useCurrentProject();
  if (loading && !projects.length) return null;
  if (!projects.length) return null;
  return (
    <select
      className="input project-picker"
      aria-label="当前项目"
      value={currentProject?.id ?? ""}
      onChange={(event) => setCurrentProjectId(event.target.value)}
      style={{ width: 160 }}
    >
      {projects.map((project) => (
        <option key={project.id} value={project.id}>
          {project.name}
          {project.isDefault ? "（默认）" : ""}
        </option>
      ))}
    </select>
  );
}

/// 顶栏快速新建项目：建完即切换为当前项目，页面随项目切换重挂载刷新。
function QuickProjectButton() {
  const { notify } = useUi();
  const { reload, setCurrentProjectId } = useCurrentProject();
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit() {
    const trimmed = name.trim();
    if (!trimmed || busy) return;
    setBusy(true);
    try {
      const project = await createProject({ name: trimmed });
      await reload();
      setCurrentProjectId(project.id);
      notify("ok", "项目已创建并切换", `${project.name} · 新建的信源与运行数据自动归入本项目`);
      setName("");
      setOpen(false);
    } catch (problem) {
      notify("bad", "创建项目失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <button type="button" className="btn btn-sm" onClick={() => setOpen(true)} title="新建项目">
        <IconPlus size={13} /> 项目
      </button>
      {open ? (
        <Modal
          title="新建项目"
          sub="只填项目名即可；之后新建的信源、计划与内容自动归入本项目。"
          onClose={() => setOpen(false)}
          footer={
            <>
              <button type="button" className="btn" onClick={() => setOpen(false)}>取消</button>
              <button type="button" className="btn btn-primary" onClick={() => void submit()} disabled={busy || !name.trim()}>
                {busy ? "创建中…" : "创建并切换"}
              </button>
            </>
          }
        >
          <Field label="项目名" htmlFor="quick-project-name">
            <input
              id="quick-project-name"
              className="input"
              value={name}
              placeholder="例如：科技快讯"
              onChange={(event) => setName(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") void submit();
              }}
            />
          </Field>
        </Modal>
      ) : null}
    </>
  );
}

function NavItem({ id, active, badge }: { id: PageId; active: boolean; badge?: React.ReactNode }) {
  const route = ROUTES.find((item) => item.id === id)!;
  return (
    <button
      type="button"
      className={`nav-item${active ? " is-active" : ""}`}
      aria-current={active ? "page" : undefined}
      onClick={() => navigate(id)}
      title={route.label}
    >
      <span className="nav-icon">{NAV_ICONS[id]}</span>
      <span className="nav-text">{route.label}</span>
      {badge}
    </button>
  );
}

function Page({ page, online }: { page: PageId; online: boolean | null }) {
  if (online === false) {
    return (
      <div className="page">
        <header className="page-head">
          <div className="page-head-text">
            <div className="kicker">{page} · 离线</div>
            <h1 className="page-title">{pageLabel(page)}</h1>
            <p className="page-lede">Node 进程可达但 collector 未就绪。恢复连接后本页会重新读取业务数据。</p>
          </div>
        </header>
        <div className="card">
          <div className="card-body">
            <Loading lines={3} />
          </div>
        </div>
      </div>
    );
  }
  switch (page) {
    case "overview":
      return <OverviewPage />;
    case "projects":
      return <ProjectsPage />;
    case "sources":
      return <SourcesPage />;
    case "plans":
      return <PlansPage />;
    case "tasks":
      return <TasksPage />;
    case "content":
      return <ContentPage />;
    case "delivery":
      return <DeliveryPage />;
    case "notifications":
      return <NotificationsPage />;
    case "external":
      return <ExternalPage />;
    case "logs":
      return <LogsPage />;
    case "tools":
      return <ToolsPage />;
    case "settings":
      return <SettingsPage />;
  }
}

function Palette({ onClose }: { onClose: () => void }) {
  const [text, setText] = useState("");
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement | null>(null);

  const items = useMemo(
    () => [
      ...ROUTES.map((route) => ({ label: `打开${route.label}`, hint: route.kicker, action: () => navigate(route.id) })),
      { label: "查看失败任务", hint: "过滤任务", action: () => navigate("tasks?status=failed") },
      { label: "打开内容库", hint: "检索与删除", action: () => navigate("content") },
      { label: "手动投递内容", hint: "选择目标并提交", action: () => navigate("delivery") },
      { label: "查看投递记录", hint: "内容投递", action: () => navigate("delivery?tab=records") },
      { label: "查看通知记录", hint: "通知管理", action: () => navigate("notifications?tab=records") },
      { label: "查看最近错误日志", hint: "过滤日志", action: () => navigate("logs?level=error") },
      { label: "打开系统设置", hint: "设置", action: () => navigate("settings") },
    ],
    [],
  );

  const matches = useMemo(() => {
    const needle = text.trim().toLowerCase();
    if (!needle) return items;
    return items.filter((item) => `${item.label} ${item.hint}`.toLowerCase().includes(needle));
  }, [items, text]);

  useEffect(() => {
    input.current?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    document.body.style.overflow = "hidden";
    return () => {
      window.removeEventListener("keydown", onKey);
      document.body.style.overflow = "";
    };
  }, [onClose]);

  const run = useCallback(
    (index: number) => {
      const item = matches[index];
      if (!item) return;
      onClose();
      item.action();
    },
    [matches, onClose],
  );

  return (
    <>
      <div className="cmd-scrim" onClick={onClose} aria-hidden="true" />
      <div className="cmd" role="dialog" aria-modal="true" aria-label="命令面板">
        <input
          ref={input}
          value={text}
          placeholder="输入页面或动作…"
          onChange={(event) => {
            setText(event.target.value);
            setActive(0);
          }}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              setActive((index) => Math.min(matches.length - 1, index + 1));
            } else if (event.key === "ArrowUp") {
              event.preventDefault();
              setActive((index) => Math.max(0, index - 1));
            } else if (event.key === "Enter") {
              event.preventDefault();
              run(active);
            }
          }}
        />
        <div className="cmd-list">
          {matches.length === 0 ? <div className="empty">没有匹配的动作</div> : null}
          {matches.map((item, index) => (
            <button
              key={item.label}
              type="button"
              className={`cmd-item${index === active ? " is-active" : ""}`}
              onMouseEnter={() => setActive(index)}
              onClick={() => run(index)}
            >
              <IconSearch size={15} />
              <span className="fill">{item.label}</span>
              <span className="muted small">{item.hint}</span>
            </button>
          ))}
        </div>
      </div>
    </>
  );
}
