import { useCallback, useEffect, useMemo, useState } from "react";

import { IconHistory, IconPause, IconPlay, IconPlus, IconRefresh, IconTrash } from "../components/icons";
import { RowSubject } from "../components/ListRow";
import { Pager, slicePage } from "../components/Pager";
import { RunsDrawer } from "../components/RunsDrawer";
import { Card, Empty, Field, Loading, Modal, Problem, StatusBadge, useUi } from "../components/ui";
import {
  createCollectionJob,
  createPlan,
  deletePlan,
  listDeliveryTargets,
  listPlans,
  listSources,
  listSourceTypes,
  listStrategies,
  pausePlan,
  resumePlan,
  updatePlan,
} from "../lib/api";
import { relative, stamp } from "../lib/format";
import { navigate, useRoute } from "../lib/router";
import type { DeliveryTarget, Plan, Source, SourceTypeRecord, Strategy } from "../lib/types";

interface Draft {
  name: string;
  /** 一次性作业（job 模式）的信源选择；recurring 计划的目标由策略承载。 */
  targets: string[];
  // 引用的策略（裸 slug）。单一语义：节律/额度/目标等行为字段全部由策略承载。
  strategy: string;
  /** 创建/保存后是否立即进入调度；默认关闭，避免误触即开始采集。 */
  enabled: boolean;
}

const EMPTY: Draft = {
  name: "",
  targets: [],
  strategy: "",
  enabled: false,
};

// 信源 id 统一成带前缀形式：PATCH 不经过 extract_targets 归一，需前端补全。
const srcId = (id: string) => (id.startsWith("source/") ? id : `source/${id}`);

export function PlansPage() {
  const { notify } = useUi();
  const { params } = useRoute();
  const [plans, setPlans] = useState<Plan[] | null>(null);
  const [sources, setSources] = useState<Source[]>([]);
  const [typeRecords, setTypeRecords] = useState<SourceTypeRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const [mode, setMode] = useState<"plan" | "job" | "edit">("plan");
  // 采集记录速览：计划派发的每次运行及其产出，不离开本页。
  const [record, setRecord] = useState<Plan | null>(null);
  const [draft, setDraft] = useState<Draft>(EMPTY);
  const [editing, setEditing] = useState<Plan | null>(null);
  const [selection, setSelection] = useState<string[]>([]);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  // 投递目标目录：计划表单里直接选，底层存成监听该计划的自动投递规则。
  const [deliveryTargets, setDeliveryTargets] = useState<DeliveryTarget[]>([]);
  // 策略目录：新建/编辑时选择引用，表单字段按所选策略预填，可覆写。
  const [strategies, setStrategies] = useState<Strategy[]>([]);
  // 信源页「关联计划」跳转：#plans?target=source/xxx 按信源过滤。
  const target = params.get("target") ?? "";

  const sourceLabel = useMemo(() => {
    const map = new Map<string, string>();
    for (const source of sources) map.set(source.id, source.name ?? source.id);
    return map;
  }, [sources]);

  // 类型 id → 产品命名；以及信源 id → 类型 id（均来自后端数据，不在前端复刻矩阵）。
  const typeLabelOf = useMemo(() => {
    const map = new Map<string, string>();
    for (const item of typeRecords) map.set(item.id, item.label);
    return (id: string) => map.get(id) ?? id;
  }, [typeRecords]);

  const sourceTypeOf = useMemo(() => {
    const map = new Map<string, string>();
    for (const source of sources) map.set(source.id, source.sourceType);
    return (id: string) => map.get(id) ?? "";
  }, [sources]);

  // 计划行类型：取首个目标的信源类型（经 targets join，不冗余存储）。
  const planTypeLabel = (plan: Plan): string | null => {
    const first = (plan.targets ?? [])[0];
    if (!first) return null;
    const id = first.replace(/^source\//, "");
    const type = sourceTypeOf(id);
    return type ? typeLabelOf(type) : null;
  };

  const targetName = useMemo(() => {
    const map = new Map<string, string>();
    for (const item of deliveryTargets) map.set(`${item.kind}:${item.id}`, item.name ?? item.id);
    return (kind: string, id: string) => map.get(`${kind}:${id}`) ?? id;
  }, [deliveryTargets]);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [planPage, sourcePage, typePage, catalog, strategyPage] = await Promise.all([
        listPlans(),
        listSources({ limit: 200 }),
        listSourceTypes(),
        listDeliveryTargets(),
        listStrategies().catch(() => null),
      ]);
      setPlans(planPage.items);
      setSources(sourcePage.items);
      setTypeRecords(typePage.items);
      setDeliveryTargets(catalog.items);
      // 策略目录缺失（旧后端）不阻塞计划页：仅下拉不可用。
      if (strategyPage) setStrategies(strategyPage.items ?? []);
    } catch (caught) {
      setError(caught);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const toggleTarget = (id: string) =>
    setDraft((current) => {
      const adding = !current.targets.includes(id);
      const targets = adding ? [...current.targets, id] : current.targets.filter((item) => item !== id);
      // 单一语义：节律/额度只来自策略基座，选目标不再内联预填行为字段。
      return { ...current, targets };
    });

  // 表单里的投递目标：`kind:id` → `{kind,id}`，空数组表示不投递。
  
  const start = (nextMode: "plan" | "job") => {
    setMode(nextMode);
    setEditing(null);
    setDraft({ ...EMPTY });
    setOpen(true);
  };

  const startEdit = (plan: Plan) => {
    setMode("edit");
    setEditing(plan);
    setDraft({
      name: plan.name ?? "",
      targets: [],
      strategy: plan.strategy ? plan.strategy.replace(/^strategy\//, "") : "",
      enabled: plan.enabled ?? plan.status !== "paused",
    });
    setOpen(true);
  };

  // 单一语义：策略选择即完成行为配置，计划不预填也不覆写行为字段。
  const pickStrategy = (slug: string) => setDraft((current) => ({ ...current, strategy: slug }));

  const submit = async () => {
    if (mode === "job") {
      if (draft.targets.length === 0) {
        notify("warn", "至少选择一个信源");
        return;
      }
      setBusy("create");
      try {
        const result = await createCollectionJob({ targets: draft.targets, name: draft.name || "一次性采集作业" });
        notify(
          result.failed.length ? "warn" : "ok",
          `批量作业：已启动 ${result.dispatched.length} 个`,
          result.failed.length ? `${result.failed.length} 个信源启动失败。` : undefined,
        );
        setOpen(false);
        setDraft(EMPTY);
        setEditing(null);
        await load();
      } catch (caught) {
        notify("bad", "作业创建失败", caught instanceof Error ? caught.message : undefined);
      } finally {
        setBusy(null);
      }
      return;
    }
    // 单一语义：计划是策略的状态实例，创建/更新只提交引用与状态字段。
    if (!draft.strategy) {
      notify("warn", "计划必须引用策略（有计划必有策略）：请先选择或创建策略");
      return;
    }
    setBusy("create");
    try {
      if (mode === "edit" && editing) {
        await updatePlan(
          editing.id,
          { name: draft.name || "采集计划", strategy: draft.strategy, enabled: draft.enabled },
          editing.etag,
        );
        notify("ok", "计划已更新");
      } else {
        await createPlan({ name: draft.name || "采集计划", strategy: draft.strategy, enabled: draft.enabled });
        notify("ok", draft.enabled ? "计划已创建并启用" : "计划已创建（未启用采集）");
      }
      setOpen(false);
      setDraft(EMPTY);
      setEditing(null);
      await load();
    } catch (caught) {
      notify(
        "bad",
        mode === "edit" ? "更新失败" : "创建失败",
        caught instanceof Error ? caught.message : undefined,
      );
    } finally {
      setBusy(null);
    }
  };

  const act = async (plan: Plan, action: "pause" | "resume" | "delete") => {
    setBusy(plan.id);
    try {
      if (action === "pause") await pausePlan(plan.id, plan.etag);
      else if (action === "resume") await resumePlan(plan.id, plan.etag);
      else await deletePlan(plan.id);
      notify("ok", action === "delete" ? "计划已删除" : action === "pause" ? "计划已暂停" : "计划已恢复");
      await load();
    } catch (caught) {
      notify("bad", "操作失败", caught instanceof Error ? caught.message : undefined);
    } finally {
      setBusy(null);
    }
  };

  // 深链：从概览或信源跳到某份计划时，直接打开它的编辑面板。
  useEffect(() => {
    if (!plans) return;
    const id = params.get("id");
    if (!id) return;
    const hit = plans.find((plan) => plan.id === id);
    if (hit && editing?.id !== hit.id) startEdit(hit);
  }, [plans, params, editing]);

  // 打开计划编辑：同时在地址栏留下 `?id=`，刷新后还能回到同一份计划。
  const openPlan = (plan: Plan) => {
    startEdit(plan);
    navigate(`plans?id=${encodeURIComponent(plan.id)}`);
  };

  // 关掉面板时清参数，避免刷新后又被重新打开。
  const closePanel = () => {
    setOpen(false);
    if (params.get("id")) navigate("plans");
  };

  const rows = useMemo(() => {
    const list = plans ?? [];
    if (!target) return list;
    const id = srcId(target);
    return list.filter(
      (plan) => plan.id === target || (plan.targets ?? []).some((t) => t === target || t === id || srcId(t) === id),
    );
  }, [plans, target]);

  const pagedRows = useMemo(() => slicePage(rows, page, pageSize), [rows, page, pageSize]);

  // 过滤条件或数据变化后回到第一页，清空选择避免跨筛选残留。
  useEffect(() => {
    setPage(1);
    setSelection([]);
  }, [target, plans?.length]);

  const selectable = rows.filter((plan) => selection.includes(plan.id));
  const canPause = selectable.filter((plan) => plan.enabled).length;
  const canResume = selectable.filter((plan) => !plan.enabled).length;

  // 批量动作逐项执行、保留成功项（与任务页同一语义）。
  async function batchAct(label: string, action: "pause" | "resume" | "delete") {
    const targets = rows.filter((plan) => selection.includes(plan.id));
    if (!targets.length) return;
    setBusy("batch");
    let succeeded = 0;
    let failed = 0;
    for (const plan of targets) {
      try {
        if (action === "pause") await pausePlan(plan.id, plan.etag);
        else if (action === "resume") await resumePlan(plan.id, plan.etag);
        else await deletePlan(plan.id);
        succeeded += 1;
      } catch {
        failed += 1;
      }
    }
    notify(failed ? "warn" : "ok", `${label}：成功 ${succeeded} 个`, failed ? `失败 ${failed} 个。` : undefined);
    setSelection([]);
    await load();
    setBusy(null);
  }

  const cadence = (plan: Plan): string => {
    const trigger = plan.trigger ?? {};
    if (trigger.kind === "oneShot") return "一次性";
    return trigger.interval ? `每 ${trigger.interval}` : "—";
  };

  const targets = (plan: Plan): string =>
    (plan.targets ?? []).map((id) => sourceLabel.get(id) ?? id.replace(/^source\//, "")).join("、") || "—";

  const deliveryText = (plan: Plan): string => {
    const chosen = plan.deliveryTargets ?? [];
    if (!chosen.length) return "—";
    const names = chosen.map((item) => targetName(item.kind, item.id)).join("、");
    return plan.deliveryEnabled === false ? `${names}（已停用）` : names;
  };

  const limitsText = (plan: Plan): string => {
    const q = plan.quota;
    const parts: string[] = [];
    if (q?.perRunItems) parts.push(`每次 ≤${q.perRunItems}`);
    if (q?.totalItems) parts.push(`总计 ≤${q.totalItems}`);
    if (q?.maxAge) parts.push(`时效 ≤${q.maxAge}`);
    return parts.join(" · ");
  };

  const stopText = (plan: Plan): string => {
    const sp = plan.stopPolicy as { onBudgetExhausted?: string; onFailureThreshold?: number } | null | undefined;
    const parts: string[] = [];
    if (sp?.onBudgetExhausted === "skip") parts.push("预算耗尽跳过");
    else if (sp?.onBudgetExhausted === "pause") parts.push("预算耗尽暂停");
    if (sp?.onFailureThreshold) parts.push(`连错 ${sp.onFailureThreshold} 停`);
    return parts.join(" · ");
  };

  return (
    <div className="page">
      {target ? (
        <div className="alert info">
          正在按信源过滤计划：{sourceLabel.get(srcId(target)) ?? target.replace(/^source\//, "")}。
          <button type="button" className="btn btn-ghost btn-sm" onClick={() => navigate("plans")}>查看全部计划</button>
        </div>
      ) : null}
      {selection.length > 0 ? (
        <div className="batchbar">
          <span>已选 {selection.length} 个计划</span>
          <button type="button" className="btn btn-sm" disabled={busy === "batch" || canPause === 0} onClick={() => void batchAct("批量暂停", "pause")}>
            批量暂停
          </button>
          <button type="button" className="btn btn-sm" disabled={busy === "batch" || canResume === 0} onClick={() => void batchAct("批量恢复", "resume")}>
            批量恢复
          </button>
          <button
            type="button"
            className="btn btn-sm btn-danger"
            disabled={busy === "batch"}
            onClick={() => {
              if (!window.confirm(`删除选中的 ${selection.length} 个计划？其历史运行记录不受影响。`)) return;
              void batchAct("批量删除", "delete");
            }}
          >
            <IconTrash size={12} /> 批量删除
          </button>
          <button type="button" className="btn btn-sm" onClick={() => setSelection([])}>清除选择</button>
        </div>
      ) : null}
      <Card
        title="采集计划"
        sub="计划定义执行节律、数量上限、目标信源与投递目标；信源声明采集地址与解析适配器。"
        action={
          <div className="row" style={{ gap: 8 }}>
            <button type="button" className="btn btn-sm" onClick={() => void load()} disabled={loading}>
              <IconRefresh size={15} /> 刷新
            </button>
            <button type="button" className="btn btn-sm" onClick={() => start("job")}>
              批量采集作业
            </button>
            <button type="button" className="btn btn-sm" onClick={() => navigate("strategies")}>
              策略管理
            </button>
            <button type="button" className="btn btn-sm" onClick={() => start("plan")}>
              <IconPlus size={15} /> 新建计划
            </button>
          </div>
        }
        flush
      >
        {loading ? (
          <Loading />
        ) : error ? (
          <Problem error={error} onRetry={() => void load()} />
        ) : rows.length === 0 ? (
          <Empty
            title={target ? "该信源暂无关联计划" : "暂无采集计划"}
            text={target ? "可在计划页选择节律，或查看全部计划。" : "新建计划：选择信源并设置节律。"}
          />
        ) : (
          <div className="table-wrap">
            <table className="table table-fixed plan-table">
              <thead>
                <tr>
                  <th className="w-check">
                    <input
                      type="checkbox"
                      aria-label="全选"
                      checked={rows.length > 0 && selection.length === rows.length}
                      onChange={(event) => setSelection(event.target.checked ? rows.map((row) => row.id) : [])}
                    />
                  </th>
                  <th className="w-name">计划</th>
                  <th className="w-targets">信源</th>
                  <th className="w-delivery">投递</th>
                  <th className="w-cadence">节律</th>
                  <th className="w-quota">额度 / 停止</th>
                  <th className="w-status">状态</th>
                  <th className="w-next">下次</th>
                  <th className="w-actions">操作</th>
                </tr>
              </thead>
              <tbody>
                {pagedRows.map((plan) => (
                  <tr key={plan.id} className={selection.includes(plan.id) ? "is-selected" : ""}>
                    <td>
                      <input
                        type="checkbox"
                        aria-label={`选择 ${plan.name ?? plan.id}`}
                        checked={selection.includes(plan.id)}
                        onChange={(event) =>
                          setSelection((current) =>
                            event.target.checked ? [...current, plan.id] : current.filter((id) => id !== plan.id),
                          )
                        }
                      />
                    </td>
                    <td className="w-name">
                      <div className="stack" style={{ gap: 2 }}>
                        <RowSubject onClick={() => openPlan(plan)} title="编辑计划">
                          {plan.name}
                        </RowSubject>
                        <span className="small muted cell-ellipsis" title={plan.id}>{plan.id}</span>
                        {plan.strategy ? (
                          <span
                            className="badge neutral"
                            title={`引用策略 ${plan.strategy}；计划自带字段是对它的覆写`}
                            style={{ alignSelf: "flex-start" }}
                          >
                            策略 · {plan.strategy.replace(/^strategy\//, "")}
                          </span>
                        ) : null}
                      </div>
                    </td>
                    <td className="small w-targets">
                      <div className="row" style={{ gap: 6 }}>
                        <span className="cell-ellipsis" title={targets(plan)}>{targets(plan)}</span>
                        {planTypeLabel(plan) ? <span className="badge neutral">{planTypeLabel(plan)}</span> : null}
                      </div>
                    </td>
                    <td className="small w-delivery"><span className="cell-ellipsis" title={deliveryText(plan)}>{deliveryText(plan)}</span></td>
                    <td className="small w-cadence"><span className="cell-ellipsis" title={cadence(plan)}>{cadence(plan)}</span></td>
                    <td className="small w-quota">
                      <div className="stack" style={{ gap: 2 }}>
                        <span className="cell-ellipsis" title={limitsText(plan)}>{limitsText(plan) || "—"}</span>
                        {stopText(plan) ? <span className="muted cell-ellipsis" title={stopText(plan)}>{stopText(plan)}</span> : null}
                      </div>
                    </td>
                    <td className="w-status">
                      <StatusBadge status={plan.status} />
                    </td>
                    <td className="small muted w-next" title={plan.nextRunAt ? stamp(plan.nextRunAt) : undefined}>
                      {plan.nextRunAt ? relative(plan.nextRunAt) : "—"}
                    </td>
                    <td className="w-actions">
                      <div className="row" style={{ gap: 4 }}>
                        <button type="button" className="btn btn-ghost btn-sm" title="采集记录" aria-label={`查看 ${plan.name} 的采集记录`} onClick={() => setRecord(plan)}>
                          <IconHistory size={14} />
                        </button>
                        {plan.enabled ? (
                          <button type="button" className="btn btn-ghost btn-sm" title="暂停计划" aria-label="暂停计划" disabled={busy === plan.id} onClick={() => void act(plan, "pause")}>
                            <IconPause size={14} />
                          </button>
                        ) : (
                          <button type="button" className="btn btn-ghost btn-sm" title="恢复计划" aria-label="恢复计划" disabled={busy === plan.id} onClick={() => void act(plan, "resume")}>
                            <IconPlay size={14} />
                          </button>
                        )}
                        <button type="button" className="btn btn-ghost btn-sm" title="删除计划" aria-label="删除计划" disabled={busy === plan.id} onClick={() => void act(plan, "delete")}>
                          <IconTrash size={14} />
                        </button>
                      </div>
                    </td>
                  </tr>
                ))}
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

      {open ? (
        <Modal
          title={mode === "job" ? "批量采集作业" : mode === "edit" ? "编辑采集计划" : "新建采集计划"}
          sub={
            mode === "job"
              ? "选择信源立即执行一次采集（一次性作业）。"
              : "计划定义执行节律、数量上限、目标信源与投递目标；数量上限用于控制数据量。"
          }
          onClose={closePanel}
          wide
          footer={
            <div className="row between">
              <button type="button" className="btn btn-ghost" onClick={closePanel}>
                取消
              </button>
              <button type="button" className="btn" disabled={busy === "create"} onClick={() => void submit()}>
                {mode === "job" ? "开始作业" : mode === "edit" ? "保存" : "创建"}
              </button>
            </div>
          }
        >
          <div className="stack" style={{ gap: 14 }}>
            <Field label="名称">
              <input className="input" value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} />
            </Field>
            {mode !== "job" ? (
              <label className="switch">
                <input type="checkbox" checked={draft.enabled} onChange={(event) => setDraft({ ...draft, enabled: event.target.checked })} />
                <span>
                  {draft.enabled ? "创建/保存后立即启用采集" : "默认关闭；可在计划列表启用"}
                </span>
              </label>
            ) : null}
            {mode !== "job" ? (
              <Field label="策略（节律与额度的唯一定义点，必选）">
                <select
                  className="input"
                  value={draft.strategy}
                  onChange={(event) => pickStrategy(event.target.value)}
                  disabled={strategies.length === 0}
                >
                  <option value="" disabled>
                    {strategies.length === 0 ? "暂无可用策略，请先建策略" : "请选择策略"}
                  </option>
                  {strategies.map((item) => (
                    <option key={item.slug} value={item.slug}>
                      {item.name ?? item.slug}
                    </option>
                  ))}
                </select>
                {strategies.length === 0 ? (
                  <span className="small muted">
                    有计划必有策略：请先在策略页创建一份策略，再回到这里建计划。
                  </span>
                ) : (
                  <span className="small muted">
                    已按策略预填下方字段，可覆写；引用同一策略的计划互不影响。
                  </span>
                )}
              </Field>
            ) : null}
            {mode === "job" ? (
              <Field label="目标信源">
                <div className="stack" style={{ gap: 4, maxHeight: 220, overflow: "auto" }}>
                  {sources.map((source) => (
                    <label key={source.id} className="row" style={{ gap: 6 }}>
                      <input type="checkbox" checked={draft.targets.includes(source.id)} onChange={() => toggleTarget(source.id)} />
                      <span>{source.name ?? source.id}</span>
                      <span className="badge neutral">{typeLabelOf(source.sourceType)}</span>
                    </label>
                  ))}
                </div>
              </Field>
            ) : null}

          </div>
        </Modal>
      ) : null}

      {record ? (
        <RunsDrawer
          title={`采集记录 · ${record.name}`}
          planId={record.id}
          onClose={() => setRecord(null)}
        />
      ) : null}
    </div>
  );
}
