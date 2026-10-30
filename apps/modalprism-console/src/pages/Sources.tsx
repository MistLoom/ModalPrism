import { useCallback, useEffect, useMemo, useState } from "react";

import {
  IconCheck,
  IconClose,
  IconFlask,
  IconHistory,
  IconPlay,
  IconPlus,
  IconRefresh,
  IconTrash,
} from "../components/icons";
import { RunsDrawer } from "../components/RunsDrawer";
import { Card, Drawer, Empty, Field, Loading, Modal, Problem, StatusBadge, StructuredValue, Tabs, useUi } from "../components/ui";
import { FilterPanel, SortHeader, sortRows, useSort } from "../components/Table";
import { Pager, slicePage } from "../components/Pager";
import {
  analyzeSource,
  approveAdapterDraft,
  createAdapterDraft,
  createCollectionRun,
  createPlan,
  createSource,
  deleteSource,
  disableSource,
  enableSource,
  listAdapterDrafts,
  listAdapters,
  listAccessMethods,
  listPlans,
  listSources,
  listSourceTypes,
  probeSource,
  rejectAdapterDraft,
  shareSource,
  unshareSource,
  updateSource,
} from "../lib/api";
import { relative, statusLabel, statusTone } from "../lib/format";
import { useCurrentProject } from "../lib/project";
import { navigate, useRoute } from "../lib/router";
import type { NamedResource, Operation, Plan, Schedule, Source, SourceInput, SourcePayload, SourceTypeRecord } from "../lib/types";

const EMPTY: SourceInput = {
  name: "",
  sourceType: "feed",
  accessMethodId: "rss",
  adapterIds: ["feed-items"],
  enabled: true,
  endpoint: "",
  channel: "",
  schedule: { mode: "fixed", intervalMs: 7_200_000 },
};

// 自定义适配器的声明示例：与内置适配器同构，放进 config/adapters 后即可在信源里勾选。
const ADAPTER_EXAMPLE = `id: adapter/news-json
kind: adapter
name: 新闻 JSON 列表
description: 从 JSON 响应里抽取标题、链接与时间
implementation: builtin:json-list
role: search
consumes:
  - {kind: raw, media_type: application/json}
produces: [discovery]
limits:
  timeout: 30s
  concurrency: 1`;

export function SourcesPage() {
  const { params } = useRoute();
  const { notify, run } = useUi();
  const { currentProject } = useCurrentProject();
  const [sources, setSources] = useState<Source[] | null>(null);
  const [plans, setPlans] = useState<Plan[]>([]);
  const [methods, setMethods] = useState<NamedResource[]>([]);
  const [adapters, setAdapters] = useState<NamedResource[]>([]);
  const [types, setTypes] = useState<SourceTypeRecord[]>([]);
  const [drafts, setDrafts] = useState<NamedResource[]>([]);
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState(params.get("query") ?? "");
  const [health, setHealth] = useState(params.get("status") ?? "all");
  const [typeFilter, setTypeFilter] = useState(params.get("type") ?? "all");
  const [editing, setEditing] = useState<Source | "new" | null>(params.get("new") ? "new" : null);
  const [busy, setBusy] = useState<string | null>(null);
  const [analysis, setAnalysis] = useState(false);
  const [draftOpen, setDraftOpen] = useState(false);
  // 采集记录速览：不离开当前页就看上游（信源/计划）与下游（本次产出）。
  const [record, setRecord] = useState<Source | null>(null);
  const [scope, setScope] = useState<"all" | "project">("project");
  const [selection, setSelection] = useState<string[]>([]);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  const [tab, setTab] = useState<"list" | "adapters">((params.get("tab") === "adapters" ? "adapters" : "list"));
  const [flag, setFlag] = useState<"all" | "enabled" | "disabled" | "shared" | "alert">("all");
  const { sort, toggle } = useSort();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [sourcePage, planPage, methodPage, adapterPage, draftPage, typePage] = await Promise.all([
        // 项目隔离由后端按 x-project-id 处理；"全部"显式请求 scope=all 取消过滤。
        listSources({ limit: 200, scope: scope === "all" ? "all" : undefined }),
        listPlans({ limit: 200 }),
        listAccessMethods(),
        listAdapters(),
        listAdapterDrafts(),
        listSourceTypes(),
      ]);
      setSources(sourcePage.items);
      setPlans(planPage.items);
      setMethods(methodPage.items);
      setAdapters(adapterPage.items);
      setDrafts(draftPage.items);
      setTypes(typePage.items);
      setError(null);
    } catch (problem) {
      setError(problem);
    } finally {
      setLoading(false);
    }
  }, [scope]);

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
    // 项目可见性已由后端按活跃项目过滤（含共享信源），这里只做本地关键词/健康/类型筛选。
    const filtered = (sources ?? []).filter((source) => {
      if (typeFilter !== "all" && source.sourceType !== typeFilter) return false;
      if (health !== "all" && source.health !== health) return false;
      if (flag === "enabled" && !source.enabled) return false;
      if (flag === "disabled" && source.enabled) return false;
      if (flag === "shared" && !source.shared) return false;
      if (flag === "alert" && !(source.alert || source.stopReason || source.health === "failed" || source.health === "configurationError")) return false;
      if (!needle) return true;
      return (
        source.name.toLowerCase().includes(needle) ||
        source.id.toLowerCase().includes(needle) ||
        source.endpoint.toLowerCase().includes(needle)
      );
    });
    return sortRows(filtered, sort, (source, key) => {
      if (key === "name") return source.name;
      if (key === "health") return source.health ?? "";
      if (key === "yield") return source.todayYield ?? 0;
      if (key === "nextRun") return source.nextRunAt ?? "";
      if (key === "plans") {
        return plans.filter(
          (plan) =>
            plan.id === source.id ||
            (plan.targets ?? []).some((target) => target === source.id || target === `source/${source.id}`),
        ).length;
      }
      return "";
    });
  }, [sources, search, health, typeFilter, flag, sort, plans]);

  // 类型 id → 产品命名的展示映射，来自后端注册表（不在前端复刻矩阵）。
  const typeLabelOf = useMemo(() => {
    const map = new Map<string, string>();
    for (const item of types) map.set(item.id, item.label);
    return (id: string) => map.get(id) ?? id;
  }, [types]);

  const pagedRows = useMemo(() => slicePage(rows, page, pageSize), [rows, page, pageSize]);

  // 筛选集变化后当前页可能越界，回到第一页。
  useEffect(() => {
    setPage(1);
    setSelection([]);
  }, [search, health, typeFilter, scope, flag, sources?.length]);

  // 计划归计划页管理：信源页只呈现关联数量，点击跳转过滤。
  const planCountOf = (sourceId: string) =>
    plans.filter(
      (plan) =>
        plan.id === sourceId ||
        (plan.targets ?? []).some((target) => target === sourceId || target === `source/${sourceId}`),
    ).length;

  // 批量动作逐项执行、保留成功项（与任务页同一语义）。
  async function batchAct(label: string, action: (source: Source) => Promise<unknown>) {
    const targets = rows.filter((row) => selection.includes(row.id));
    if (!targets.length) return;
    setBusy("batch");
    let succeeded = 0;
    let failed = 0;
    for (const source of targets) {
      try {
        await action(source);
        succeeded += 1;
      } catch {
        failed += 1;
      }
    }
    setSelection([]);
    notify(failed ? "warn" : "ok", `${label}：成功 ${succeeded} 个`, failed ? `失败 ${failed} 个（可能停用中被引用等原因）。` : undefined);
    await load();
    setBusy(null);
  }

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

  async function toggleShare(source: Source) {
    const sharing = !source.shared;
    await act(sharing ? "共享信源" : "取消共享", () =>
      sharing ? shareSource(source.id) : unshareSource(source.id),
    );
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
          <h1 className="page-title">信源清单与适配器</h1>
          <p className="page-lede">
            信源声明取什么、怎么解析；执行节律、数量上限与停止条件在「计划」页管理。手动采集与探测共用同一条执行核心；所有修改都经过配置校验与热加载。
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

      <Tabs
        value={tab}
        onChange={setTab}
        items={[
          { id: "list", label: "信源清单", count: rows.length },
          { id: "adapters", label: "适配器", count: adapters.length },
        ]}
      />

      {tab === "list" ? (
      <>
      <div className="toolbar-band">
        <label className="search">
          <span className="muted" aria-hidden="true">⌕</span>
          <input value={search} onChange={(event) => setSearch(event.target.value)} placeholder="搜索名称、标识或地址" aria-label="搜索信源" />
        </label>
        <div className="seg" role="group" aria-label="健康筛选">
          {[
            ["all", "全部"],
            ["healthy", "正常"],
            ["degraded", "能力受限"],
            ["failed", "失败"],
            ["suspicious", "待核实"],
            ["configurationError", "配置错误"],
          ].map(([id, label]) => (
            <button key={id} type="button" className={health === id ? "is-active" : ""} onClick={() => setHealth(id)}>
              {label}
            </button>
          ))}
        </div>
        <FilterPanel active={[typeFilter !== "all", scope === "all", flag !== "all"].filter(Boolean).length} onReset={() => { setTypeFilter("all"); setScope("project"); setFlag("all"); }}>
          <select
            className="select"
            value={typeFilter}
            onChange={(event) => setTypeFilter(event.target.value)}
            aria-label="类型筛选"
          >
            <option value="all">全部信源类型</option>
            {types.map((item) => (
              <option key={item.id} value={item.id}>{item.label}</option>
            ))}
          </select>
          <select className="select" value={flag} onChange={(event) => setFlag(event.target.value as typeof flag)} aria-label="状态附加筛选">
            <option value="all">不限状态</option>
            <option value="enabled">仅启用</option>
            <option value="disabled">仅停用</option>
            <option value="alert">需处理的事项</option>
            <option value="shared">已共享给其他项目</option>
          </select>
          <div className="seg" role="group" aria-label="项目范围">
            <button type="button" className={scope === "all" ? "is-active" : ""} onClick={() => setScope("all")}>
              全部项目
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
        </FilterPanel>
        <span className="fill" />
        <span className="small muted">
          {rows.length} / {sources?.length ?? 0} 个信源
        </span>
      </div>

      {selection.length > 0 ? (
        <div className="batchbar">
          <span>已选 {selection.length} 个信源</span>
          <button type="button" className="btn btn-sm" disabled={busy === "batch"} onClick={() => void batchAct("批量启用", (source) => (source.enabled ? Promise.resolve() : enableSource(source.id, source.etag)))}>
            批量启用
          </button>
          <button type="button" className="btn btn-sm" disabled={busy === "batch"} onClick={() => void batchAct("批量停用", (source) => (source.enabled ? disableSource(source.id, source.etag) : Promise.resolve()))}>
            批量停用
          </button>
          <button
            type="button"
            className="btn btn-sm btn-danger"
            disabled={busy === "batch"}
            onClick={() => {
              if (!window.confirm(`删除选中的 ${selection.length} 个信源？引用它们的声明必须先行处理。`)) return;
              void batchAct("批量删除", (source) => deleteSource(source.id, source.etag));
            }}
          >
            <IconTrash size={12} /> 批量删除
          </button>
          <button type="button" className="btn btn-sm" onClick={() => setSelection([])}>清除选择</button>
        </div>
      ) : null}

      <Card title="信源清单" sub="启停、关联计划与最近一次运行" flush>
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
                  <th style={{ width: 34 }}>
                    <input
                      type="checkbox"
                      aria-label="全选"
                      checked={rows.length > 0 && selection.length === rows.length}
                      onChange={(event) => setSelection(event.target.checked ? rows.map((row) => row.id) : [])}
                    />
                  </th>
                  <th>信源</th>
                  <SortHeader label="健康" sortKey="health" sort={sort} onSort={toggle} />
                  <SortHeader label="计划" sortKey="plans" sort={sort} onSort={toggle} />
                  <SortHeader label="今日产出" sortKey="yield" sort={sort} onSort={toggle} />
                  <SortHeader label="下一次" sortKey="nextRun" sort={sort} onSort={toggle} />
                  <th>状态</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {pagedRows.map((source) => {
                  const planCount = planCountOf(source.id);
                  // 归属当前项目才能改；他项目共享来的只读。
                  const owned = !source.project || source.project === currentProject?.id;
                  return (
                    <tr key={source.id} className={selection.includes(source.id) ? "is-selected" : ""}>
                      <td>
                        <input
                          type="checkbox"
                          aria-label={`选择 ${source.name}`}
                          checked={selection.includes(source.id)}
                          onChange={(event) =>
                            setSelection((current) =>
                              event.target.checked ? [...current, source.id] : current.filter((id) => id !== source.id),
                            )
                          }
                        />
                      </td>
                      <td>
                        <div className="cell-main">
                          <span className="source-name">
                            {source.name}
                            <span className="badge neutral" style={{ marginLeft: 6 }}>{typeLabelOf(source.sourceType)}</span>
                            {source.shared ? <span className="badge neutral" style={{ marginLeft: 6 }}>已共享</span> : null}
                          </span>
                          <span className="source-sub mono">{source.id} · {source.endpoint || (source.channel ? `频道 ${source.channel}` : "")}</span>
                        </div>
                      </td>
                      <td>
                        <div className="cell-main">
                          <StatusBadge status={source.health} />
                          {source.stopReason ? <span className="source-sub">命中限额/停止：{source.stopReason}</span> : null}
                          {source.alert && source.sourceType === "web" ? (
                            <span className="source-sub" style={{ color: "var(--danger)" }}>疑似改版失效，请检查页面结构</span>
                          ) : null}
                        </div>
                      </td>
                      <td className="small">
                        {planCount > 0 ? (
                          <button
                            type="button"
                            className="btn btn-ghost btn-sm"
                            onClick={() => navigate(`plans?target=${encodeURIComponent(source.id.startsWith("source/") ? source.id : `source/${source.id}`)}`)}
                          >
                            关联计划 {planCount}
                          </button>
                        ) : source.sourceType === "push" ? (
                          <span className="muted">入口监听</span>
                        ) : (
                          <span className="muted">未配置计划</span>
                        )}
                      </td>
                      <td className="mono">{source.todayYield ?? 0}</td>
                      <td className="small muted">{relative(source.nextRunAt)}</td>
                      <td>
                        <span className={`badge ${source.enabled ? "ok" : "neutral"}`}>{source.enabled ? "启用" : "停用"}</span>
                      </td>
                      <td className="actions">
                        {owned ? (
                          <div className="row" style={{ justifyContent: "flex-end" }}>
                            <button type="button" className="btn btn-sm" title="查看采集记录" aria-label={`查看 ${source.name} 的采集记录`} onClick={() => setRecord(source)}>
                              <IconHistory size={12} /> 记录
                            </button>
                            {source.sourceType === "push" ? null : (
                              <button type="button" className="btn btn-sm" onClick={() => void collect(source)} disabled={busy === `collect-${source.id}` || !source.enabled}>
                                <IconPlay size={12} /> 采集
                              </button>
                            )}
                            {source.sourceType === "push" ? null : (
                              <button type="button" className="btn btn-sm" onClick={() => void probe(source)} disabled={busy === `probe-${source.id}`}>
                                <IconFlask size={12} /> 探测
                              </button>
                            )}
                            <button type="button" className="btn btn-sm" onClick={() => setEditing(source)}>
                              编辑
                            </button>
                            <button
                              type="button"
                              className="btn btn-sm"
                              onClick={() => void toggleShare(source)}
                              disabled={busy === "共享信源" || busy === "取消共享"}
                            >
                              {source.shared ? "取消共享" : "共享"}
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
                        ) : (
                          <div className="row" style={{ justifyContent: "flex-end", gap: 6 }}>
                            <span className="badge neutral">他项目共享</span>
                            <span className="small muted">只读</span>
                          </div>
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
        {rows.length > pageSize ? (
          <Pager
            page={page}
            pageSize={pageSize}
            total={rows.length}
            onPage={setPage}
            onPageSize={(size) => {
              setPageSize(size);
              setPage(1);
            }}
          />
        ) : null}
      </Card>
      </>
      ) : null}

      {tab === "adapters" ? (
      <div className="stack" style={{ gap: 16 }}>
        <Card
          title="可用适配器"
          sub="适配器负责把信源原始响应解析成条目；下面列出当前可勾选的适配器与各自用在哪类信源"
          action={
            <button type="button" className="btn btn-sm" onClick={() => setEditing("new")}>
              <IconPlus size={12} /> 用适配器新建信源
            </button>
          }
          flush
        >
          {adapters.length === 0 ? (
            <Empty title="没有可用适配器" text="检查 config/adapters 下的声明是否通过校验。" />
          ) : (
            <div className="list">
              {adapters.map((adapter) => {
                const used = (sources ?? []).filter((source) => source.adapterIds.includes(adapter.id));
                return (
                  <div className="list-row" key={adapter.id}>
                    <span className="list-main">
                      <span className="list-title">
                        {adapter.name}
                        {adapter.builtin ? <span className="badge neutral" style={{ marginLeft: 6 }}>内置</span> : null}
                        {String(adapter.role ?? "") ? <span className="badge neutral" style={{ marginLeft: 6 }}>{String(adapter.role)}</span> : null}
                      </span>
                      <span className="list-meta">
                        <span className="mono">{adapter.id}</span>
                        {adapter.description ? <span>{String(adapter.description)}</span> : null}
                        {Array.isArray(adapter.produces) && adapter.produces.length ? (
                          <span>产出：{adapter.produces.map((item) => String((item as Record<string, unknown>)?.kind ?? item)).join("、")}</span>
                        ) : null}
                      </span>
                    </span>
                    {used.length > 0 ? (
                      <button
                        type="button"
                        className="btn btn-ghost btn-sm"
                        onClick={() => setSearch(used[0].id)}
                        title={used.map((source) => source.name).join("、")}
                      >
                        {used.length} 个信源在用
                      </button>
                    ) : (
                      <span className="small muted">暂无信源使用</span>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </Card>

        <Card
          title="AI 生成草稿"
          sub="AI 生成的适配器在批准前不会写入配置；批准后可与内置适配器一样勾选"
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

        <Card title="声明示例" sub="自定义适配器写成一份 YAML，放进 config/adapters 即参与校验与勾选">
          <div className="code">{ADAPTER_EXAMPLE}</div>
        </Card>
      </div>
      ) : null}

      {editing ? (
        <SourceDrawer
          source={editing === "new" ? null : editing}
          methods={methods}
          adapters={adapters}
          types={types}
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

      {record ? (
        <RunsDrawer
          title={`采集记录 · ${record.name}`}
          sourceId={record.id}
          onClose={() => setRecord(null)}
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
  types,
  onClose,
  onAnalyze,
  onSaved,
}: {
  source: Source | null;
  methods: NamedResource[];
  adapters: NamedResource[];
  types: SourceTypeRecord[];
  onClose: () => void;
  onAnalyze: () => void;
  onSaved: () => void | Promise<void>;
}) {
  const { notify, run } = useUi();
  const [form, setForm] = useState<SourceInput>(() =>
    source
      ? {
          name: source.name,
          sourceType: source.sourceType,
          accessMethodId: source.accessMethodId,
          adapterIds: source.adapterIds,
          enabled: source.enabled,
          endpoint: source.endpoint,
          channel: source.channel ?? "",
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

  // 类型剖面向表单收窄：接入方式取注册表的 formAccessors（简易表单能落盘的子集），
  // 适配器取 allowedAdapters；前端只渲染，判断一律在后端。
  const selectedType = types.find((item) => item.id === (form.sourceType ?? "feed"));
  const formAccessors = selectedType?.formAccessors ?? null;
  const allowedAdapters = selectedType?.allowedAdapters ?? null;
  const methodOptions = formAccessors ? methods.filter((item) => formAccessors.includes(item.id)) : methods;
  const adapterOptions = allowedAdapters ? adapters.filter((item) => allowedAdapters.includes(item.id)) : adapters;
  const isPush = (form.sourceType ?? "feed") === "push";

  function changeType(id: string) {
    const next = types.find((item) => item.id === id);
    const accessors = next?.formAccessors ?? methods.map((item) => item.id);
    const allowed = next?.allowedAdapters ?? adapters.map((item) => item.id);
    setForm((current) => {
      const accessMethodId = accessors.includes(current.accessMethodId)
        ? current.accessMethodId
        : accessors[0] ?? current.accessMethodId;
      let adapterIds = current.adapterIds.filter((item) => allowed.includes(item));
      if (adapterIds.length === 0) adapterIds = allowed.slice(0, 1);
      const schedule: Schedule =
        id === "push"
          ? { mode: "push" }
          : current.schedule.mode === "push"
            ? { mode: "fixed", intervalMs: 7_200_000 }
            : current.schedule;
      return { ...current, sourceType: id, accessMethodId, adapterIds, schedule, channel: id === "push" ? current.channel ?? "" : "" };
    });
  }

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
    // 信源不再承载节律：采集节律落到计划。这里只写静态声明，节律经计划表达。
    const { schedule, ...base } = {
      ...form,
      schedule:
        form.schedule.mode === "push"
          ? { mode: "push" }
          : { ...form.schedule, intervalMs: Math.max(60_000, Math.round(intervalValue) * 60_000) },
    };
    const minutes = Math.max(1, Math.round(intervalValue));
    const trigger =
      schedule.mode === "push"
        ? { kind: "oneShot", fireAt: "immediate" }
        : { kind: "recurring", interval: `${minutes}m` };
    try {
      if (!base.name.trim()) {
        throw new Error("名称不能为空");
      }
      if (base.sourceType === "push") {
        if (!(base.channel ?? "").trim()) throw new Error("接收类信源需要频道标识");
      } else if (!base.endpoint.trim()) {
        throw new Error("地址不能为空");
      }
      if (base.adapterIds.length === 0) {
        throw new Error("至少选择一个适配器");
      }
      // 接收类不写地址与节律：后端按 channel 落 locator.channel，并去掉 schedule。
      const payload: SourcePayload =
        base.sourceType === "push"
          ? { ...base, endpoint: "", channel: (base.channel ?? "").trim() }
          : { ...base, channel: undefined };
      if (!source) {
        const created = await createSource(payload);
        // 新建信源配一个同名计划（默认关闭），确认后在计划页启用；接收类由入口监听驱动，不建计划。
        if (created.sourceType !== "push") {
          try {
            // 默认关闭：新建信源先核对，采集在计划页确认后再启用。
            await createPlan({ name: `${created.name} · 采集计划`, targets: [created.id], trigger, enabled: false });
          } catch {
            // 计划创建失败不回滚信源，可在计划页补建。
          }
        }
        // 归属由后端按当前项目自动写入，无需手动绑定。
        notify("ok", "信源已创建并归入当前项目");
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
          {!isPush ? (
            <button type="button" className="btn btn-sm" onClick={onAnalyze}>
              <IconFlask size={13} /> 让 AI 分析地址
            </button>
          ) : null}
          {source && !isPush ? (
            <button type="button" className="btn btn-sm" onClick={() => void collectNow()}>
              <IconPlay size={13} /> 立即采集
            </button>
          ) : null}
        </div>
        <Field label="名称" htmlFor="source-name">
          <input id="source-name" className="input" value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} placeholder="例如 Hugging Face Blog" />
        </Field>
        <Field label="信源类型" hint="先定类型：地址/频道与可选的接入方式、适配器都随类型收窄">
          <select className="select" value={form.sourceType ?? "feed"} onChange={(event) => changeType(event.target.value)}>
            {types.map((item) => (
              <option key={item.id} value={item.id}>{item.label}</option>
            ))}
          </select>
        </Field>
        {isPush ? (
          <Field label="频道标识" hint="接收类信源由外部按频道推送，不抓取地址；采集进程据此频道监听。" htmlFor="source-channel">
            <input id="source-channel" className="input mono" value={form.channel ?? ""} onChange={(event) => setForm({ ...form, channel: event.target.value })} placeholder="items" />
          </Field>
        ) : (
          <Field label="地址" hint="只接受 http/https。单 URL 采集会替换这个地址，但不会改写声明。" htmlFor="source-endpoint">
            <input id="source-endpoint" className="input" value={form.endpoint} onChange={(event) => setForm({ ...form, endpoint: event.target.value })} placeholder="https://example.com/feed.xml" />
          </Field>
        )}
        <div className="grid cols-2">
          <Field label="接入方式">
            <select className="select" value={form.accessMethodId} onChange={(event) => setForm({ ...form, accessMethodId: event.target.value })}>
              {methodOptions.map((method) => (
                <option key={method.id} value={method.id}>{method.name}</option>
              ))}
            </select>
          </Field>
          <Field
            label="采集节奏"
            hint={
              source
                ? "节律已由采集计划承载，请到「计划」页调整这条信源所属计划的节律与额度。"
                : "信源不再承载节律：创建后会生成一个默认关闭的同名计划，确认无误再到「计划」页启用。"
            }
          >
            <div className="row">
              <span className="muted small">
                {form.schedule.mode === "push" ? "随推送（一次性触发）" : "在「计划」页配置"}
              </span>
            </div>
          </Field>
        </div>
        <Field label="适配器" hint="适配器负责把原始响应解析为内容；可多选并按顺序执行">
          <div className="stack" style={{ gap: 6 }}>
            {adapterOptions.map((adapter) => (
              <label key={adapter.id} className="switch" style={{ justifyContent: "space-between" }}>
                <span className="stack" style={{ gap: 2 }}>
                  <span>{adapter.name}</span>
                  {adapter.description ? <span className="small muted">{String(adapter.description)}</span> : null}
                </span>
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
          <dl className="kv">
            <dt>信源标识</dt><dd className="mono">{source.id}</dd>
            <dt>健康状态</dt><dd>{source.health || "—"}{source.healthReason ? ` · ${source.healthReason}` : ""}</dd>
            <dt>关联引用</dt><dd>{typeof source.references === "number" ? source.references : "—"}</dd>
            <dt>配置版本</dt><dd className="mono">{source.etag}</dd>
          </dl>
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
            <StructuredValue value={proposal} />
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
