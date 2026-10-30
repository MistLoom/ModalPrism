import { useCallback, useEffect, useMemo, useState } from "react";

import { useUi, Field, Loading, Modal, Problem } from "./ui";
import { PlatformIcon } from "./platforms";
import { createDeliveryJob, listContent, listDeliveryTargets } from "../lib/api";
import { relative } from "../lib/format";
import type { ContentItem, DeliveryTarget, DeliveryTargetGroup } from "../lib/types";

/**
 * 手动投递：选目标 → 提交。
 *
 * 内容页带 `preset` 进来时不再重复选内容，只挑目标；投递页不带 preset 时
 * 才在弹窗里检索内容。两种入口共用同一套目标选择与提交逻辑。
 */
export function ManualDeliveryModal({
  preset,
  onClose,
  onDone,
}: {
  preset?: { items: ContentItem[] };
  onClose: () => void;
  onDone?: () => void;
}) {
  const { notify, run } = useUi();
  const [query, setQuery] = useState("");
  const [items, setItems] = useState<ContentItem[] | null>(null);
  const [selected, setSelected] = useState<string[]>(preset?.items.map((item) => item.id) ?? []);
  const [targets, setTargets] = useState<DeliveryTarget[]>([]);
  const [groups, setGroups] = useState<DeliveryTargetGroup[]>([]);
  const [chosen, setChosen] = useState<string[]>([]);
  const [note, setNote] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const fixed = Boolean(preset);

  const load = useCallback(async () => {
    setError(null);
    try {
      const catalog = await listDeliveryTargets();
      setTargets(catalog.items);
      setGroups(catalog.groups);
      if (fixed) return;
      const contentPage = await listContent({ limit: 50, query: query.trim() || undefined, status: "active" });
      setItems(contentPage.items);
    } catch (problem) {
      setError(problem);
      if (!fixed) setItems([]);
    }
  }, [fixed, query]);

  useEffect(() => {
    const timer = window.setTimeout(() => void load(), 200);
    return () => window.clearTimeout(timer);
  }, [load]);

  const grouped = useMemo(
    () =>
      groups.map((group) => ({
        ...group,
        items: targets.filter((target) => target.kind === group.kind),
      })),
    [groups, targets],
  );

  const toggle = (list: string[], value: string) =>
    list.includes(value) ? list.filter((item) => item !== value) : [...list, value];

  async function submit() {
    if (!selected.length) {
      notify("warn", "先选择要投递的内容");
      return;
    }
    if (!chosen.length) {
      notify("warn", "先选择至少一个投递目标");
      return;
    }
    setBusy(true);
    let started = false;
    try {
      const operation = await createDeliveryJob({
        contentIds: selected,
        targets: chosen.map((value) => {
          const at = value.indexOf(":");
          return { kind: value.slice(0, at), id: value.slice(at + 1) };
        }),
        note: note.trim() || undefined,
      });
      started = true;
      await run(operation, "内容投递");
      onDone?.();
      onClose();
    } catch (problem) {
      if (!started) {
        notify("bad", "投递失败", problem instanceof Error ? problem.message : undefined);
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title="手动投递"
      sub={
        fixed
          ? "内容已选定，只需选择投递目标；模型会生成摘要，外部 API 按 JSON 提交，通知渠道直接推送"
          : "选择内容与目标；模型会生成摘要，外部 API 按 JSON 提交，通知渠道直接推送"
      }
      onClose={onClose}
      wide
      footer={
        <>
          <span className="small muted">
            已选 {selected.length} 条 · {chosen.length} 个目标
          </span>
          <span className="fill" />
          <button type="button" className="btn" onClick={onClose}>
            取消
          </button>
          <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void submit()}>
            {busy ? "提交中…" : "开始投递"}
          </button>
        </>
      }
    >
      <div className="stack" style={{ gap: 14 }}>
        {error ? <Problem error={error} onRetry={() => void load()} /> : null}
        {fixed ? (
          <div className="stack" style={{ gap: 8 }}>
            <div className="row between">
              <strong className="small">本次投递内容</strong>
              <span className="small muted">{preset?.items.length ?? 0} 条</span>
            </div>
            <div className="pick-list">
              {(preset?.items ?? []).map((item) => (
                <div key={item.id} className="pick-row is-selected">
                  <span className="pick-main">
                    <span className="pick-title">{item.title}</span>
                    <span className="pick-meta">
                      <span>{item.sourceName}</span>
                      <span>收录 {relative(item.createdAt)}</span>
                    </span>
                  </span>
                </div>
              ))}
            </div>
            <TargetPicker grouped={grouped} chosen={chosen} onToggle={(key) => setChosen((current) => toggle(current, key))} />
            <Field label="备注" hint="可选，写清用途方便以后核对">
              <input className="input" value={note} onChange={(event) => setNote(event.target.value)} placeholder="例如：给编辑组的每日摘要" />
            </Field>
          </div>
        ) : (
          <div className="grid cols-2" style={{ alignItems: "start", gap: 16 }}>
            <div className="stack" style={{ gap: 8 }}>
              <div className="row between">
                <strong className="small">内容</strong>
                <span className="small muted">{items?.length ?? 0} 条可选</span>
              </div>
              <input
                className="input"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder="按标题或正文检索"
                aria-label="检索内容"
              />
              {items === null ? (
                <Loading lines={4} />
              ) : (
                <div className="pick-list">
                  {items.length === 0 ? <div className="empty">没有匹配内容</div> : null}
                  {items.map((item) => (
                    <label key={item.id} className={`pick-row${selected.includes(item.id) ? " is-selected" : ""}`}>
                      <input
                        type="checkbox"
                        checked={selected.includes(item.id)}
                        onChange={() => setSelected((current) => toggle(current, item.id))}
                      />
                      <span className="pick-main">
                        <span className="pick-title">{item.title}</span>
                        <span className="pick-meta">
                          <span>{item.sourceName}</span>
                          <span>收录 {relative(item.createdAt)}</span>
                        </span>
                      </span>
                    </label>
                  ))}
                </div>
              )}
            </div>
            <div className="stack" style={{ gap: 8 }}>
              <TargetPicker grouped={grouped} chosen={chosen} onToggle={(key) => setChosen((current) => toggle(current, key))} />
              <Field label="备注" hint="可选，写清用途方便以后核对">
                <input className="input" value={note} onChange={(event) => setNote(event.target.value)} placeholder="例如：给编辑组的每日摘要" />
              </Field>
            </div>
          </div>
        )}
      </div>
    </Modal>
  );
}

/** 目标选择：按模型 / 外部 API / 通知渠道分组，停用的目标不可选。 */
function TargetPicker({
  grouped,
  chosen,
  onToggle,
}: {
  grouped: (DeliveryTargetGroup & { items: DeliveryTarget[] })[];
  chosen: string[];
  onToggle: (key: string) => void;
}) {
  return (
    <div className="stack" style={{ gap: 8 }}>
      <strong className="small">投递目标（可多选）</strong>
      {grouped.map((group) => (
        <div className="stack" key={group.kind} style={{ gap: 4 }}>
          <div className="small muted" title={group.note}>
            {group.label} · {group.note}
          </div>
          {group.items.length === 0 ? (
            <div className="small muted">暂无可用目标</div>
          ) : (
            group.items.map((target) => {
              const key = `${target.kind}:${target.id}`;
              const disabled = target.enabled === false;
              return (
                <label key={key} className={`pick-row${chosen.includes(key) ? " is-selected" : ""}${disabled ? " is-disabled" : ""}`}>
                  <input type="checkbox" checked={chosen.includes(key)} disabled={disabled} onChange={() => onToggle(key)} />
                  <PlatformIcon icon={target.kind} size={24} />
                  <span className="pick-main">
                    <span className="pick-title">{target.name ?? target.id}</span>
                    <span className="pick-meta">
                      <span>{target.detail ?? target.kind}</span>
                      {disabled ? <span>已停用</span> : null}
                    </span>
                  </span>
                </label>
              );
            })
          )}
        </div>
      ))}
    </div>
  );
}
