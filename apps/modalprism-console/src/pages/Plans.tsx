import { useCallback, useEffect, useMemo, useState } from "react";

import { IconHistory, IconPause, IconPlay, IconPlus, IconRefresh, IconTrash } from "../components/icons";
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
  pausePlan,
  resumePlan,
  updatePlan,
} from "../lib/api";
import { relative, stamp } from "../lib/format";
import { navigate, useRoute } from "../lib/router";
import type { DeliveryTarget, DeliveryTargetGroup, Plan, Source, SourceTypeRecord } from "../lib/types";

interface Draft {
  name: string;
  targets: string[];
  kind: "recurring" | "oneShot";
  interval: string;
  perRunItems: string;
  totalItems: string;
  maxAge: string;
  onBudgetExhausted: string;
  onFailureThreshold: string;
  /** 计划维度的投递目标，键为 `kind:id`。 */
  deliveryTargets: string[];
  deliveryEnabled: boolean;
  /** 创建/保存后是否立即进入调度；默认关闭，避免误触即开始采集。 */
  enabled: boolean;
}

const EMPTY: Draft = {
  name: "",
  targets: [],
  kind: "recurring",
  interval: "",
  perRunItems: "",
  totalItems: "",
  maxAge: "",
  onBudgetExhausted: "",
  onFailureThreshold: "",
  deliveryTargets: [],
  deliveryEnabled: true,
  enabled: false,
};

// 信源 id 统一成带前缀形式：PATCH 不经过 extract_targets 归一，需前端补全。
const srcId = (id: string) => (id.startsWith("source/") ? id : `source/${id}`);

// 由草稿构造额度对象（只含已填字段；编辑时整份替换，清空即移除）。
function buildQuota(draft: Draft): Record<string, unknown> {
  const quota: Record<string, unknown> = {};
  if (draft.perRunItems.trim()) quota.perRunItems = Number(draft.perRunItems);
  if (draft.totalItems.trim()) quota.totalItems = Number(draft.totalItems);
  if (draft.maxAge.trim()) quota.maxAge = draft.maxAge.trim();
  return quota;
}

function buildStopPolicy(draft: Draft): Record<string, unknown> {
  const stopPolicy: Record<string, unknown> = {};
  if (draft.onBudgetExhausted) stopPolicy.onBudgetExhausted = draft.onBudgetExhausted;
  if (draft.onFailureThreshold.trim()) stopPolicy.onFailureThreshold = Number(draft.onFailureThreshold);
  return stopPolicy;
}

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
  const [deliveryGroups, setDeliveryGroups] = useState<DeliveryTargetGroup[]>([]);
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

  // 按类型套用创建期默认剧本（v3 R17）：来自注册表 planDefaults，仅用于新建时预填。
  const defaultsOf = (typeId: string): Partial<Draft> => {
    const record = typeRecords.find((item) => item.id === typeId);
    if (!record) return {};
    const trigger = record.planDefaults.trigger as Record<string, unknown> | null | undefined;
    const quota = record.planDefaults.quota ?? {};
    const stop = record.planDefaults.stopPolicy ?? {};
    return {
      interval: typeof trigger?.interval === "string" ? trigger.interval : "",
      perRunItems: quota.perRunItems != null ? String(quota.perRunItems) : "",
      maxAge: typeof quota.maxAge === "string" ? quota.maxAge : "",
      onBudgetExhausted: typeof stop.onBudgetExhausted === "string" ? stop.onBudgetExhausted : "",
      onFailureThreshold: stop.onFailureThreshold != null ? String(stop.onFailureThreshold) : "",
    };
  };

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
      const [planPage, sourcePage, typePage, catalog] = await Promise.all([
        listPlans(),
        listSources({ limit: 200 }),
        listSourceTypes(),
        listDeliveryTargets(),
      ]);
      setPlans(planPage.items);
      setSources(sourcePage.items);
      setTypeRecords(typePage.items);
      setDeliveryTargets(catalog.items);
      setDeliveryGroups(catalog.groups);
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
      // 选第一个目标时按其信源类型套默认剧本；用户随后可改。
      if (adding && current.targets.length === 0) {
        return { ...current, ...defaultsOf(sourceTypeOf(id)), targets };
      }
      return { ...current, targets };
    });

  // 表单里的投递目标：`kind:id` → `{kind,id}`，空数组表示不投递。
  const deliveryPayload = () =>
    draft.deliveryTargets.map((value) => {
      const at = value.indexOf(":");
      return { kind: value.slice(0, at), id: value.slice(at + 1) };
    });

  const submit = async () => {
    if (draft.targets.length === 0) {
      notify("warn", "至少选择一个信源");
      return;
    }
    setBusy("create");
    try {
      if (mode === "job") {
        const result = await createCollectionJob({ targets: draft.targets, name: draft.name || "一次性采集作业" });
        notify(
          result.failed.length ? "warn" : "ok",
          `批量作业：已启动 ${result.dispatched.length} 个`,
          result.failed.length ? `${result.failed.length} 个信源启动失败。` : undefined,
        );
      } else {
        const trigger: Record<string, unknown> = { kind: draft.kind };
        if (draft.kind === "recurring") trigger.interval = draft.interval || "1h";
        else trigger.fireAt = "immediate";
        const quota = buildQuota(draft);
        const stopPolicy = buildStopPolicy(draft);
        if (mode === "edit" && editing) {
          // 编辑整份替换 quota/stopPolicy：清空字段即从计划中移除该限制。
          const patch: Record<string, unknown> = {
            name: draft.name || "采集计划",
            targets: draft.targets.map(srcId),
            trigger,
            quota,
            stopPolicy,
            enabled: draft.enabled,
            deliveryTargets: deliveryPayload(),
            deliveryEnabled: draft.deliveryEnabled,
          };
          await updatePlan(editing.id, patch, editing.etag);
          notify("ok", "计划已更新");
        } else {
          const input: Record<string, unknown> = {
            name: draft.name || "采集计划",
            targets: draft.targets,
            trigger,
            enabled: draft.enabled,
            deliveryTargets: deliveryPayload(),
            deliveryEnabled: draft.deliveryEnabled,
          };
          if (Object.keys(quota).length) input.quota = quota;
          if (Object.keys(stopPolicy).length) input.stopPolicy = stopPolicy;
          await createPlan(input);
          notify("ok", draft.enabled ? "计划已创建并启用" : "计划已创建（未启用采集）");
        }
      }
      setOpen(false);
      setDraft(EMPTY);
      setEditing(null);
      await load();
    } catch (caught) {
      notify(
        "bad",
        mode === "job" ? "作业创建失败" : mode === "edit" ? "更新失败" : "创建失败",
        caught instanceof Error ? caught.message : undefined,
      );
    } finally {
      setBusy(null);
    }
  };

  const start = (nextMode: "plan" | "job") => {
    setMode(nextMode);
    setEditing(null);
    setDraft({ ...EMPTY, kind: nextMode === "job" ? "oneShot" : "recurring" });
    setOpen(true);
  };

  const startEdit = (plan: Plan) => {
    const quota = plan.quota;
    const stop = plan.stopPolicy as { onBudgetExhausted?: string; onFailureThreshold?: number } | null | undefined;
    setMode("edit");
    setEditing(plan);
    setDraft({
      name: plan.name ?? "",
      targets: plan.targets ?? [],
      kind: plan.trigger?.kind === "oneShot" ? "oneShot" : "recurring",
      interval: plan.trigger?.interval ?? "2h",
      perRunItems: quota?.perRunItems ? String(quota.perRunItems) : "",
      totalItems: quota?.totalItems ? String(quota.totalItems) : "",
      maxAge: quota?.maxAge ?? "",
      onBudgetExhausted: stop?.onBudgetExhausted ?? "",
      onFailureThreshold: stop?.onFailureThreshold ? String(stop.onFailureThreshold) : "",
      deliveryTargets: (plan.deliveryTargets ?? []).map((item) => `${item.kind}:${item.id}`),
      deliveryEnabled: plan.deliveryEnabled ?? true,
      enabled: plan.enabled ?? plan.status !== "paused",
    });
    setOpen(true);
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
        sub="计划定义执行节律、数量上限、目标信源与投递目标；信源只声明取什么、怎么解析。"
        action={
          <div className="row" style={{ gap: 8 }}>
            <button type="button" className="btn btn-sm" onClick={() => void load()} disabled={loading}>
              <IconRefresh size={15} /> 刷新
            </button>
            <button type="button" className="btn btn-sm" onClick={() => start("job")}>
              批量采集作业
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
            title={target ? "该信源还没有关联计划" : "还没有采集计划"}
            text={target ? "到计划新建时为它选择节律，或查看全部计划。" : "新建一个计划，选择一个或多个信源并设定节律。"}
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
                        <strong className="cell-ellipsis" title={plan.name}>{plan.name}</strong>
                        <span className="small muted cell-ellipsis" title={plan.id}>{plan.id}</span>
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
                        <button type="button" className="btn btn-ghost btn-sm" disabled={busy === plan.id} onClick={() => startEdit(plan)}>
                          编辑
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
              ? "选择信源立即执行一次采集，相当于只跑一次的计划。"
              : "计划定义执行节律、数量上限、目标信源与投递目标；上限用于控制数据量。"
          }
          onClose={() => setOpen(false)}
          wide
          footer={
            <div className="row between">
              <button type="button" className="btn btn-ghost" onClick={() => setOpen(false)}>
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
                  {draft.enabled ? "创建/保存后立即启用采集" : "保持关闭，稍后在计划列表启用"}
                </span>
              </label>
            ) : null}
            {mode !== "job" ? (
              <Field label="触发">
                <div className="row" style={{ gap: 12 }}>
                  <label className="row" style={{ gap: 4 }}>
                    <input type="radio" name="kind" checked={draft.kind === "recurring"} onChange={() => setDraft({ ...draft, kind: "recurring" })} />
                    周期
                  </label>
                  <label className="row" style={{ gap: 4 }}>
                    <input type="radio" name="kind" checked={draft.kind === "oneShot"} onChange={() => setDraft({ ...draft, kind: "oneShot" })} />
                    一次性
                  </label>
                  {draft.kind === "recurring" ? (
                    <input className="input" style={{ maxWidth: 120 }} value={draft.interval} placeholder="2h" onChange={(event) => setDraft({ ...draft, interval: event.target.value })} />
                  ) : null}
                </div>
              </Field>
            ) : null}
            {mode !== "job" ? (
              <>
                <Field label="数量上限（留空表示不限制）">
                  <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
                    <input className="input" style={{ maxWidth: 140 }} value={draft.perRunItems} placeholder="单次上限" onChange={(event) => setDraft({ ...draft, perRunItems: event.target.value })} />
                    <input className="input" style={{ maxWidth: 140 }} value={draft.totalItems} placeholder="累计上限" onChange={(event) => setDraft({ ...draft, totalItems: event.target.value })} />
                    <input className="input" style={{ maxWidth: 150 }} value={draft.maxAge} placeholder="时效（如 365d）" onChange={(event) => setDraft({ ...draft, maxAge: event.target.value })} />
                  </div>
                </Field>
                <Field label="停止策略">
                  <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
                    <select className="input" style={{ maxWidth: 180 }} value={draft.onBudgetExhausted} onChange={(event) => setDraft({ ...draft, onBudgetExhausted: event.target.value })}>
                      <option value="">预算耗尽：默认</option>
                      <option value="pause">预算耗尽：暂停</option>
                      <option value="skip">预算耗尽：跳过</option>
                    </select>
                    <input className="input" style={{ maxWidth: 160 }} value={draft.onFailureThreshold} placeholder="连续失败阈值" onChange={(event) => setDraft({ ...draft, onFailureThreshold: event.target.value })} />
                  </div>
                </Field>
              </>
            ) : null}
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
            {mode !== "job" ? (
              <Field label="投递目标（可多选，留空表示不自动投递）">
                <div className="stack" style={{ gap: 8 }}>
                  {deliveryGroups.map((group) => {
                    const items = deliveryTargets.filter((item) => item.kind === group.kind);
                    if (!items.length) return null;
                    return (
                      <div key={group.kind} className="stack" style={{ gap: 4 }}>
                        <span className="small muted" title={group.note}>
                          {group.label}
                        </span>
                        <div className="row wrap" style={{ gap: 12 }}>
                          {items.map((item) => {
                            const key = `${item.kind}:${item.id}`;
                            return (
                              <label key={key} className="row" style={{ gap: 5 }}>
                                <input
                                  type="checkbox"
                                  checked={draft.deliveryTargets.includes(key)}
                                  onChange={() =>
                                    setDraft({
                                      ...draft,
                                      deliveryTargets: draft.deliveryTargets.includes(key)
                                        ? draft.deliveryTargets.filter((value) => value !== key)
                                        : [...draft.deliveryTargets, key],
                                    })
                                  }
                                />
                                <span className="small">{item.name ?? item.id}</span>
                                {item.enabled === false ? <span className="badge neutral">已停用</span> : null}
                              </label>
                            );
                          })}
                        </div>
                      </div>
                    );
                  })}
                  {deliveryTargets.length === 0 ? (
                    <span className="small muted">还没有可用目标：模型在系统设置登记，外部 API 与通知渠道在内容投递页维护。</span>
                  ) : null}
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={draft.deliveryEnabled}
                      disabled={!draft.deliveryTargets.length}
                      onChange={(event) => setDraft({ ...draft, deliveryEnabled: event.target.checked })}
                    />
                    <span>启用自动投递</span>
                  </label>
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
