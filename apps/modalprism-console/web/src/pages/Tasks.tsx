// 执行队列：一条任务从排队、租约到终态的生命周期。
//
// 统计与分页都在服务端：页面只持有一页数据，不再拉全量再在浏览器里数。
import { useCallback, useEffect, useMemo, useState } from "react";

import { BatchBar } from "../components/batch";
import { Drawer } from "../components/Drawer";
import { IconAlert, IconPause, IconPlay, IconRefresh, IconTrash } from "../components/icons";
import {
  Card,
  Definitions,
  Empty,
  FilterSelect,
  Pagination,
  Pill,
  SearchInput,
  Seg,
  Skeleton,
} from "../components/ui";
import {
  taskCancel,
  taskList,
  taskPurge,
  taskRemove,
  taskRequeue,
  taskRun,
  taskStats,
  taskTransition,
  type TaskQuery,
} from "../lib/api";
import { useConsole } from "../lib/console";
import { count, relative, stamp } from "../lib/format";
import { useRoute } from "../lib/router";
import { useSelection } from "../lib/selection";
import type { TaskPage, TaskRow, TaskStats, TaskStatus } from "../lib/types";

/** 每页条数；游标翻页，不提供跳页。 */
const PAGE_SIZE = 50;

const STATUS_LABELS: Record<string, string> = {
  queued: "排队中",
  leased: "执行中",
  done: "已完成",
  cancelled: "已取消",
};

const REASONS: Record<string, string> = {
  schedule: "调度",
  follow: "跟进",
  detail: "详情",
  render_upgrade: "渲染升级",
  replay: "回放",
  manual: "手动",
};

function statusTone(status: TaskStatus): "ok" | "info" | "idle" | "warn" {
  if (status === "leased") {
    return "ok";
  }
  if (status === "queued") {
    return "info";
  }
  return status === "cancelled" ? "warn" : "idle";
}

function locatorOf(row: TaskRow): string {
  const url = row.locator?.url;
  return typeof url === "string" ? url : "—";
}

function reasonLabel(reason: string): string {
  return REASONS[reason] ?? reason;
}

export function TasksPage() {
  const { sources, notify, now } = useConsole();
  const { param } = useRoute();
  const [page, setPage] = useState<TaskPage | null>(null);
  const [stats, setStats] = useState<TaskStats | null>(null);
  const [status, setStatus] = useState<"all" | TaskStatus>("all");
  const [queue, setQueue] = useState("");
  const [source, setSource] = useState(param ?? "");
  const [reason, setReason] = useState("");
  const [text, setText] = useState("");
  const [needle, setNeedle] = useState("");
  const [stack, setStack] = useState<(string | null)[]>([null]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [detail, setDetail] = useState<string | null>(null);
  const selection = useSelection<string>();
  const cursor = stack[stack.length - 1] ?? null;

  // 深链 #/tasks/<sourceId>：进入页面时预置来源筛选。
  useEffect(() => {
    if (param !== null) {
      setSource(param);
      setStack([null]);
    }
  }, [param]);

  // 关键字防抖，与内容库一致。
  useEffect(() => {
    const timer = window.setTimeout(() => {
      setNeedle(text.trim());
      setStack([null]);
    }, 320);
    return () => window.clearTimeout(timer);
  }, [text]);

  const filters = useMemo<TaskQuery>(
    () => ({
      status: status === "all" ? undefined : status,
      queue: queue || undefined,
      sourceId: source || undefined,
      reason: reason || undefined,
      query: needle || undefined,
    }),
    [status, queue, source, reason, needle],
  );

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [next, summary] = await Promise.all([
        taskList({ ...filters, cursor, limit: PAGE_SIZE }),
        taskStats(),
      ]);
      setPage(next);
      setStats(summary);
      setError(null);
    } catch (problem) {
      setError(problem instanceof Error ? problem.message : "读取任务队列失败");
    } finally {
      setLoading(false);
    }
  }, [filters, cursor]);

  useEffect(() => {
    void load();
  }, [load]);

  const rows = page?.rows ?? [];
  const picked = selection.list();
  const byStatus = stats?.byStatus ?? {};
  const queueOptions = useMemo(
    () => [
      { value: "", label: "全部队列" },
      ...Object.keys(stats?.byQueue ?? {})
        .sort()
        .map((name) => ({ value: name, label: name })),
    ],
    [stats],
  );
  const sourceOptions = useMemo(
    () => [
      { value: "", label: "全部信源" },
      ...sources.map((item) => ({ value: item.id, label: item.name })),
    ],
    [sources],
  );
  const reasonOptions = useMemo(
    () => [
      { value: "", label: "全部原因" },
      ...Object.keys(stats?.byReason ?? {})
        .sort()
        .map((name) => ({ value: name, label: reasonLabel(name) })),
    ],
    [stats],
  );
  const detailRow = rows.find((row) => row.task_id === detail) ?? null;

  /** 一次操作后统一刷新：结果与统计一起重新读，避免页面数值滞后。 */
  async function after(action: () => Promise<string | null>) {
    setBusy(true);
    try {
      const message = await action();
      if (message) {
        notify("ok", message);
      }
      selection.clear();
      setDetail(null);
      await load();
    } catch (problem) {
      notify("bad", "操作没有完成", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  function act(row: TaskRow, kind: "cancel" | "requeue" | "run" | "remove") {
    void after(async () => {
      if (kind === "cancel") {
        return (await taskCancel(row.task_id)) ? "任务已取消" : "这条任务已不在排队或执行中";
      }
      if (kind === "requeue") {
        return (await taskRequeue(row.task_id)) ? "任务已回到队列" : "这条任务没能回到队列";
      }
      if (kind === "run") {
        await taskRun(row.task_id);
        return "任务已执行，结果会写入内容库";
      }
      return (await taskRemove(row.task_id)) ? "已删除这条终态任务" : "只有终态任务可以删除";
    });
  }

  function batchAct(kind: "cancel" | "requeue" | "remove") {
    const targets = rows.filter((row) => picked.includes(row.task_id));
    if (targets.length === 0) {
      return;
    }
    if (kind === "remove" && !window.confirm(`删除选中的 ${targets.length} 条终态任务？非终态任务会被跳过。`)) {
      return;
    }
    void after(async () => {
      const report = await taskTransition(
        targets.map((row) => row.task_id),
        kind,
      );
      const verb = kind === "cancel" ? "取消" : kind === "requeue" ? "重试" : "删除";
      return `${verb}完成 ${report.done} 条${report.skipped > 0 ? `，跳过 ${report.skipped} 条` : ""}`;
    });
  }

  async function purge() {
    setBusy(true);
    try {
      const preview = await taskPurge({ ...filters, dryRun: true });
      if (preview.matched === 0) {
        notify("info", "当前筛选下没有可清理的终态任务");
        return;
      }
      const scope = filters.status && filters.status !== "cancelled" ? `状态为「${STATUS_LABELS[filters.status]}」的` : "";
      if (!window.confirm(`将清理${scope} ${preview.matched} 条终态任务（已完成或已取消），此操作不可撤销。继续？`)) {
        return;
      }
      const report = await taskPurge({ ...filters, dryRun: false });
      notify("ok", `已清理 ${report.removed} 条终态任务`);
      setStack([null]);
      await load();
    } catch (problem) {
      notify("bad", "清理失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <span className="page-kicker">tasks · 执行队列</span>
          <h1 className="page-title">任务生命周期</h1>
          <p className="page-lede">
            调度派发的每一条任务在这里排队、挂租约、执行到终态。可以取消、重试或恢复、立即执行，终态任务能单独删除，也能按筛选一次清理。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn" onClick={() => void load()} disabled={loading}>
            <IconRefresh size={13} /> 刷新
          </button>
          <button type="button" className="btn btn-danger" onClick={() => void purge()} disabled={busy}>
            <IconTrash size={13} /> 清理终态
          </button>
        </div>
      </header>

      {error ? (
        <div className="note is-bad">
          <strong>任务队列读取失败</strong>
          <span>{error}</span>
        </div>
      ) : null}

      <div className="stat-grid">
        <div className="stat">
          <span className="stat-label">排队中</span>
          <span className="stat-value">{stats === null ? "—" : count(byStatus.queued ?? 0)}</span>
          <span className="stat-foot">
            {stats?.oldestQueuedAt ? `最早一条 ${relative(stats.oldestQueuedAt, now)}` : "队列是空的"}
          </span>
        </div>
        <div className="stat">
          <span className="stat-label">执行中</span>
          <span className="stat-value">{stats === null ? "—" : count(byStatus.leased ?? 0)}</span>
          <span className="stat-foot">
            {stats && stats.leasedOverdue > 0 ? `${stats.leasedOverdue} 条租约已过期` : "没有过期租约"}
          </span>
        </div>
        <div className="stat">
          <span className="stat-label">已完成</span>
          <span className="stat-value">{stats === null ? "—" : count(byStatus.done ?? 0)}</span>
          <span className="stat-foot">终态可清理</span>
        </div>
        <div className="stat">
          <span className="stat-label">已取消</span>
          <span className="stat-value">{stats === null ? "—" : count(byStatus.cancelled ?? 0)}</span>
          <span className="stat-foot">
            {stats?.nextRetryAt ? `${relative(stats.nextRetryAt, now)}后有一条重试` : "没有等待重试的任务"}
          </span>
        </div>
      </div>

      <div className="toolbar">
        <SearchInput value={text} onChange={setText} placeholder="按任务号、信源或错误筛选…" label="筛选任务" />
        <FilterSelect label="队列" value={queue} onChange={(value) => {
          setQueue(value);
          setStack([null]);
        }} options={queueOptions} />
        <FilterSelect label="信源" value={source} onChange={(value) => {
          setSource(value);
          setStack([null]);
        }} options={sourceOptions} />
        <FilterSelect label="原因" value={reason} onChange={(value) => {
          setReason(value);
          setStack([null]);
        }} options={reasonOptions} />
        <span className="spacer" />
        <span className="faint" style={{ fontSize: "var(--fs-xs)" }}>
          {page === null ? "读取中" : `共 ${count(page.total)} 条`}
        </span>
      </div>

      <BatchBar
        count={selection.size}
        busy={busy}
        onClear={selection.clear}
        actions={[
          { key: "cancel", label: "取消", icon: <IconPause size={12} />, onClick: () => batchAct("cancel") },
          { key: "requeue", label: "重试", icon: <IconPlay size={12} />, onClick: () => batchAct("requeue") },
          {
            key: "remove",
            label: "删除",
            icon: <IconTrash size={12} />,
            danger: true,
            onClick: () => batchAct("remove"),
          },
        ]}
      />

      <Card
        title="任务列表"
        sub="按创建时间倒序，游标翻页"
        action={
          <Seg
            value={status}
            onChange={(value) => {
              setStatus(value);
              setStack([null]);
            }}
            label="任务状态"
            options={[
              { value: "all", label: "全部" },
              { value: "queued", label: "排队" },
              { value: "leased", label: "执行中" },
              { value: "done", label: "已完成" },
              { value: "cancelled", label: "已取消" },
            ]}
          />
        }
        tight
      >
        {page === null ? (
          <Skeleton lines={6} />
        ) : rows.length === 0 ? (
          <Empty
            title="没有符合筛选的任务"
            text={page.total === 0 ? "调度器派出任务后，这里会出现记录。" : "换个状态或清空筛选再看。"}
          />
        ) : (
          <div className="table-scroll">
            <table className="table">
              <thead>
                <tr>
                  <th className="pick-col">
                    <input
                      type="checkbox"
                      aria-label="全选本页"
                      checked={rows.length > 0 && selection.size === rows.length}
                      onChange={(event) => {
                        if (event.target.checked) {
                          rows.forEach((row) => {
                            if (!selection.has(row.task_id)) {
                              selection.toggle(row.task_id);
                            }
                          });
                        } else {
                          selection.clear();
                        }
                      }}
                    />
                  </th>
                  <th>任务</th>
                  <th>队列</th>
                  <th>信源</th>
                  <th>原因</th>
                  <th>状态</th>
                  <th className="align-right">尝试</th>
                  <th>创建</th>
                  <th className="align-right">操作</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((row) => (
                  <tr key={row.task_id} className="row-button" onClick={() => setDetail(row.task_id)}>
                    <td className="pick-col" onClick={(event) => event.stopPropagation()}>
                      <input
                        type="checkbox"
                        aria-label={`选择任务 ${row.task_id}`}
                        checked={selection.has(row.task_id)}
                        onChange={() => selection.toggle(row.task_id)}
                      />
                    </td>
                    <td>
                      <span className="mono" style={{ fontSize: "var(--fs-2xs)" }}>
                        {row.task_id.slice(0, 10)}
                      </span>
                      <div className="cell-meta" style={{ maxWidth: 260, overflowWrap: "anywhere" }}>
                        {locatorOf(row)}
                      </div>
                    </td>
                    <td className="mono" style={{ fontSize: "var(--fs-xs)" }}>
                      {row.queue}
                    </td>
                    <td className="mono" style={{ fontSize: "var(--fs-xs)" }}>
                      {row.source_id}
                    </td>
                    <td className="muted" style={{ fontSize: "var(--fs-xs)" }}>
                      {reasonLabel(row.reason)}
                    </td>
                    <td>
                      <Pill tone={statusTone(row.status)} dot>
                        {STATUS_LABELS[row.status] ?? row.status}
                      </Pill>
                      {row.status === "queued" && row.retry_at ? (
                        <div className="cell-meta">{relative(row.retry_at, now)}可租约</div>
                      ) : null}
                      {row.last_error && row.status !== "cancelled" ? (
                        <div className="cell-meta" style={{ maxWidth: 200, overflowWrap: "anywhere" }}>
                          {row.last_error}
                        </div>
                      ) : null}
                    </td>
                    <td className="num align-right">{row.attempt}</td>
                    <td className="muted">{relative(row.created_at, now)}</td>
                    <td className="align-right" onClick={(event) => event.stopPropagation()}>
                      <span className="row-actions">
                        {row.status === "queued" || row.status === "leased" ? (
                          <button type="button" className="btn btn-sm btn-quiet" disabled={busy} onClick={() => act(row, "cancel")}>
                            取消
                          </button>
                        ) : null}
                        {row.status === "queued" || row.status === "cancelled" ? (
                          <button type="button" className="btn btn-sm btn-quiet" disabled={busy} onClick={() => act(row, "requeue")}>
                            {row.status === "queued" ? "立即重试" : "恢复"}
                          </button>
                        ) : null}
                        {row.status === "queued" ? (
                          <button type="button" className="btn btn-sm btn-quiet" disabled={busy} onClick={() => act(row, "run")}>
                            执行
                          </button>
                        ) : null}
                        {row.status === "done" || row.status === "cancelled" ? (
                          <button type="button" className="btn btn-sm btn-quiet" disabled={busy} onClick={() => act(row, "remove")}>
                            删除
                          </button>
                        ) : null}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Card>

      {page !== null && (stack.length > 1 || page.nextCursor !== null) ? (
        <Pagination
          page={stack.length}
          hasPrev={stack.length > 1}
          hasNext={page.nextCursor !== null}
          onChange={(next) => {
            if (next > stack.length && page.nextCursor) {
              setStack((current) => [...current, page.nextCursor]);
            } else if (next < stack.length && stack.length > 1) {
              setStack((current) => current.slice(0, -1));
            }
          }}
          onFirst={() => setStack([null])}
        />
      ) : null}

      <TaskDrawer
        row={detailRow}
        busy={busy}
        onClose={() => setDetail(null)}
        onAct={act}
      />

      <p className="faint" style={{ fontSize: "var(--fs-xs)", lineHeight: 1.7 }}>
        <IconAlert size={11} /> 同一信源指向同一地址的活跃任务只有一个；取消后腾出的去重位可以被新的任务复用。清理只作用于已完成与已取消的任务。
      </p>
    </div>
  );
}

function TaskDrawer({
  row,
  busy,
  onClose,
  onAct,
}: {
  row: TaskRow | null;
  busy: boolean;
  onClose: () => void;
  onAct: (row: TaskRow, kind: "cancel" | "requeue" | "run" | "remove") => void;
}) {
  if (!row) {
    return null;
  }
  return (
    <Drawer
      key={row.task_id}
      open
      kicker={`任务 · ${row.queue}`}
      title={row.task_id}
      onClose={onClose}
      footer={
        <>
          {row.status === "queued" || row.status === "leased" ? (
            <button type="button" className="btn" disabled={busy} onClick={() => onAct(row, "cancel")}>
              <IconPause size={12} /> 取消
            </button>
          ) : null}
          {row.status === "queued" || row.status === "cancelled" ? (
            <button type="button" className="btn btn-primary" disabled={busy} onClick={() => onAct(row, "requeue")}>
              <IconPlay size={12} /> {row.status === "queued" ? "立即重试" : "恢复任务"}
            </button>
          ) : null}
          {row.status === "queued" ? (
            <button type="button" className="btn" disabled={busy} onClick={() => onAct(row, "run")}>
              立即执行
            </button>
          ) : null}
          {row.status === "done" || row.status === "cancelled" ? (
            <button type="button" className="btn btn-danger" disabled={busy} onClick={() => onAct(row, "remove")}>
              <IconTrash size={12} /> 删除
            </button>
          ) : null}
        </>
      }
    >
      <div className="row">
        <Pill tone={statusTone(row.status)} dot>
          {STATUS_LABELS[row.status] ?? row.status}
        </Pill>
        <Pill tone="info">第 {row.attempt} 次尝试</Pill>
        <Pill>优先级 {row.priority}</Pill>
      </div>

      <Definitions
        items={[
          ["信源", <span className="mono">{row.source_id}</span>],
          ["通道", row.accessor ? <span className="mono">{row.accessor}</span> : "—"],
          ["原因", reasonLabel(row.reason)],
          ["配置代际", `G-${row.generation}`],
          ["目标地址", <span className="mono" style={{ fontSize: "var(--fs-xs)" }}>{locatorOf(row)}</span>],
          ["创建时间", `${stamp(row.created_at)} · ${relative(row.created_at)}`],
          ["租约到期", row.lease_until ? `${stamp(row.lease_until)} · ${relative(row.lease_until)}` : "没有租约"],
          ["下次重试", row.retry_at ? `${stamp(row.retry_at)} · ${relative(row.retry_at)}` : "—"],
        ]}
      />

      {row.last_error && row.status !== "cancelled" ? (
        <div className="note is-warn">
          <div style={{ display: "grid", gap: 4 }}>
            <strong>上次没有顺利完成</strong>
            <span className="mono" style={{ fontSize: "var(--fs-2xs)", overflowWrap: "anywhere" }}>
              {row.last_error}
            </span>
          </div>
        </div>
      ) : null}

      <p className="faint" style={{ fontSize: "var(--fs-2xs)", lineHeight: 1.7 }}>
        任务体里可能带凭据，这里只展示通道与目标地址，不展示完整载荷。
      </p>
    </Drawer>
  );
}
