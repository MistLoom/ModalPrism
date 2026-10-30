import { useCallback, useEffect, useMemo, useState } from "react";

import { IconChevronRight, IconHistory, IconPlus, IconRefresh } from "../components/icons";
import { RunsDrawer } from "../components/RunsDrawer";
import { Badge, Card, Empty, Field, Loading, Modal, Problem, useUi } from "../components/ui";
import {
  createProject,
  getWizard,
  listCollectionRuns,
  listPlans,
  listSources,
  systemOverview,
} from "../lib/api";
import { relative, stamp } from "../lib/format";
import { useCurrentProject } from "../lib/project";
import { navigate, type PageId } from "../lib/router";
import type { CollectionRun, Plan, Project, Source, WizardDoc } from "../lib/types";

/** 起步清单环节跳转目标：只作导航提示，不阻塞任何操作。 */
const STEP_LINKS: Record<string, { page: PageId; label: string }> = {
  create_project: { page: "projects", label: "本项目" },
  scenario_template: { page: "sources", label: "去信源" },
  configure_sources: { page: "sources", label: "去信源" },
  verify_collection: { page: "tasks", label: "看运行" },
  model_gate: { page: "settings", label: "去模型设置" },
  processing_output: { page: "content", label: "去内容" },
  api_surface: { page: "tools", label: "去调试工具" },
};

interface ProjectStats {
  /** 自有信源（归属本项目的信源；共享引进的另计）。 */
  owned: number;
  sources: number;
  content: number;
  plans: number;
  running: number;
  queued: number;
  lastRunAt?: string;
}

/**
 * 项目页：项目是数据归属的根，所以这里展示的是「一个项目带出多少信源、
 * 多少计划、多少内容与最近一次采集」，而不是资源绑定清单。
 * 每张卡片按项目上下文只读取数，点开即可看该项目的信源、计划与采集记录。
 */
export function ProjectsPage() {
  const { notify } = useUi();
  const { projects, currentProjectId, setCurrentProjectId, reload, loading } = useCurrentProject();
  const [selected, setSelected] = useState<string | null>(currentProjectId);
  const [stats, setStats] = useState<Record<string, ProjectStats>>({});
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [heroRecord, setHeroRecord] = useState<{ sourceId?: string; planId?: string; title: string } | null>(null);

  // 每个项目各自取一次概览、计划总数与最近一次采集：卡片上的数字必须属于它自己。
  useEffect(() => {
    if (!projects.length) return;
    let alive = true;
    void (async () => {
      const entries = await Promise.all(
        projects.map(async (project) => {
          const [overview, plans, runs, sourcePage] = await Promise.all([
            systemOverview(project.id).catch(() => null),
            listPlans({ limit: 200, projectId: project.id }).catch(() => null),
            listCollectionRuns({ limit: 1, projectId: project.id }).catch(() => null),
            listSources({ limit: 200, projectId: project.id }).catch(() => null),
          ]);
          const counts = overview?.counts;
          const owned = (sourcePage?.items ?? []).filter((item) => !item.project || item.project === project.id).length;
          return [
            project.id,
            {
              owned,
              sources: counts?.sources ?? owned,
              content: counts?.content ?? 0,
              plans: plans?.total ?? plans?.items.length ?? 0,
              running: counts?.tasksRunning ?? 0,
              queued: counts?.tasksQueued ?? 0,
              lastRunAt: runs?.items[0]?.createdAt,
            } satisfies ProjectStats,
          ] as const;
        }),
      );
      if (alive) setStats(Object.fromEntries(entries));
    })();
    return () => {
      alive = false;
    };
  }, [projects]);

  useEffect(() => {
    if (!selected || !projects.some((project) => project.id === selected)) {
      setSelected(currentProjectId ?? projects[0]?.id ?? null);
    }
  }, [projects, currentProjectId, selected]);

  const selectedProject = useMemo(
    () => projects.find((project) => project.id === selected) ?? null,
    [projects, selected],
  );

  async function submit() {
    const trimmed = name.trim();
    if (!trimmed || busy) return;
    setBusy(true);
    try {
      const project = await createProject({ name: trimmed });
      setName("");
      setCreating(false);
      await reload();
      // 建完即切入新项目：此后新建的信源、计划与采集记录都归它，页面随项目切换重挂载。
      setCurrentProjectId(project.id);
      notify("ok", "项目已创建并切换", `${project.name} · 新建信源与采集记录归属本项目`);
    } catch (problem) {
      notify("bad", "创建项目失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <div className="kicker">projects · 项目</div>
          <h1 className="page-title">项目</h1>
          <p className="page-lede">
            项目是数据归属单元：信源归属项目，内容、任务与采集记录由信源派生。切换项目即切换数据范围。
            共享仅开放信源的跨项目只读，不包含其内容与运行记录。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn btn-sm" onClick={() => void reload()} disabled={loading}>
            <IconRefresh size={14} /> 刷新
          </button>
          <button type="button" className="btn btn-primary btn-sm" onClick={() => setCreating(true)}>
            <IconPlus size={14} /> 新建项目
          </button>
        </div>
      </header>

      {!projects.length && loading ? <Loading lines={4} /> : null}

      {projects.length ? (
        <div className="project-grid">
          {projects.map((project) => {
            const item = stats[project.id];
            const isCurrent = project.id === currentProjectId;
            return (
              <button
                key={project.id}
                type="button"
                className={`project-card${project.id === selected ? " is-selected" : ""}`}
                onClick={() => setSelected(project.id)}
              >
                <span className="project-card-head">
                  <span className="project-card-name">{project.name}</span>
                  {project.isDefault ? <Badge tone="neutral">默认</Badge> : null}
                  {isCurrent ? <Badge tone="ok">当前</Badge> : null}
                  <span className="fill" />
                  <IconChevronRight size={14} />
                </span>
                <span className="project-nums">
                  <span>自有信源 <strong>{item ? item.owned : "…"}</strong></span>
                  <span>计划 <strong>{item ? item.plans : "…"}</strong></span>
                  <span>内容 <strong>{item ? item.content : "…"}</strong></span>
                  <span>在途 <strong>{item ? item.running + item.queued : "…"}</strong></span>
                </span>
                <span className="small muted">
                  {item ? `共享引入 ${item.sources - item.owned} 个 · ` : ""}
                  {item?.lastRunAt ? `最近采集 ${relative(item.lastRunAt)}` : "暂无采集记录"} · 建 {stamp(project.createdAt)}
                </span>
              </button>
            );
          })}
        </div>
      ) : null}

      {heroRecord ? (
        <RunsDrawer
          title={heroRecord.title}
          sourceId={heroRecord.sourceId}
          planId={heroRecord.planId}
          projectId={selectedProject?.id}
          onClose={() => setHeroRecord(null)}
        />
      ) : null}

      {selectedProject ? (
        <ProjectWorkbench
          key={selectedProject.id}
          project={selectedProject}
          stats={stats[selectedProject.id]}
          isCurrent={selectedProject.id === currentProjectId}
          onSwitch={() => {
            setCurrentProjectId(selectedProject.id);
            notify("ok", "已切换项目", selectedProject.name);
          }}
          onOpenRuns={() => setHeroRecord({ title: `采集记录 · ${selectedProject.name}` })}
        />
      ) : null}

      {creating ? (
        <Modal
          title="新建项目"
          sub="仅需填写项目名；新建信源、计划与采集记录归属本项目。"
          onClose={() => setCreating(false)}
          footer={
            <>
              <button type="button" className="btn" onClick={() => setCreating(false)}>取消</button>
              <button type="button" className="btn btn-primary" onClick={() => void submit()} disabled={busy || !name.trim()}>
                {busy ? "创建中…" : "创建并切换"}
              </button>
            </>
          }
        >
          <Field label="项目名" htmlFor="project-name">
            <input
              id="project-name"
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
    </div>
  );
}

/** 选中项目的工作面：数字 + 信源 + 计划 + 采集记录 + 起步清单，全部在本页内看完再决定动作。 */
function ProjectWorkbench({
  project,
  stats,
  isCurrent,
  onSwitch,
  onOpenRuns,
}: {
  project: Project;
  stats?: ProjectStats;
  isCurrent: boolean;
  onSwitch: () => void;
  onOpenRuns: () => void;
}) {
  const [sources, setSources] = useState<Source[] | null>(null);
  const [plans, setPlans] = useState<Plan[] | null>(null);
  const [runs, setRuns] = useState<CollectionRun[] | null>(null);
  const [wizard, setWizard] = useState<WizardDoc | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const [record, setRecord] = useState<{ sourceId?: string; planId?: string; runId?: string; title: string } | null>(null);
  const [showAllSteps, setShowAllSteps] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [sourcePage, planPage, runPage, doc] = await Promise.all([
        listSources({ limit: 200, projectId: project.id }).catch(() => null),
        listPlans({ limit: 200, projectId: project.id }).catch(() => null),
        listCollectionRuns({ limit: 8, projectId: project.id }).catch(() => null),
        getWizard(project.id).catch(() => null),
      ]);
      // 任一请求失败时保留原值，不把「没取到」伪装成 0，避免误读为“这个项目是空的”。
      if (sourcePage) setSources(sourcePage.items);
      if (planPage) setPlans(planPage.items);
      if (runPage) setRuns(runPage.items);
      if (doc) setWizard(doc);
      setFailed(!sourcePage || !planPage || !runPage);
      setError(null);
    } catch (problem) {
      setError(problem);
    } finally {
      setLoading(false);
    }
  }, [project.id]);

  useEffect(() => {
    void load();
  }, [load]);

  const owned = useMemo(
    () => (sources ?? []).filter((item) => !item.project || item.project === project.id),
    [sources, project.id],
  );
  const shared = useMemo(
    () => (sources ?? []).filter((item) => Boolean(item.project) && item.project !== project.id),
    [sources, project.id],
  );
  const pendingSteps = useMemo(() => (wizard?.steps ?? []).filter((step) => !step.met), [wizard]);
  const visibleSteps = showAllSteps ? wizard?.steps ?? [] : pendingSteps;

  return (
    <Card
      title={project.name}
      sub={`${project.description || "项目上下文下的信源、计划与采集记录"} · 建 ${stamp(project.createdAt)}`}
      action={
        <div className="row wrap" style={{ gap: 6 }}>
          {isCurrent ? (
            <Badge tone="ok">当前项目</Badge>
          ) : (
            <button type="button" className="btn btn-sm" onClick={onSwitch}>设为当前项目</button>
          )}
          <button type="button" className="btn btn-sm" onClick={() => navigate("sources")}>信源</button>
          <button type="button" className="btn btn-sm" onClick={() => navigate("plans")}>计划</button>
          <button type="button" className="btn btn-sm" onClick={() => navigate("content")}>内容</button>
          <button type="button" className="btn btn-sm" onClick={onOpenRuns}>
            <IconHistory size={12} /> 采集记录
          </button>
          <button type="button" className="btn btn-sm" onClick={() => void load()} disabled={loading}>
            <IconRefresh size={12} />
          </button>
        </div>
      }
    >
      {error ? <Problem error={error} onRetry={() => void load()} /> : null}
      {failed && !sources ? <div className="alert warn">项目数据未完整加载，下方内容可能不完整；可刷新重试。</div> : null}
      {loading && !sources ? <Loading lines={4} /> : null}

      <div className="stack" style={{ gap: 16 }}>
        <div className="stat-grid">
          <div className="stat">
            <span className="stat-label">自有信源</span>
            <span className="stat-value">{sources === null ? "…" : owned.length}</span>
            <span className="stat-note">{shared.length} 个共享引入 · {owned.filter((item) => item.enabled).length} 个启用</span>
          </div>
          <div className="stat">
            <span className="stat-label">采集计划</span>
            <span className="stat-value">{plans === null ? "…" : plans.length}</span>
            <span className="stat-note">{(plans ?? []).filter((plan) => plan.enabled).length} 个启用</span>
          </div>
          <div className="stat">
            <span className="stat-label">已入库内容</span>
            <span className="stat-value">{stats ? stats.content : "…"}</span>
            <span className="stat-note">由本项目信源派生</span>
          </div>
          <div className="stat">
            <span className="stat-label">在途任务</span>
            <span className="stat-value">{stats ? stats.running + stats.queued : "…"}</span>
            <span className="stat-note">{stats?.lastRunAt ? `最近采集 ${relative(stats.lastRunAt)}` : "暂无采集"}</span>
          </div>
        </div>

        <div className="split-2">
          <Card
            title="信源"
            sub={`${owned.length} 个自有 · ${shared.length} 个共享`}
            flush
            action={<button type="button" className="btn btn-sm" onClick={() => navigate("sources")}>管理</button>}
          >
            {(sources ?? []).length === 0 ? (
              <Empty title="暂无信源" text="新建信源归属本项目；共享信源只读。" />
            ) : (
              <div className="list">
                {[...owned, ...shared].slice(0, 8).map((source) => (
                  <button
                    key={source.id}
                    type="button"
                    className="list-row clickable"
                    onClick={() => setRecord({ sourceId: source.id, title: `采集记录 · ${source.name}` })}
                  >
                    <span className={`dot ${source.health === "healthy" ? "ok" : source.health === "failed" ? "bad" : "warn"}`} />
                    <span className="list-main">
                      <span className="list-title">
                        {source.name}
                        {source.project && source.project !== project.id ? <span className="badge neutral" style={{ marginLeft: 6 }}>共享</span> : null}
                        {source.enabled ? null : <span className="badge neutral" style={{ marginLeft: 6 }}>停用</span>}
                      </span>
                      <span className="list-meta">
                        <span className="mono">{source.id}</span>
                        <span>今日 {source.todayYield ?? 0} 条</span>
                        <span>下次 {relative(source.nextRunAt)}</span>
                      </span>
                    </span>
                    <IconHistory size={14} />
                  </button>
                ))}
              </div>
            )}
          </Card>

          <Card
            title="采集计划"
            sub={`${plans?.length ?? 0} 个`}
            flush
            action={<button type="button" className="btn btn-sm" onClick={() => navigate("plans")}>管理</button>}
          >
            {(plans === null ? <Loading lines={2} /> : plans.length === 0 ? (
              <Empty title="暂无计划" text="计划按节律派发采集，也可手动执行一次。" />
            ) : (
              <div className="list">
                {(plans ?? []).slice(0, 8).map((plan) => (
                  <button
                    key={plan.id}
                    type="button"
                    className="list-row clickable"
                    onClick={() => setRecord({ planId: plan.id, title: `采集记录 · ${plan.name}` })}
                  >
                    <span className="list-main">
                      <span className="list-title">
                        {plan.name}
                        <span className={`badge ${plan.enabled ? "ok" : "neutral"}`} style={{ marginLeft: 6 }}>
                          {plan.enabled ? "启用" : "暂停"}
                        </span>
                      </span>
                      <span className="list-meta">
                        <span>{plan.trigger?.interval ?? "手动"}</span>
                        <span>{(plan.targets ?? []).length} 个信源</span>
                        <span>下次 {relative(plan.nextRunAt)}</span>
                      </span>
                    </span>
                    <IconHistory size={14} />
                  </button>
                ))}
              </div>
            ))}
          </Card>
        </div>

        <Card
          title="最近采集"
          sub="按信源与计划归组；展开查看本次产出"
          flush
          action={<button type="button" className="btn btn-sm" onClick={onOpenRuns}>全部记录</button>}
        >
          {(runs === null ? <Loading lines={2} /> : runs.length === 0 ? (
            <Empty title="暂无采集记录" text="计划触发与手动采集记录。" />
          ) : (
            <div className="list">
              {(runs ?? []).map((run) => (
                <button
                  key={run.id}
                  type="button"
                  className="list-row clickable"
                  onClick={() =>
                    setRecord({
                      sourceId: run.sourceId,
                      runId: run.id,
                      title: `采集记录 · ${(sources ?? []).find((item) => item.id === run.sourceId)?.name ?? run.sourceId}`,
                    })
                  }
                >
                  <span className={`dot ${run.status === "failed" ? "bad" : run.status === "succeeded" ? "ok" : "warn"}`} />
                  <span className="list-main">
                    <span className="list-title">
                      {(sources ?? []).find((item) => item.id === run.sourceId)?.name ?? run.sourceId}
                      <span className="badge neutral" style={{ marginLeft: 6 }}>{run.mode === "manual" ? "手动" : "计划"}</span>
                    </span>
                    <span className="list-meta">
                      <span>{relative(run.createdAt)}</span>
                      <span>{run.status === "succeeded" ? "成功" : run.status === "failed" ? "失败" : run.status === "running" ? "执行中" : "排队"}</span>
                      {run.planId ? <span>计划 {(plans ?? []).find((item) => item.id === run.planId)?.name ?? run.planId}</span> : null}
                    </span>
                    {run.error ? <span className="list-meta" style={{ color: "var(--danger)" }}>{run.error}</span> : null}
                  </span>
                  <IconChevronRight size={14} />
                </button>
              ))}
            </div>
          ))}
        </Card>

        <Card
          title="起步清单"
          sub={wizard ? (wizard.status === "ready" ? "已就绪，无待办" : `${wizard.pending} 项待办（按当前数据计算）`) : undefined}
          flush
          action={
            <div className="row" style={{ gap: 6 }}>
              {wizard ? <Badge tone={wizard.status === "ready" ? "ok" : "warn"}>{wizard.status === "ready" ? "已就绪" : "待办"}</Badge> : null}
              <button type="button" className="btn btn-sm" onClick={() => setShowAllSteps((value) => !value)}>
                {showAllSteps ? "只看待办" : "显示全部环节"}
              </button>
            </div>
          }
        >
          {!wizard ? (
            <Empty title="暂时没有清单" text="刷新重试。" />
          ) : visibleSteps.length === 0 ? (
            <Empty title="没有待办环节" text="信源、模型与内容都已就绪。" />
          ) : (
            <div className="list">
              {visibleSteps.map((step) => {
                const link = STEP_LINKS[step.id];
                return (
                  <div className="list-row" key={step.id}>
                    <span className="list-main">
                      <span className="list-title">{step.title}</span>
                      <span className="list-meta">
                        {step.description ? <span>{step.description}</span> : null}
                        {!step.met && step.hint ? <span>配置入口：{step.hint}</span> : null}
                      </span>
                    </span>
                    <span className="row" style={{ gap: 6 }}>
                      <Badge tone={step.status === "done" ? "ok" : step.status === "optional" ? "neutral" : "warn"}>
                        {step.status === "done" ? "完成" : step.status === "optional" ? "可选" : "待办"}
                      </Badge>
                      {link && !step.met ? (
                        <button type="button" className="btn btn-sm" onClick={() => navigate(link.page)}>
                          {link.label}
                        </button>
                      ) : null}
                    </span>
                  </div>
                );
              })}
            </div>
          )}
        </Card>
      </div>

      {record ? (
        <RunsDrawer
          title={record.title}
          sourceId={record.sourceId}
          planId={record.planId}
          initialRunId={record.runId}
          projectId={project.id}
          onClose={() => setRecord(null)}
        />
      ) : null}
    </Card>
  );
}
