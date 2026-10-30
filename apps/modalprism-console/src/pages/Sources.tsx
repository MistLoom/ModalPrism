import { useCallback, useEffect, useMemo, useState } from "react";

import {
  IconCheck,
  IconClose,
  IconFlask,
  IconHistory,
  IconPencil,
  IconPlay,
  IconPlus,
  IconRefresh,
  IconTrash,
} from "../components/icons";
import { RowSubject } from "../components/ListRow";
import { RunsDrawer } from "../components/RunsDrawer";
import { Card, Drawer, Empty, Field, Loading, Modal, Problem, StatusBadge, StructuredValue, Tabs, useUi } from "../components/ui";
import { FilterPanel, SortHeader, sortRows, useSort } from "../components/Table";
import { Pager, slicePage } from "../components/Pager";
import {
  analyzeSource,
  approveAdapterDraft,
  assignSourcesToGroup,
  createAdapterDraft,
  createCollectionRun,
  createPlan,
  createSource,
  createSourceGroup,
  deleteSource,
  deleteSourceGroup,
  disableSource,
  enableSource,
  listAdapterDrafts,
  listAdapters,
  listAccessMethods,
  listPlans,
  listSources,
  listSourceGroups,
  listSourceTypes,
  probeSource,
  rejectAdapterDraft,
  shareSource,
  unshareSource,
  updateSource,
  updateSourceGroup,
} from "../lib/api";
import { relative, statusLabel, statusTone, stopReasonLabel } from "../lib/format";
import { useCurrentProject } from "../lib/project";
import { navigate, useRoute } from "../lib/router";
import type { NamedResource, Operation, Plan, Schedule, Source, SourceGroup, SourceInput, SourcePayload, SourceTypeRecord } from "../lib/types";

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

/** 上次所在的分类：分类是长期工作范围，下次打开还在原处。 */
const GROUP_STORAGE = "modalprism.sourceGroup";

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
  // 分类是工作范围，不是筛选：选中分类即进入它的管理范围，并沿用到下次打开。
  const [groups, setGroups] = useState<SourceGroup[]>([]);
  const [groupId, setGroupId] = useState(() => params.get("group") ?? window.localStorage.getItem(GROUP_STORAGE) ?? "all");
  const [groupModal, setGroupModal] = useState<"new" | SourceGroup | null>(null);
  const [assignOpen, setAssignOpen] = useState(false);
  const { sort, toggle } = useSort();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [sourcePage, planPage, methodPage, adapterPage, draftPage, typePage, groupPage] = await Promise.all([
        // 项目隔离由后端按 x-project-id 处理；"全部"显式请求 scope=all 取消过滤。
        listSources({ limit: 200, scope: scope === "all" ? "all" : undefined }),
        listPlans({ limit: 200 }),
        listAccessMethods(),
        listAdapters(),
        listAdapterDrafts(),
        listSourceTypes(),
        // “全部项目”时一并取消分类过滤：共享信源的分类才回显得出来。
        listSourceGroups(scope === "all" ? { scope: "all" } : {}),
      ]);
      setSources(sourcePage.items);
      setPlans(planPage.items);
      setMethods(methodPage.items);
      setAdapters(adapterPage.items);
      setDrafts(draftPage.items);
      setTypes(typePage.items);
      setGroups(groupPage.items);
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

  // 深链：从内容详情、日志健康或概览跳过来时，直接打开这家信源的编辑面板。
  useEffect(() => {
    if (!sources) return;
    const id = params.get("id");
    if (!id) return;
    const hit = sources.find((source) => source.id === id || source.id === id.replace(/^source\//, ""));
    if (hit) setEditing(hit);
  }, [sources, params]);

  useEffect(() => {
    const timer = window.setTimeout(() => setSearch(params.get("query") ?? search), 200);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [params]);

  const rows = useMemo(() => {
    const needle = search.trim().toLowerCase();
    // 分类先定工作范围，其余条件才在范围内收窄；两者语义不同，不互相覆盖。
    const localGroup = (source: Source) =>
      source.groupId && groups.some((group) => group.id === source.groupId) ? source.groupId : "";
    const filtered = (sources ?? []).filter((source) => {
      if (groupId !== "all") {
        const own = localGroup(source);
        if (groupId === "none" ? own !== "" : own !== groupId) return false;
      }
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
  }, [sources, search, health, typeFilter, flag, sort, plans, groupId, groups]);

  // 类型 id → 产品命名的展示映射，来自后端注册表（不在前端复刻矩阵）。
  const typeLabelOf = useMemo(() => {
    const map = new Map<string, string>();
    for (const item of types) map.set(item.id, item.label);
    return (id: string) => map.get(id) ?? id;
  }, [types]);

  const pagedRows = useMemo(() => slicePage(rows, page, pageSize), [rows, page, pageSize]);

  // 分类 id → 名称；只认当前项目的分类，他项目共享来的信源按未分组处理。
  const groupNameOf = useMemo(() => {
    const map = new Map(groups.map((group) => [group.id, group.name]));
    return (source: Source) => (source.groupId && map.has(source.groupId) ? map.get(source.groupId) ?? "" : "");
  }, [groups]);

  const scopeGroup = groupId === "all" || groupId === "none" ? null : groups.find((group) => group.id === groupId) ?? null;

  const groupCounts = useMemo(() => {
    const per = new Map<string, number>();
    let ungrouped = 0;
    for (const source of sources ?? []) {
      const own = source.groupId && groups.some((group) => group.id === source.groupId) ? source.groupId : "";
      if (own) per.set(own, (per.get(own) ?? 0) + 1);
      else ungrouped += 1;
    }
    return { per, ungrouped, total: sources?.length ?? 0 };
  }, [sources, groups]);

  // 记忆里的分类可能已被删除：回到全部，避免空范围。
  useEffect(() => {
    if (groupId === "all" || groupId === "none" || !groups.length) return;
    if (!groups.some((group) => group.id === groupId)) setGroupId("all");
  }, [groups, groupId]);

  // 筛选集变化后当前页可能越界，回到第一页。
  useEffect(() => {
    setPage(1);
    setSelection([]);
  }, [search, health, typeFilter, scope, flag, sources?.length, groupId]);

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

  // 进入分类即切换工作范围：地址栏与本地记忆同时留存，刷新或下次打开仍在原分类。
  function selectGroup(next: string) {
    setGroupId(next);
    setPage(1);
    setSelection([]);
    window.localStorage.setItem(GROUP_STORAGE, next);
    navigate(next === "all" ? "sources" : `sources?group=${encodeURIComponent(next)}`);
  }

  // 分类管理动作与筛选无关：批量改变归属后当前范围可能变空，重载即可。
  async function moveSelection(target: string | null) {
    if (!selection.length) return;
    setBusy("assign");
    try {
      const result = await assignSourcesToGroup({ groupId: target, sourceIds: selection });
      setSelection([]);
      setAssignOpen(false);
      notify(
        result.skipped.length ? "warn" : "ok",
        `已移动 ${result.updated.length} 个信源`,
        result.skipped.length ? `${result.skipped.length} 个未移动：${result.skipped[0].reason}` : undefined,
      );
      await load();
    } catch (problem) {
      notify("bad", "移动信源失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(null);
    }
  }

  async function removeGroup(group: SourceGroup) {
    if (group.builtin) {
      notify("bad", "内置分类不可删除", "可改名，或把成员移出后自建分类。");
      return;
    }
    const members = groupCounts.per.get(group.id) ?? 0;
    if (!window.confirm(`删除分类「${group.name}」？其中 ${members} 个信源会退回未分组。`)) return;
    setBusy(`group-${group.id}`);
    try {
      await deleteSourceGroup(group.id);
      notify("ok", "分类已删除", "成员已退回未分组，信源本身不受影响。");
      if (groupId === group.id) selectGroup("all");
      await load();
    } catch (problem) {
      notify("bad", "删除分类失败", problem instanceof Error ? problem.message : undefined);
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
          <h1 className="page-title">信源</h1>
          <p className="page-lede">
            信源声明采集地址与解析适配器；执行节律、数量上限与停止条件由计划管理。左侧分类用于长期管理，筛选仅收窄当前视图。修改经配置校验后热加载。
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
      <div className="source-layout">
        <aside className="source-rail" aria-label="信源分类">
          <div className="source-rail-head">
            <span>分类</span>
            <button type="button" className="btn btn-sm" onClick={() => setGroupModal("new")}>
              <IconPlus size={12} /> 新建
            </button>
          </div>
          <div className={`source-rail-row${groupId === "all" ? " is-active" : ""}`}>
            <button type="button" className="source-rail-pick" onClick={() => selectGroup("all")}>
              <span className="source-rail-name">全部信源</span>
              <span className="source-rail-count">{groupCounts.total}</span>
            </button>
          </div>
          {groups.map((group) => (
            <div key={group.id} className={`source-rail-row${groupId === group.id ? " is-active" : ""}`}>
              <button
                type="button"
                className="source-rail-pick"
                onClick={() => selectGroup(group.id)}
                title={group.description || group.name}
              >
                <span className="source-rail-name">
                  {group.name}
                  {group.builtin ? <span className="source-rail-tag">内置</span> : null}
                </span>
                <span className="source-rail-count">{groupCounts.per.get(group.id) ?? 0}</span>
              </button>
              <button
                type="button"
                className="source-rail-edit"
                aria-label={`编辑分类 ${group.name}`}
                onClick={() => setGroupModal(group)}
              >
                <IconPencil size={13} />
              </button>
            </div>
          ))}
          <div className={`source-rail-row${groupId === "none" ? " is-active" : ""}`}>
            <button type="button" className="source-rail-pick" onClick={() => selectGroup("none")}>
              <span className="source-rail-name">未分组</span>
              <span className="source-rail-count">{groupCounts.ungrouped}</span>
            </button>
          </div>
          <p className="source-rail-note">分类用于长期管理；筛选仅收窄当前视图，不改变归属。</p>
        </aside>

        <div className="source-main">
      {scopeGroup || groupId === "none" ? (
        <div className="scope-band">
          <div className="scope-copy">
            <span className="scope-kicker">正在管理分类</span>
            <strong>{scopeGroup ? scopeGroup.name : "未分组"}</strong>
            {scopeGroup?.description ? <span className="small muted">{scopeGroup.description}</span> : null}
          </div>
          <span className="fill" />
          <span className="small muted">
            {scopeGroup ? groupCounts.per.get(scopeGroup.id) ?? 0 : groupCounts.ungrouped} 个信源
          </span>
          {scopeGroup ? (
            <>
              <button type="button" className="btn btn-sm" onClick={() => setGroupModal(scopeGroup)}>
                <IconPencil size={12} /> 重命名
              </button>
              {scopeGroup.builtin ? (
                <span className="small muted">内置分类不可删除</span>
              ) : (
                <button
                  type="button"
                  className="btn btn-sm btn-danger"
                  disabled={busy === `group-${scopeGroup.id}`}
                  onClick={() => void removeGroup(scopeGroup)}
                >
                  <IconTrash size={12} /> 删除分类
                </button>
              )}
            </>
          ) : null}
        </div>
      ) : null}
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
          <button type="button" className="btn btn-sm" disabled={busy === "assign"} onClick={() => setAssignOpen(true)}>
            移入分类…
          </button>
          {scopeGroup || groupId === "none" ? (
            <button type="button" className="btn btn-sm" disabled={busy === "assign"} onClick={() => void moveSelection(null)}>
              移出分类
            </button>
          ) : null}
          <button
            type="button"
            className="btn btn-sm btn-danger"
            disabled={busy === "batch"}
            onClick={() => {
              if (!window.confirm(`删除选中的 ${selection.length} 个信源？需先处理引用它们的声明。`)) return;
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
            title={sources?.length ? "无匹配信源" : "暂无信源"}
            text={sources?.length ? "调整关键词或健康筛选。" : "从模板新建订阅源，或用 AI 分析目标地址。"}
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
                          <RowSubject onClick={() => setEditing(source)} title="编辑信源">
                            {source.name}
                            <span className="badge neutral" style={{ marginLeft: 6 }}>{typeLabelOf(source.sourceType)}</span>
                            {source.shared ? <span className="badge neutral" style={{ marginLeft: 6 }}>已共享</span> : null}
                            {groupId === "all" && groupNameOf(source) ? (
                              <span className="badge neutral" style={{ marginLeft: 6 }}>{groupNameOf(source)}</span>
                            ) : null}
                          </RowSubject>
                          <span className="source-sub mono">{source.id} · {source.endpoint || (source.channel ? `频道 ${source.channel}` : "")}</span>
                        </div>
                      </td>
                      <td>
                        <div className="cell-main">
                          <StatusBadge status={source.health} />
                          {source.stopReason ? <span className="source-sub">命中限额/停止：{stopReasonLabel(source.stopReason)}</span> : null}
                          {source.alert && source.sourceType === "web" ? (
                            <span className="source-sub" style={{ color: "var(--danger)" }}>疑似页面改版导致失效，请检查页面结构</span>
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
                                if (!window.confirm(`删除信源「${source.name}」？需先处理引用它的声明。`)) return;
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
        </div>
      </div>
      </>
      ) : null}

      {tab === "adapters" ? (
      <div className="stack" style={{ gap: 16 }}>
        <Card
          title="可用适配器"
          sub="适配器将信源原始响应解析为条目；以下为当前可选适配器及其适用信源类型"
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
          sub="AI 生成的适配器需批准后写入配置，并可与内置适配器一样选用"
          action={
            <button type="button" className="btn btn-sm" onClick={() => setDraftOpen(true)}>
              <IconPlus size={12} /> 新建草稿
            </button>
          }
          flush
        >
          {drafts.length === 0 ? (
            <Empty title="暂无适配器草稿" text="先执行信源分析，再根据页面结构生成适配器。" />
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

        <Card title="声明示例" sub="自定义适配器以 YAML 声明放入 config/adapters，参与校验与选用">
          <div className="code">{ADAPTER_EXAMPLE}</div>
        </Card>
      </div>
      ) : null}

      {groupModal ? (
        <GroupModal
          group={groupModal === "new" ? null : groupModal}
          onClose={() => setGroupModal(null)}
          onSaved={async (saved) => {
            const created = groupModal === "new";
            setGroupModal(null);
            await load();
            if (created) selectGroup(saved.id);
          }}
        />
      ) : null}

      {assignOpen ? (
        <AssignGroupModal
          groups={groups}
          count={selection.length}
          busy={busy === "assign"}
          onClose={() => setAssignOpen(false)}
          onConfirm={(target) => void moveSelection(target)}
        />
      ) : null}

      {editing ? (
        <SourceDrawer
          source={editing === "new" ? null : editing}
          methods={methods}
          adapters={adapters}
          types={types}
          groups={groups}
          defaultGroupId={scopeGroup?.id ?? ""}
          onClose={() => {
            setEditing(null);
            // 深链打开的编辑面板关掉后清参数，避免刷新又被重新打开。
            if (params.get("id")) navigate("sources");
          }}
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
  groups,
  defaultGroupId,
  onClose,
  onAnalyze,
  onSaved,
}: {
  source: Source | null;
  methods: NamedResource[];
  adapters: NamedResource[];
  types: SourceTypeRecord[];
  groups: SourceGroup[];
  /** 在当前分类里新建时默认归入该分类。 */
  defaultGroupId: string;
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
          // 他项目共享来的分类不在本项目的分类清单里，表单按未分组处理。
          groupId: source.groupId && groups.some((group) => group.id === source.groupId) ? source.groupId : "",
        }
      : { ...EMPTY, adapterIds: [...EMPTY.adapterIds], schedule: { ...EMPTY.schedule }, groupId: defaultGroupId },
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
        <Field label="信源类型" hint="信源类型决定地址或频道、接入方式与适配器选项">
          <select className="select" value={form.sourceType ?? "feed"} onChange={(event) => changeType(event.target.value)}>
            {types.map((item) => (
              <option key={item.id} value={item.id}>{item.label}</option>
            ))}
          </select>
        </Field>
        <Field label="分类" hint="只影响管理归组，不改变采集方式；可在信源页批量调整成员">
          <select
            className="select"
            value={form.groupId ?? ""}
            onChange={(event) => setForm({ ...form, groupId: event.target.value })}
          >
            <option value="">未分组</option>
            {groups.map((group) => (
              <option key={group.id} value={group.id}>{group.name}</option>
            ))}
          </select>
        </Field>
        {isPush ? (
          <Field label="频道标识" hint="接收类信源由外部按频道推送，不抓取地址；采集进程据此频道监听。" htmlFor="source-channel">
            <input id="source-channel" className="input mono" value={form.channel ?? ""} onChange={(event) => setForm({ ...form, channel: event.target.value })} placeholder="items" />
          </Field>
        ) : (
          <Field label="地址" hint="仅接受 http/https。单 URL 采集替换该地址，不修改声明。" htmlFor="source-endpoint">
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
                ? "节律由采集计划管理；在「计划」页调整该信源所属计划的节律与额度。"
                : "信源不承载节律；创建后生成默认关闭的同名计划，可在「计划」页启用。"
            }
          >
            <div className="row">
              <span className="muted small">
                {form.schedule.mode === "push" ? "随推送（一次性触发）" : "在「计划」页配置"}
              </span>
            </div>
          </Field>
        </div>
        <Field label="适配器" hint="适配器将原始响应解析为内容；可多选并按顺序执行">
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
            <dt>健康状态</dt><dd>{statusLabel(source.health)}{source.healthReason ? ` · ${source.healthReason}` : ""}</dd>
            <dt>关联引用</dt><dd>{typeof source.references === "number" ? source.references : "—"}</dd>
            <dt>配置版本</dt><dd className="mono">{source.etag?.replace(/"/g, "")}</dd>
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
      notify("ok", "分析完成", "草稿需人工确认后创建信源。");
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
      sub="使用系统设置中的默认模型与版本化提示词；结果为草稿"
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
            <div className="alert info">模型输出为草稿；保存前需核对接入方式、地址与风险。</div>
            <StructuredValue value={proposal} />
          </>
        ) : null}
      </div>
    </Modal>
  );
}

function GroupModal({
  group,
  onClose,
  onSaved,
}: {
  group: SourceGroup | null;
  onClose: () => void;
  onSaved: (saved: SourceGroup) => void | Promise<void>;
}) {
  const { notify } = useUi();
  const [name, setName] = useState(group?.name ?? "");
  const [description, setDescription] = useState(group?.description ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function save() {
    if (!name.trim()) {
      setError("分类名不能为空");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const saved = group
        ? await updateSourceGroup(group.id, { name: name.trim(), description })
        : await createSourceGroup({ name: name.trim(), description });
      notify("ok", group ? "分类已更新" : "分类已创建");
      await onSaved(saved);
    } catch (problem) {
      setError(problem instanceof Error ? problem.message : "保存失败");
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title={group ? "编辑分类" : "新建分类"}
      sub="分类用于长期管理信源；不改变采集方式，与筛选不同"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>取消</button>
          <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void save()}>
            {busy ? "保存中…" : "保存"}
          </button>
        </>
      }
    >
      <div className="stack" style={{ gap: 12 }}>
        {error ? <div className="alert bad">{error}</div> : null}
        {group?.builtin ? (
          <div className="alert info">内置分类随仓库提供，可改名、改说明与移出成员；不支持删除。</div>
        ) : null}
        <Field label="分类名称">
          <input
            className="input"
            value={name}
            maxLength={40}
            onChange={(event) => setName(event.target.value)}
            placeholder="例如 金融数据"
          />
        </Field>
        <Field label="说明" hint="可选；说明该分类收录的信源范围">
          <textarea className="textarea" value={description} onChange={(event) => setDescription(event.target.value)} />
        </Field>
      </div>
    </Modal>
  );
}

function AssignGroupModal({
  groups,
  count,
  busy,
  onClose,
  onConfirm,
}: {
  groups: SourceGroup[];
  count: number;
  busy: boolean;
  onClose: () => void;
  onConfirm: (target: string | null) => void;
}) {
  const [target, setTarget] = useState("");
  return (
    <Modal
      title="移动到分类"
      sub={`已选择 ${count} 个信源；他项目共享信源只读，将被跳过`}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>取消</button>
          <button type="button" className="btn btn-primary" disabled={busy || count === 0} onClick={() => onConfirm(target || null)}>
            {busy ? "移动中…" : "移动"}
          </button>
        </>
      }
    >
      <Field label="目标分类" hint="选择「未分组」将成员移出当前分类">
        <select className="select" value={target} onChange={(event) => setTarget(event.target.value)}>
          <option value="">未分组</option>
          {groups.map((group) => (
            <option key={group.id} value={group.id}>{group.name}</option>
          ))}
        </select>
      </Field>
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
