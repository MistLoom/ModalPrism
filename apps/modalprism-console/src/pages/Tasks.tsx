import { useCallback, useEffect, useMemo, useState } from "react";

import { IconAlert, IconPause, IconPlay, IconRefresh, IconTrash } from "../components/icons";
import { Card, Drawer, Empty, Field, Loading, Modal, Problem, StatusBadge, useUi } from "../components/ui";
import { batchTasks, listQueues, listTasks, purgeTasks } from "../lib/api";
import { count, kindLabel, relative, stamp, statusLabel } from "../lib/format";
import { ProjectScopeBadge } from "../lib/project";
import { navigate, useRoute } from "../lib/router";
import type { Queue, Task } from "../lib/types";

export function TasksPage() {
  const { params } = useRoute();
  const { notify, run } = useUi();
  const [tasks, setTasks] = useState<Task[] | null>(null);
  const [next, setNext] = useState<string | null>(null);
  const [queues, setQueues] = useState<Queue[]>([]);
  const [status, setStatus] = useState(params.get("status") ?? "all");
  const [kind, setKind] = useState(params.get("kind") ?? "all");
  const [search, setSearch] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);
  const [selection, setSelection] = useState<string[]>([]);
  const [detail, setDetail] = useState<Task | null>(null);
  const [busy, setBusy] = useState(false);
  const [purgeOpen, setPurgeOpen] = useState(false);

  const load = useCallback(
    async (cursor?: string) => {
      setLoading(true);
      try {
        const [page, queuePage] = await Promise.all([
          listTasks({
            limit: 100,
            cursor,
            status: status === "all" ? undefined : status,
            kind: kind === "all" ? undefined : kind,
          }),
          listQueues(),
        ]);
        setTasks((current) => (cursor ? [...(current ?? []), ...page.items] : page.items));
        setNext(page.nextCursor);
        setQueues(queuePage.items as unknown as Queue[]);
        setError(null);
      } catch (problem) {
        setError(problem);
      } finally {
        setLoading(false);
      }
    },
    [status, kind],
  );

  useEffect(() => {
    setSelection([]);
    void load();
  }, [load]);

  const rows = useMemo(() => {
    const needle = search.trim().toLowerCase();
    if (!needle) return tasks ?? [];
    return (tasks ?? []).filter((task) =>
      `${task.id} ${task.kind} ${task.title ?? ""} ${task.sourceId ?? ""} ${task.error ?? ""}`.toLowerCase().includes(needle),
    );
  }, [tasks, search]);

  async function action(label: string, ids: string[], decision: string) {
    if (!ids.length) return;
    setBusy(true);
    let started = false;
    try {
      const operation = await batchTasks(ids, decision);
      started = true;
      await run(operation, label);
      setSelection([]);
      await load();
    } catch (problem) {
      if (!started) {
        notify("bad", `${label}失败`, problem instanceof Error ? problem.message : undefined);
      }
    } finally {
      setBusy(false);
    }
  }

  const selectedTasks = rows.filter((task) => selection.includes(task.id));
  const active = selectedTasks.filter((task) => ["queued", "running"].includes(task.status)).length;
  const retryable = selectedTasks.filter((task) => ["failed", "cancelled"].includes(task.status)).length;

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <div className="kicker">tasks · 任务</div>
          <h1 className="page-title">跨采集、生产、发布与维护的执行记录</h1>
          <p className="page-lede">
            任务展示业务状态，内部租约与句柄不进入界面。批量动作逐项执行并保留成功项；活动租约不会被直接删除。
          </p>
        </div>
        <div className="page-actions">
          <ProjectScopeBadge />
          <button type="button" className="btn" onClick={() => void load()} disabled={loading}>
            <IconRefresh size={14} /> 刷新
          </button>
          <button type="button" className="btn btn-danger" onClick={() => setPurgeOpen(true)}>
            <IconTrash size={14} /> 清空任务
          </button>
        </div>
      </header>

      {error ? <Problem error={error} onRetry={() => void load()} /> : null}

      <div className="grid cols-4">
        {queues.map((queue) => (
          <div className={`queue-card${queue.paused ? " is-paused" : ""}`} key={queue.id}>
            <div className="queue-head">
              <span className="queue-name mono">{queue.id}</span>
              <span className={`badge ${queue.paused ? "warn" : "ok"}`}>{queue.paused ? "暂停" : "派发中"}</span>
              <span className="fill" />
              <span className="small muted">{queue.running}/{queue.capacity}</span>
            </div>
            <div className="queue-nums">
              <span>排队 <strong>{count(queue.queued)}</strong></span>
              <span>执行 <strong>{count(queue.running)}</strong></span>
              <span>完成 <strong>{count(queue.done ?? 0)}</strong></span>
              <span>取消 <strong>{count(queue.cancelled ?? 0)}</strong></span>
            </div>
          </div>
        ))}
      </div>

      <div className="toolbar-band">
        <label className="search">
          <span className="muted" aria-hidden="true">⌕</span>
          <input value={search} onChange={(event) => setSearch(event.target.value)} placeholder="搜索任务号、来源或错误" aria-label="搜索任务" />
        </label>
        <div className="seg" role="group" aria-label="任务状态">
          {[
            ["all", "全部"],
            ["queued", "排队"],
            ["running", "执行中"],
            ["succeeded", "成功"],
            ["failed", "失败"],
            ["cancelled", "已取消"],
          ].map(([id, label]) => (
            <button key={id} type="button" className={status === id ? "is-active" : ""} onClick={() => setStatus(id)}>
              {label}
            </button>
          ))}
        </div>
        <select className="select" value={kind} onChange={(event) => setKind(event.target.value)} aria-label="任务类型">
          <option value="all">全部类型</option>
          <option value="collection">采集</option>
          <option value="production">生产</option>
          <option value="publication">发布</option>
        </select>
        <span className="fill" />
        <span className="small muted">{rows.length} 条</span>
      </div>

      {selection.length > 0 ? (
        <div className="batchbar">
          <span>已选 {selection.length} 条</span>
          <button type="button" className="btn btn-sm" disabled={busy || active === 0} onClick={() => void action("批量取消", selection.filter((id) => rows.find((row) => row.id === id && ["queued", "running"].includes(row.status))), "cancel")}>
            <IconPause size={12} /> 取消
          </button>
          <button type="button" className="btn btn-sm" disabled={busy || retryable === 0} onClick={() => void action("批量重排", selection.filter((id) => rows.find((row) => row.id === id && ["failed", "cancelled"].includes(row.status))), "requeue")}>
            <IconPlay size={12} /> 重排
          </button>
          <button type="button" className="btn btn-sm btn-danger" disabled={busy} onClick={() => void action("删除选中任务", selection, "delete")}>
            <IconTrash size={12} /> 删除
          </button>
          <button type="button" className="btn btn-sm" onClick={() => setSelection([])}>清除选择</button>
        </div>
      ) : null}

      <Card title="任务列表" sub="新任务在前；跨类型统一投影" flush>
        {loading && !tasks ? (
          <div className="card-body">
            <Loading lines={6} />
          </div>
        ) : rows.length === 0 ? (
          <Empty title="没有匹配的任务" text="调整筛选，或在信源页发起一次手动采集。" />
        ) : (
          <div className="table-wrap">
            <table className="table">
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
                  <th>任务</th>
                  <th>类型</th>
                  <th>状态</th>
                  <th>队列</th>
                  <th>创建</th>
                  <th>耗时/错误</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {rows.map((task) => (
                  <tr key={task.id} className={selection.includes(task.id) ? "is-selected" : ""}>
                    <td>
                      <input
                        type="checkbox"
                        aria-label={`选择 ${task.id}`}
                        checked={selection.includes(task.id)}
                        onChange={(event) =>
                          setSelection((current) => (event.target.checked ? [...current, task.id] : current.filter((id) => id !== task.id)))
                        }
                      />
                    </td>
                    <td>
                      <div className="cell-main">
                        <span className="source-name">{task.title || task.sourceId || task.id}</span>
                        <span className="source-sub mono">{task.id}</span>
                      </div>
                    </td>
                    <td><span className="tag">{kindLabel(task.kind)}</span></td>
                    <td>
                      <div className="cell-main">
                        <StatusBadge status={task.status} />
                        {task.attempt && task.attempt > 1 ? <span className="source-sub">第 {task.attempt} 次</span> : null}
                      </div>
                    </td>
                    <td className="mono small">{task.queue || "—"}</td>
                    <td className="small muted">{relative(task.createdAt)}</td>
                    <td className="small">
                      {task.error ? (
                        <span style={{ color: "var(--danger)" }} title={task.error}>{task.error.slice(0, 42)}{task.error.length > 42 ? "…" : ""}</span>
                      ) : (
                        <span className="muted">{task.retryAt ? `重试 ${relative(task.retryAt)}` : "—"}</span>
                      )}
                    </td>
                    <td className="actions">
                      <button type="button" className="btn btn-sm" onClick={() => setDetail(task)}>详情</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        {next ? (
          <div className="pager">
            <span>还有更多任务</span>
            <button type="button" className="btn btn-sm" onClick={() => void load(next)} disabled={loading}>加载更多</button>
          </div>
        ) : null}
      </Card>

      {detail ? <TaskDrawer task={detail} onClose={() => setDetail(null)} /> : null}
      {purgeOpen ? (
        <TaskPurgeModal
          status={status}
          kind={kind}
          onClose={() => setPurgeOpen(false)}
          onDone={() => {
            setPurgeOpen(false);
            void load();
          }}
        />
      ) : null}
    </div>
  );
}

function TaskPurgeModal({ status, kind, onClose, onDone }: { status: string; kind: string; onClose: () => void; onDone: () => void }) {
  const { notify } = useUi();
  const [scope, setScope] = useState<"filtered" | "all">("filtered");
  const [preview, setPreview] = useState<number | null>(null);
  const [deleted, setDeleted] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);

  const filter = () => ({
    status: scope === "filtered" && status !== "all" ? status : undefined,
    kind: scope === "filtered" && kind !== "all" ? kind : undefined,
  });

  useEffect(() => {
    let alive = true;
    setPreview(null);
    void (async () => {
      setBusy(true);
      try {
        const report = await purgeTasks({ ...filter(), dryRun: true });
        if (alive) setPreview(report.matched);
      } catch (problem) {
        if (alive) notify("bad", "预演失败", problem instanceof Error ? problem.message : undefined);
      } finally {
        if (alive) setBusy(false);
      }
    })();
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scope, status, kind]);

  async function purge() {
    if (!window.confirm(`永久删除 ${preview ?? 0} 条任务？排队与执行中的任务会先被取消。`)) return;
    setBusy(true);
    try {
      const report = await purgeTasks({ ...filter(), dryRun: false });
      setDeleted(report.deleted);
      notify("ok", "任务已清空", `共永久删除 ${report.deleted} 条`);
      onDone();
    } catch (problem) {
      notify("bad", "清空任务失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title="清空任务"
      sub="活动任务先取消再删除；控制台与队列记录一并移除"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>取消</button>
          <button type="button" className="btn btn-danger" onClick={() => void purge()} disabled={busy || !preview}>
            {busy ? "处理中…" : `永久删除 ${preview ?? 0} 条`}
          </button>
        </>
      }
    >
      <div className="stack" style={{ gap: 12 }}>
        <Field label="范围">
          <select className="select" value={scope} onChange={(event) => setScope(event.target.value as "filtered" | "all")}>
            <option value="filtered">当前筛选（状态 {status === "all" ? "全部" : statusLabel(status)} · 类型 {kind === "all" ? "全部" : kindLabel(kind)}）</option>
            <option value="all">全部任务</option>
          </select>
        </Field>
        <div className="alert warn">
          预演命中 <strong className="mono">{preview ?? "…"}</strong> 条。删除后不可恢复；失败与取消的任务同样会被移除。
        </div>
        {deleted !== null ? <div className="small muted">已删除 {deleted} 条</div> : null}
      </div>
    </Modal>
  );
}

function TaskDrawer({ task, onClose }: { task: Task; onClose: () => void }) {
  const correlation = task.runId || task.publicationId || task.id;
  return (
    <Drawer
      title={task.title || task.id}
      sub={`${kindLabel(task.kind)} · ${statusLabel(task.status)}`}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={() => navigate(`logs?correlationId=${encodeURIComponent(correlation)}`)}>
            在日志页追踪
          </button>
          <button type="button" className="btn" onClick={onClose}>关闭</button>
        </>
      }
    >
      <div className="stack" style={{ gap: 14 }}>
        <div className="detail-hero">
          <span className="detail-title">{task.title || "未命名任务"}</span>
          <div className="detail-meta">
            <StatusBadge status={task.status} />
            <span className="mono">{task.id}</span>
            <span>{task.kind}</span>
            {task.queue ? <span>队列 {task.queue}</span> : null}
          </div>
        </div>
        <dl className="kv">
          <dt>来源</dt><dd className="mono">{task.sourceId || "—"}</dd>
          <dt>原因</dt><dd>{task.reason || "—"}</dd>
          <dt>创建</dt><dd>{stamp(task.createdAt)}</dd>
          <dt>更新</dt><dd>{stamp(task.updatedAt)}</dd>
          <dt>重试时间</dt><dd>{stamp(task.retryAt)}</dd>
          <dt>生产运行</dt><dd className="mono">{task.runId || "—"}</dd>
          <dt>发布</dt><dd className="mono">{task.publicationId || "—"}</dd>
          <dt>错误</dt><dd style={{ color: task.error ? "var(--danger)" : undefined }}>{task.error || "—"}</dd>
        </dl>
        {task.locator ? (
          <div className="drawer-section">
            <h3>定位</h3>
            <div className="code">{JSON.stringify(task.locator, null, 2)}</div>
          </div>
        ) : null}
        {task.error ? (
          <div className="alert bad">
            <IconAlert size={15} />
            <span>{task.error}</span>
          </div>
        ) : null}
      </div>
    </Drawer>
  );
}
