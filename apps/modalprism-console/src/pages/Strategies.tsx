import { useCallback, useEffect, useState } from "react";

import { IconPlus, IconRefresh, IconTrash } from "../components/icons";
import { RowSubject } from "../components/ListRow";
import { Card, Empty, Field, Loading, Modal, Problem, useUi } from "../components/ui";
import {
  createStrategy,
  deleteStrategy,
  listDeliveryTargets,
  listSources,
  listStrategies,
  updateStrategy,
} from "../lib/api";
import type { DeliveryTarget, DeliveryTargetGroup, Source, Strategy } from "../lib/types";

interface Draft {
  id: string;
  name: string;
  description: string;
  kind: "recurring" | "oneShot";
  interval: string;
  /** 目标信源：行为的唯一定义点归策略，计划只引用。 */
  targets: string[];
  perRunItems: string;
  totalItems: string;
  maxAge: string;
  onBudgetExhausted: string;
  onFailureThreshold: string;
  /** 计划维度的投递目标，键为 `kind:id`。 */
  deliveryTargets: string[];
  deliveryEnabled: boolean;
}

const EMPTY: Draft = {
  id: "",
  name: "",
  description: "",
  kind: "recurring",
  interval: "",
  targets: [],
  perRunItems: "",
  totalItems: "",
  maxAge: "",
  onBudgetExhausted: "",
  onFailureThreshold: "",
  deliveryTargets: [],
  deliveryEnabled: true,
};

// 由草稿构造策略文档（只含已填字段；编辑时整份替换，清空即移除）。
function buildDoc(draft: Draft): Record<string, unknown> {
  const trigger: Record<string, unknown> = { kind: draft.kind };
  if (draft.kind === "recurring" && draft.interval.trim()) trigger.interval = draft.interval.trim();
  const quota: Record<string, unknown> = {};
  if (draft.perRunItems.trim()) quota.perRunItems = Number(draft.perRunItems);
  if (draft.totalItems.trim()) quota.totalItems = Number(draft.totalItems);
  if (draft.maxAge.trim()) quota.maxAge = draft.maxAge.trim();
  const stopPolicy: Record<string, unknown> = {};
  if (draft.onBudgetExhausted) stopPolicy.onBudgetExhausted = draft.onBudgetExhausted;
  if (draft.onFailureThreshold.trim()) stopPolicy.onFailureThreshold = Number(draft.onFailureThreshold);
  const doc: Record<string, unknown> = {
    name: draft.name || "采集策略",
    targets: draft.targets,
    trigger,
  };
  if (draft.description.trim()) doc.description = draft.description.trim();
  if (Object.keys(quota).length) doc.quota = quota;
  if (Object.keys(stopPolicy).length) doc.stopPolicy = stopPolicy;
  // 表单键为 `kind:id`，落盘前拆回 `{kind, id}` 对象（与后端 schema 对齐）。
  doc.deliveryTargets = draft.deliveryTargets.map((key) => {
    const at = key.indexOf(":");
    return { kind: key.slice(0, at), id: key.slice(at + 1) };
  });
  doc.deliveryEnabled = draft.deliveryEnabled;
  return doc;
}

const cadence = (strategy: Strategy): string => {
  const trigger = strategy.trigger ?? {};
  if (trigger.kind === "oneShot") return "一次性";
  return trigger.interval ? `每 ${trigger.interval}` : "—";
};

const limitsText = (strategy: Strategy): string => {
  const q = strategy.quota;
  const parts: string[] = [];
  if (q?.perRunItems) parts.push(`每次 ≤${q.perRunItems}`);
  if (q?.totalItems) parts.push(`总计 ≤${q.totalItems}`);
  if (q?.maxAge) parts.push(`时效 ≤${q.maxAge}`);
  return parts.join(" · ");
};

const stopText = (strategy: Strategy): string => {
  const sp = strategy.stopPolicy as { onBudgetExhausted?: string; onFailureThreshold?: number } | null | undefined;
  const parts: string[] = [];
  if (sp?.onBudgetExhausted === "skip") parts.push("预算耗尽跳过");
  else if (sp?.onBudgetExhausted === "pause") parts.push("预算耗尽暂停");
  if (sp?.onFailureThreshold) parts.push(`连错 ${sp.onFailureThreshold} 停`);
  return parts.join(" · ");
};

export function StrategiesPage() {
  const { notify } = useUi();
  const [strategies, setStrategies] = useState<Strategy[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState<Strategy | null>(null);
  const [draft, setDraft] = useState<Draft>(EMPTY);
  const [sources, setSources] = useState<Source[]>([]);
  const [deliveryCatalog, setDeliveryCatalog] = useState<DeliveryTarget[]>([]);
  const [deliveryGroups, setDeliveryGroups] = useState<DeliveryTargetGroup[]>([]);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [page, sourcePage, delivery] = await Promise.all([
        listStrategies(),
        listSources({ limit: 200 }),
        listDeliveryTargets(),
      ]);
      setStrategies(page.items);
      setSources(sourcePage.items);
      setDeliveryCatalog(delivery.items);
      setDeliveryGroups(delivery.groups);
    } catch (caught) {
      setError(caught);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const start = () => {
    setEditing(null);
    setDraft(EMPTY);
    setOpen(true);
  };

  const startEdit = (strategy: Strategy) => {
    const quota = strategy.quota ?? {};
    const stop = (strategy.stopPolicy ?? {}) as { onBudgetExhausted?: string; onFailureThreshold?: number };
    setEditing(strategy);
    setDraft({
      id: strategy.slug,
      name: strategy.name ?? "",
      description: strategy.description ?? "",
      kind: strategy.trigger?.kind === "oneShot" ? "oneShot" : "recurring",
      interval: strategy.trigger?.interval ?? "",
      targets: strategy.targets ?? [],
      perRunItems: quota.perRunItems != null ? String(quota.perRunItems) : "",
      totalItems: quota.totalItems != null ? String(quota.totalItems) : "",
      maxAge: quota.maxAge ?? "",
      onBudgetExhausted: stop.onBudgetExhausted ?? "",
      onFailureThreshold: stop.onFailureThreshold != null ? String(stop.onFailureThreshold) : "",
      deliveryTargets: (strategy.deliveryTargets ?? []).map((item) => `${item.kind}:${item.id}`),
      deliveryEnabled: strategy.deliveryEnabled ?? true,
    });
    setOpen(true);
  };

  const submit = async () => {
    if (!draft.kind || (draft.kind === "recurring" && !draft.interval.trim())) {
      notify("warn", "周期策略需要填写间隔（如 2h）");
      return;
    }
    if (draft.kind === "recurring" && draft.targets.length === 0) {
      notify("warn", "周期策略至少要有一个目标信源（策略是目标的唯一定义点）");
      return;
    }
    setBusy("create");
    try {
      if (editing) {
        await updateStrategy(editing.slug, buildDoc(draft));
        notify("ok", "策略已更新；引用它的计划在下次解析时生效");
      } else {
        const input: Record<string, unknown> = buildDoc(draft);
        if (draft.id.trim()) input.id = draft.id.trim();
        await createStrategy(input);
        notify("ok", "策略已创建，可在计划中引用");
      }
      setOpen(false);
      await load();
    } catch (caught) {
      notify("bad", editing ? "更新失败" : "创建失败", caught instanceof Error ? caught.message : undefined);
    } finally {
      setBusy(null);
    }
  };

  const remove = async (strategy: Strategy) => {
    setBusy(strategy.slug);
    try {
      await deleteStrategy(strategy.slug);
      notify("ok", "策略已删除");
      await load();
    } catch (caught) {
      // 引用护栏：仍被计划引用时后端拒绝删除并回显占用方。
      notify("bad", "删除失败", caught instanceof Error ? caught.message : undefined);
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="page">
      <Card
        title="采集策略"
        sub="采集行为的唯一定义点：目标信源、节律、额度与投递都在这里声明；计划引用策略并作为其状态实例运行。"
        action={
          <div className="row" style={{ gap: 8 }}>
            <button type="button" className="btn btn-sm" onClick={() => void load()} disabled={loading}>
              <IconRefresh size={15} /> 刷新
            </button>
            <button type="button" className="btn btn-sm" onClick={start}>
              <IconPlus size={15} /> 新建策略
            </button>
          </div>
        }
        flush
      >
        {loading ? (
          <Loading />
        ) : error ? (
          <Problem error={error} onRetry={() => void load()} />
        ) : !strategies || strategies.length === 0 ? (
          <Empty title="暂无采集策略" text="新建策略：定义节律与额度，供多个计划引用复用。" />
        ) : (
          <div className="table-wrap">
            <table className="table table-fixed plan-table">
              <thead>
                <tr>
                  <th className="w-name">策略</th>
                  <th className="w-cadence">节律</th>
                  <th className="w-quota">额度 / 停止</th>
                  <th className="w-delivery">说明</th>
                  <th className="w-actions">操作</th>
                </tr>
              </thead>
              <tbody>
                {strategies.map((strategy) => (
                  <tr key={strategy.id}>
                    <td className="w-name">
                      <div className="stack" style={{ gap: 2 }}>
                        <RowSubject onClick={() => startEdit(strategy)} title="编辑策略">
                          {strategy.name ?? strategy.slug}
                        </RowSubject>
                        <span className="small muted cell-ellipsis" title={strategy.id}>{strategy.id}</span>
                      </div>
                    </td>
                    <td className="small w-cadence"><span className="cell-ellipsis" title={cadence(strategy)}>{cadence(strategy)}</span></td>
                    <td className="small w-quota">
                      <div className="stack" style={{ gap: 2 }}>
                        <span className="cell-ellipsis" title={limitsText(strategy)}>{limitsText(strategy) || "—"}</span>
                        {stopText(strategy) ? <span className="muted cell-ellipsis" title={stopText(strategy)}>{stopText(strategy)}</span> : null}
                      </div>
                    </td>
                    <td className="small w-delivery">
                      <span className="cell-ellipsis" title={strategy.description ?? undefined}>{strategy.description || "—"}</span>
                    </td>
                    <td className="w-actions">
                      <div className="row" style={{ gap: 4 }}>
                        <button
                          type="button"
                          className="btn btn-ghost btn-sm"
                          title="删除策略"
                          aria-label="删除策略"
                          disabled={busy === strategy.slug}
                          onClick={() => {
                            if (!window.confirm(`删除策略 ${strategy.name ?? strategy.slug}？仍被计划引用时会被拒绝。`)) return;
                            void remove(strategy);
                          }}
                        >
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
      </Card>

      {open ? (
        <Modal
          title={editing ? "编辑采集策略" : "新建采集策略"}
          sub="策略是无状态的行为定义；修改后引用它的计划在下次派发解析时生效。"
          onClose={() => setOpen(false)}
          footer={
            <div className="row between">
              <button type="button" className="btn btn-ghost" onClick={() => setOpen(false)}>
                取消
              </button>
              <button type="button" className="btn" disabled={busy === "create"} onClick={() => void submit()}>
                {editing ? "保存" : "创建"}
              </button>
            </div>
          }
        >
          <div className="stack" style={{ gap: 14 }}>
            <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
              <Field label="名称">
                <input className="input" style={{ minWidth: 200 }} value={draft.name} placeholder="如：标准采集节奏" onChange={(event) => setDraft({ ...draft, name: event.target.value })} />
              </Field>
              {!editing ? (
                <Field label="标识（可选，缺省按内容派生）">
                  <input className="input" style={{ minWidth: 200 }} value={draft.id} placeholder="如：standard" onChange={(event) => setDraft({ ...draft, id: event.target.value })} />
                </Field>
              ) : null}
            </div>
            <Field label="触发">
              <div className="row" style={{ gap: 12 }}>
                <label className="row" style={{ gap: 4 }}>
                  <input type="radio" name="strategy-kind" checked={draft.kind === "recurring"} onChange={() => setDraft({ ...draft, kind: "recurring" })} />
                  周期
                </label>
                <label className="row" style={{ gap: 4 }}>
                  <input type="radio" name="strategy-kind" checked={draft.kind === "oneShot"} onChange={() => setDraft({ ...draft, kind: "oneShot" })} />
                  一次性
                </label>
                {draft.kind === "recurring" ? (
                  <input className="input" style={{ maxWidth: 120 }} value={draft.interval} placeholder="2h" onChange={(event) => setDraft({ ...draft, interval: event.target.value })} />
                ) : null}
              </div>
            </Field>
            <Field label="目标信源（周期策略至少一个；采集目标归策略承载）">
              <div className="stack" style={{ gap: 4, maxHeight: 220, overflow: "auto" }}>
                {sources.map((source) => (
                  <label key={source.id} className="row" style={{ gap: 6 }}>
                    <input
                      type="checkbox"
                      checked={draft.targets.includes(source.id)}
                      onChange={() =>
                        setDraft((current) => ({
                          ...current,
                          targets: current.targets.includes(source.id)
                            ? current.targets.filter((item) => item !== source.id)
                            : [...current.targets, source.id],
                        }))
                      }
                    />
                    <span>{source.name ?? source.id}</span>
                    <span className="badge neutral">{source.sourceType}</span>
                  </label>
                ))}
                {sources.length === 0 ? <span className="small muted">暂无可用信源，请先在信源页创建。</span> : null}
              </div>
            </Field>
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
            <Field label="投递目标（可多选，留空表示不自动投递）">
              <div className="stack" style={{ gap: 8 }}>
                {deliveryGroups.map((group) => {
                  const items = deliveryCatalog.filter((item) => item.kind === group.kind);
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
                {deliveryCatalog.length === 0 ? (
                  <span className="small muted">暂无可用目标：模型在系统设置登记，外部 API 与通知渠道在内容投递页维护。</span>
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
            <Field label="说明">
              <input className="input" value={draft.description} onChange={(event) => setDraft({ ...draft, description: event.target.value })} />
            </Field>
          </div>
        </Modal>
      ) : null}
    </div>
  );
}
