import { useCallback, useEffect, useMemo, useState } from "react";

import {
  IconCheck,
  IconClose,
  IconFlask,
  IconPlay,
  IconPlus,
  IconRefresh,
  IconTrash,
} from "../components/icons";
import { Card, Drawer, Empty, Field, Loading, Modal, Problem, StatusBadge, useUi } from "../components/ui";
import {
  analyzeSource,
  approveAdapterDraft,
  bindProjectResource,
  createAdapterDraft,
  createCollectionRun,
  createSource,
  deleteSource,
  disableSource,
  enableSource,
  listAdapterDrafts,
  listAdapters,
  listAccessMethods,
  listPlans,
  listSources,
  pausePlan,
  probeSource,
  rejectAdapterDraft,
  resumePlan,
  updateSource,
} from "../lib/api";
import { interval, relative, statusLabel, statusTone } from "../lib/format";
import { useCurrentProject } from "../lib/project";
import { navigate, useRoute } from "../lib/router";
import type { NamedResource, Operation, Plan, Source, SourceInput } from "../lib/types";

const EMPTY: SourceInput = {
  name: "",
  accessMethodId: "rss",
  adapterIds: ["feed-items"],
  enabled: true,
  endpoint: "",
  schedule: { mode: "fixed", intervalMs: 7_200_000 },
};

export function SourcesPage() {
  const { params } = useRoute();
  const { notify, run } = useUi();
  const { currentProject, boundIds } = useCurrentProject();
  const [sources, setSources] = useState<Source[] | null>(null);
  const [plans, setPlans] = useState<Plan[]>([]);
  const [methods, setMethods] = useState<NamedResource[]>([]);
  const [adapters, setAdapters] = useState<NamedResource[]>([]);
  const [drafts, setDrafts] = useState<NamedResource[]>([]);
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState(params.get("query") ?? "");
  const [health, setHealth] = useState(params.get("status") ?? "all");
  const [editing, setEditing] = useState<Source | "new" | null>(params.get("new") ? "new" : null);
  const [busy, setBusy] = useState<string | null>(null);
  const [analysis, setAnalysis] = useState(false);
  const [draftOpen, setDraftOpen] = useState(false);
  const [scope, setScope] = useState<"all" | "project">("all");

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [sourcePage, planPage, methodPage, adapterPage, draftPage] = await Promise.all([
        listSources({ limit: 200 }),
        listPlans({ limit: 200 }),
        listAccessMethods(),
        listAdapters(),
        listAdapterDrafts(),
      ]);
      setSources(sourcePage.items);
      setPlans(planPage.items);
      setMethods(methodPage.items);
      setAdapters(adapterPage.items);
      setDrafts(draftPage.items);
      setError(null);
    } catch (problem) {
      setError(problem);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    const timer = window.setTimeout(() => setSearch(params.get("query") ?? search), 200);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [params]);

  const rows = useMemo(() => {
    const needle = search.trim().toLowerCase();
    const projectBound = scope === "project" && currentProject ? new Set(boundIds("source")) : null;
    return (sources ?? []).filter((source) => {
      if (projectBound && !projectBound.has(source.id)) return false;
      if (health !== "all" && source.health !== health) return false;
      if (!needle) return true;
      return (
        source.name.toLowerCase().includes(needle) ||
        source.id.toLowerCase().includes(needle) ||
        source.endpoint.toLowerCase().includes(needle)
      );
    });
  }, [sources, search, health, scope, currentProject, boundIds]);

  const planOf = (sourceId: string) => plans.find((plan) => plan.sourceId === sourceId);

  async function act(label: string, action: () => Promise<unknown>) {
    setBusy(label);
    try {
      await action();
      notify("ok", `${label}：完成`);
      await load();
    } catch (problem) {
      notify("bad", `${label}：失败`, problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(null);
    }
  }

  // 异步动作由 run 统一提示结果；这里只负责创建请求失败的提示与忙碌态。
  async function perform(label: string, action: () => Promise<Operation>) {
    setBusy(label);
    let started = false;
    try {
      const operation = await action();
      started = true;
      await run(operation, label);
      await load();
    } catch (problem) {
      if (!started) {
        notify("bad", `${label}：失败`, problem instanceof Error ? problem.message : undefined);
      }
    } finally {
      setBusy(null);
    }
  }

  async function collect(source: Source, locator?: string) {
    setBusy(`collect-${source.id}`);
    let started = false;
    try {
      const operation = await createCollectionRun({ sourceId: source.id, mode: locator ? "singleLocator" : "normal", locator });
      started = true;
      await run(operation, `手动采集 ${source.name}`);
      await load();
    } catch (problem) {
      if (!started) {
        notify("bad", "手动采集失败", problem instanceof Error ? problem.message : undefined);
      }
    } finally {
      setBusy(null);
    }
  }

  async function probe(source: Source) {
    setBusy(`probe-${source.id}`);
    let started = false;
    try {
      const operation = await probeSource(source.id);
      started = true;
      await run(operation, `探测 ${source.name}`);
    } catch (problem) {
      if (!started) {
        notify("bad", "探测失败", problem instanceof Error ? problem.message : undefined);
      }
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <div className="kicker">sources · 信源</div>
          <h1 className="page-title">信源、接入方式与采集计划</h1>
          <p className="page-lede">
            信源是采集的业务对象，接入方式与适配器藏在业务选项后面。手动采集、探测与自动计划共用同一条执行核心；所有修改都经过配置校验与代际热加载。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn" onClick={() => setDraftOpen(true)}>
            <IconFlask size={15} /> AI 生成适配器
          </button>
          <button type="button" className="btn" onClick={() => void load()} disabled={loading}>
            <IconRefresh size={14} /> 刷新
          </button>
          <button type="button" className="btn btn-primary" onClick={() => setEditing("new")}>
            <IconPlus size={15} /> 新建信源
          </button>
        </div>
      </header>

      {error ? <Problem error={error} onRetry={() => void load()} /> : null}

      <div className="toolbar-band">
        <label className="search">
          <span className="muted" aria-hidden="true">⌕</span>
          <input value={search} onChange={(event) => setSearch(event.target.value)} placeholder="搜索名称、标识或地址" aria-label="搜索信源" />
        </label>
        <div className="seg" role="group" aria-label="健康筛选">
          {[
            ["all", "全部"],
            ["healthy", "正常"],
            ["degraded", "退化"],
            ["failed", "失败"],
            ["suspicious", "可疑"],
            ["configurationError", "配置错误"],
          ].map(([id, label]) => (
            <button key={id} type="button" className={health === id ? "is-active" : ""} onClick={() => setHealth(id)}>
              {label}
            </button>
          ))}
        </div>
        <div className="seg" role="group" aria-label="项目范围">
          <button type="button" className={scope === "all" ? "is-active" : ""} onClick={() => setScope("all")}>
            全部
          </button>
          <button
            type="button"
            className={scope === "project" ? "is-active" : ""}
            onClick={() => setScope("project")}
            disabled={!currentProject}
          >
            {currentProject ? `仅 ${currentProject.name}` : "当前项目"}
          </button>
        </div>
        <span className="fill" />
        <span className="small muted">
          {rows.length} / {sources?.length ?? 0} 个信源
        </span>
      </div>

      <Card title="信源清单" sub="启停、计划与最近一次运行" flush>
        {loading && !sources ? (
          <div className="card-body">
            <Loading lines={5} />
          </div>
        ) : rows.length === 0 ? (
          <Empty
            title={sources?.length ? "没有匹配的信源" : "还没有信源"}
            text={sources?.length ? "调整关键词或健康筛选。" : "从模板新建一个订阅源，或让 AI 先分析目标地址。"}
            action={<button type="button" className="btn btn-primary btn-sm" onClick={() => setEditing("new")}>新建信源</button>}
          />
        ) : (
          <div className="table-wrap">
            <table className="table source-table">
              <thead>
                <tr>
                  <th>信源</th>
                  <th>健康</th>
                  <th>节奏</th>
                  <th>今日产出</th>
                  <th>下一次</th>
                  <th>状态</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {rows.map((source) => {
                  const plan = planOf(source.id);
                  return (
                    <tr key={source.id}>
                      <td>
                        <div className="cell-main">
                          <span className="source-name">{source.name}</span>
                          <span className="source-sub mono">{source.id} · {source.endpoint}</span>
                        </div>
                      </td>
                      <td>
                        <StatusBadge status={source.health} />
                      </td>
                      <td>
                        <div className="cell-main">
                          <span className="small">{source.schedule.mode === "push" ? "随推送" : interval(source.schedule.intervalMs)}</span>
                          <span className="source-sub">{plan?.status === "paused" ? "计划已暂停" : source.enabled ? "计划生效" : "未启用"}</span>
                        </div>
                      </td>
                      <td className="mono">{source.todayYield ?? 0}</td>
                      <td className="small muted">{relative(source.nextRunAt)}</td>
                      <td>
                        <span className={`badge ${source.enabled ? "ok" : "neutral"}`}>{source.enabled ? "启用" : "停用"}</span>
                      </td>
                      <td className="actions">
                        <div className="row" style={{ justifyContent: "flex-end" }}>
                          <button type="button" className="btn btn-sm" onClick={() => void collect(source)} disabled={busy === `collect-${source.id}` || !source.enabled}>
                            <IconPlay size={12} /> 采集
                          </button>
                          <button type="button" className="btn btn-sm" onClick={() => void probe(source)} disabled={busy === `probe-${source.id}`}>
                            <IconFlask size={12} /> 探测
                          </button>
                          <button type="button" className="btn btn-sm" onClick={() => setEditing(source)}>
                            编辑
                          </button>
                          <button
                            type="button"
                            className="btn btn-sm"
                            onClick={() => void act(source.enabled ? "停用" : "启用", async () =>
                              source.enabled ? disableSource(source.id, source.etag) : enableSource(source.id, source.etag),
                            )}
                            disabled={busy === "启用" || busy === "停用"}
                          >
                            {source.enabled ? "停用" : "启用"}
                          </button>
                          {plan ? (
                            <button
                              type="button"
                              className="btn btn-sm"
                              onClick={() =>
                                void act(plan.status === "paused" ? "恢复计划" : "暂停计划", async () =>
                                  plan.status === "paused" ? resumePlan(plan.id, plan.etag) : pausePlan(plan.id, plan.etag),
                                )
                              }
                            >
                              {plan.status === "paused" ? "恢复" : "暂停"}
                            </button>
                          ) : null}
                          <button
                            type="button"
                            className="icon-btn"
                            aria-label={`删除 ${source.name}`}
                            onClick={() => {
                              if (!window.confirm(`删除信源「${source.name}」？引用它的声明必须先行处理。`)) return;
                              void perform("删除信源", async () => deleteSource(source.id, source.etag));
                            }}
                          >
                            <IconTrash size={15} />
                          </button>
                        </div>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </Card>

      <Card
        title="适配器草稿"
        sub="AI 生成的适配器在批准前不会写入配置"
        action={
          <button type="button" className="btn btn-sm" onClick={() => setDraftOpen(true)}>
            <IconPlus size={12} /> 新建草稿
          </button>
        }
        flush
      >
        {drafts.length === 0 ? (
          <Empty title="没有适配器草稿" text="先做一次信源分析，再根据页面结构生成适配器。" />
        ) : (
          <div className="list">
            {drafts.map((draft) => {
              const status = String(draft.status ?? "draft");
              const etag = String(draft.etag ?? "");
              return (
                <div className="list-row" key={draft.id}>
                  <span className="list-main">
                    <span className="list-title">{draft.name}</span>
                    <span className="list-meta">
                      <span className="mono">{draft.id}</span>
                      <span>分析 {String(draft.sourceAnalysisId ?? "")}</span>
                      {Array.isArray(draft.checks) && draft.checks.length > 0 ? <span>{draft.checks.length} 项检查未通过</span> : null}
                    </span>
                  </span>
                  <span className={`badge ${statusTone(status)}`}>{statusLabel(status)}</span>
                  {status === "validated" ? (
                    <div className="row">
                      <button
                        type="button"
                        className="btn btn-sm"
                        onClick={() =>
                          void act("批准适配器", async () => {
                            const operation = await approveAdapterDraft(draft.id, etag, "控制台批准");
                            await run(operation, "批准适配器");
                          })
                        }
                      >
                        <IconCheck size={12} /> 批准
                      </button>
                      <button
                        type="button"
                        className="btn btn-sm"
                        onClick={() =>
                          void act("驳回适配器", async () => {
                            await rejectAdapterDraft(draft.id, etag, "控制台驳回");
                          })
                        }
                      >
                        <IconClose size={12} /> 驳回
                      </button>
                    </div>
                  ) : (
                    <button type="button" className="btn btn-sm" onClick={() => navigate(`logs?correlationId=${encodeURIComponent(String(draft.id ?? ""))}`)}>
                      查看日志
                    </button>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </Card>

      {editing ? (
        <SourceDrawer
          source={editing === "new" ? null : editing}
          methods={methods}
          adapters={adapters}
          onClose={() => setEditing(null)}
          onAnalyze={() => setAnalysis(true)}
          onSaved={async () => {
            setEditing(null);
            await load();
          }}
        />
      ) : null}

      {analysis ? (
        <AnalyzeModal
          onClose={() => setAnalysis(false)}
          onUse={(proposal) => {
            setAnalysis(false);
            setEditing("new");
            window.setTimeout(() => {
              window.dispatchEvent(new CustomEvent("modalprism:source-proposal", { detail: proposal }));
            }, 40);
          }}
        />
      ) : null}

      {draftOpen ? <AdapterDraftModal sources={sources ?? []} onClose={() => setDraftOpen(false)} onCreated={() => void load()} /> : null}
    </div>
  );
}

function SourceDrawer({
  source,
  methods,
  adapters,
  onClose,
  onAnalyze,
  onSaved,
}: {
  source: Source | null;
  methods: NamedResource[];
  adapters: NamedResource[];
  onClose: () => void;
  onAnalyze: () => void;
  onSaved: () => void | Promise<void>;
}) {
  const { notify, run } = useUi();
  const { currentProjectId } = useCurrentProject();
  const [form, setForm] = useState<SourceInput>(() =>
    source
      ? {
          name: source.name,
          accessMethodId: source.accessMethodId,
          adapterIds: source.adapterIds,
          enabled: source.enabled,
          endpoint: source.endpoint,
          schedule: { ...source.schedule },
        }
      : { ...EMPTY, adapterIds: [...EMPTY.adapterIds], schedule: { ...EMPTY.schedule } },
  );
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [intervalValue, setIntervalValue] = useState(() => {
    const ms = source?.schedule.intervalMs ?? EMPTY.schedule.intervalMs ?? 7_200_000;
    return ms / 60_000;
  });

  useEffect(() => {
    const apply = (event: Event) => {
      const proposal = (event as CustomEvent<Record<string, unknown>>).detail;
      setForm((current) => ({
        ...current,
        name: String(proposal.name ?? current.name),
        endpoint: String(proposal.endpoint ?? current.endpoint),
        accessMethodId: String(proposal.accessMethodId ?? current.accessMethodId).replace(/^accessor\//, ""),
        adapterIds: Array.isArray(proposal.adapterIds) ? proposal.adapterIds.map((id) => String(id).replace(/^adapter\//, "")) : current.adapterIds,
        schedule: {
          ...current.schedule,
          intervalMs: Number(proposal.intervalMs ?? current.schedule.intervalMs ?? 7_200_000),
        },
      }));
      if (proposal.intervalMs) setIntervalValue(Number(proposal.intervalMs) / 60_000);
    };
    window.addEventListener("modalprism:source-proposal", apply);
    return () => window.removeEventListener("modalprism:source-proposal", apply);
  }, []);

  function toggleAdapter(id: string) {
    setForm((current) => ({
      ...current,
      adapterIds: current.adapterIds.includes(id)
        ? current.adapterIds.filter((item) => item !== id)
        : [...current.adapterIds, id],
    }));
  }

  async function save() {
    setSaving(true);
    setError(null);
    const payload: SourceInput = {
      ...form,
      schedule:
        form.schedule.mode === "push"
          ? { mode: "push" }
          : { ...form.schedule, intervalMs: Math.max(60_000, Math.round(intervalValue) * 60_000) },
    };
    try {
      if (!payload.name.trim() || !payload.endpoint.trim()) {
        throw new Error("名称与地址不能为空");
      }
      if (payload.adapterIds.length === 0) {
        throw new Error("至少选择一个适配器");
      }
      if (!source) {
        const created = await createSource(payload);
        // 新建信源默认归属当前项目（上下文未就绪则跳过，可稍后在项目页手动绑定）。
        if (currentProjectId) {
          try {
            await bindProjectResource(currentProjectId, "source", created.id);
          } catch {
            // 绑定失败不阻断创建本身。
          }
        }
        notify("ok", "信源已创建");
      } else {
        await updateSource(source.id, payload, source.etag);
        notify("ok", "信源已更新");
      }
      await onSaved();
    } catch (problem) {
      setError(problem instanceof Error ? problem.message : "保存失败");
      notify("bad", "保存信源失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setSaving(false);
    }
  }

  async function collectNow() {
    if (!source) return;
    let started = false;
    try {
      const operation = await createCollectionRun({ sourceId: source.id, mode: "normal" });
      started = true;
      await run(operation, `手动采集 ${source.name}`);
    } catch (problem) {
      if (!started) {
        notify("bad", "手动采集失败", problem instanceof Error ? problem.message : undefined);
      }
    }
  }

  return (
    <Drawer
      title={source ? "编辑信源" : "新建信源"}
      sub={source ? source.id : "保存后立即校验并热加载"}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>取消</button>
          <button type="button" className="btn btn-primary" onClick={() => void save()} disabled={saving}>
            {saving ? "保存中…" : "保存信源"}
          </button>
        </>
      }
    >
      <div className="stack" style={{ gap: 14 }}>
        {error ? <div className="alert bad">{error}</div> : null}
        <div className="row wrap">
          <button type="button" className="btn btn-sm" onClick={onAnalyze}>
            <IconFlask size={13} /> 让 AI 分析地址
          </button>
          {source ? (
            <button type="button" className="btn btn-sm" onClick={() => void collectNow()}>
              <IconPlay size={13} /> 立即采集
            </button>
          ) : null}
        </div>
        <Field label="名称" htmlFor="source-name">
          <input id="source-name" className="input" value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} placeholder="例如 Hugging Face Blog" />
        </Field>
        <Field label="地址" hint="只接受 http/https。单 URL 采集会替换这个地址，但不会改写声明。" htmlFor="source-endpoint">
          <input id="source-endpoint" className="input" value={form.endpoint} onChange={(event) => setForm({ ...form, endpoint: event.target.value })} placeholder="https://example.com/feed.xml" />
        </Field>
        <div className="grid cols-2">
          <Field label="接入方式">
            <select className="select" value={form.accessMethodId} onChange={(event) => setForm({ ...form, accessMethodId: event.target.value })}>
              {methods.map((method) => (
                <option key={method.id} value={method.id}>{method.name}</option>
              ))}
            </select>
          </Field>
          <Field label="采集节奏" hint={form.schedule.mode === "push" ? "推送信源按通道速率接收" : "固定间隔；自适应由系统在上下限内调整"}>
            <div className="row">
              <input
                className="input"
                type="number"
                min={1}
                value={intervalValue}
                onChange={(event) => setIntervalValue(Number(event.target.value))}
                disabled={form.schedule.mode === "push"}
                aria-label="采集间隔（分钟）"
              />
              <span className="muted small">分钟</span>
            </div>
          </Field>
        </div>
        <Field label="适配器" hint="适配器负责把原始响应解析为内容；可多选并按顺序执行">
          <div className="stack" style={{ gap: 6 }}>
            {adapters.map((adapter) => (
              <label key={adapter.id} className="switch" style={{ justifyContent: "space-between" }}>
                <span>{adapter.name}</span>
                <input type="checkbox" checked={form.adapterIds.includes(adapter.id)} onChange={() => toggleAdapter(adapter.id)} />
              </label>
            ))}
          </div>
        </Field>
        <label className="switch">
          <input type="checkbox" checked={form.enabled} onChange={(event) => setForm({ ...form, enabled: event.target.checked })} />
          <span>启用这条信源</span>
        </label>
        {source ? (
          <div className="code">
            {JSON.stringify({ id: source.id, health: source.health, references: source.references, etag: source.etag }, null, 2)}
          </div>
        ) : null}
      </div>
    </Drawer>
  );
}

function AnalyzeModal({ onClose, onUse }: { onClose: () => void; onUse: (proposal: Record<string, unknown>) => void }) {
  const { notify, run } = useUi();
  const [url, setUrl] = useState("");
  const [instruction, setInstruction] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<Record<string, unknown> | null>(null);
  const [error, setError] = useState<unknown>(null);

  async function start() {
    setBusy(true);
    setError(null);
    try {
      const operation = await analyzeSource({ url, instruction: instruction || undefined });
      const finished = await run(operation, "AI 信源分析");
      const summary = (finished.resultSummary ?? {}) as Record<string, unknown>;
      setResult(summary);
      notify("ok", "分析完成", "草稿需要人工确认后才会创建信源。");
    } catch (problem) {
      setError(problem);
    } finally {
      setBusy(false);
    }
  }

  const proposal = (result?.proposal ?? result ?? {}) as Record<string, unknown>;

  return (
    <Modal
      title="AI 分析信源"
      sub="使用系统设置里的默认模型与版本化提示词；结果只作为草稿"
      onClose={onClose}
      wide
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>关闭</button>
          {result ? (
            <button type="button" className="btn btn-primary" onClick={() => onUse(proposal)}>
              填入新建表单
            </button>
          ) : (
            <button type="button" className="btn btn-primary" onClick={() => void start()} disabled={busy || !url.trim()}>
              {busy ? "分析中…" : "开始分析"}
            </button>
          )}
        </>
      }
    >
      <div className="stack" style={{ gap: 12 }}>
        <Field label="目标地址">
          <input className="input" value={url} onChange={(event) => setUrl(event.target.value)} placeholder="https://example.com/news" />
        </Field>
        <Field label="补充说明" hint="可选，例如“只需要最近 30 天”“列表在脚本里的 routePath”">
          <textarea className="textarea" value={instruction} onChange={(event) => setInstruction(event.target.value)} />
        </Field>
        {error ? <Problem error={error} /> : null}
        {busy ? <Loading lines={3} /> : null}
        {result ? (
          <>
            <div className="alert info">模型给出的是草稿；请核对接入方式、地址与风险，再保存信源。</div>
            <div className="code">{JSON.stringify(proposal, null, 2)}</div>
          </>
        ) : null}
      </div>
    </Modal>
  );
}

function AdapterDraftModal({ sources, onClose, onCreated }: { sources: Source[]; onClose: () => void; onCreated: () => void }) {
  const { notify, run } = useUi();
  const [analysisId, setAnalysisId] = useState("");
  const [name, setName] = useState("");
  const [instruction, setInstruction] = useState("");
  const [busy, setBusy] = useState(false);

  async function create() {
    setBusy(true);
    let started = false;
    try {
      const operation = await createAdapterDraft({ sourceAnalysisId: analysisId, name, instruction: instruction || undefined });
      started = true;
      await run(operation, "生成适配器草稿");
      onCreated();
      onClose();
    } catch (problem) {
      if (!started) {
        notify("bad", "生成适配器失败", problem instanceof Error ? problem.message : undefined);
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title="生成适配器草稿"
      sub="需要一个已完成信源分析的标识"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>取消</button>
          <button type="button" className="btn btn-primary" onClick={() => void create()} disabled={busy || !analysisId.trim() || !name.trim()}>
            {busy ? "生成中…" : "生成草稿"}
          </button>
        </>
      }
    >
      <div className="stack" style={{ gap: 12 }}>
        <Field label="信源分析 ID" hint="在信源页运行 AI 分析后，结果里会返回 analysisId">
          <input className="input" value={analysisId} onChange={(event) => setAnalysisId(event.target.value)} placeholder="analysis_…" />
        </Field>
        <Field label="适配器名称">
          <input className="input" value={name} onChange={(event) => setName(event.target.value)} placeholder="例如 站点列表解析器" />
        </Field>
        <Field label="补充要求">
          <textarea className="textarea" value={instruction} onChange={(event) => setInstruction(event.target.value)} placeholder="字段路径、分页、日期格式…" />
        </Field>
        <div className="small muted">可选信源：{sources.slice(0, 6).map((source) => source.id).join("、")}</div>
      </div>
    </Modal>
  );
}
