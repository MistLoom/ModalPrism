import { useCallback, useEffect, useMemo, useState } from "react";

import { IconAlert, IconPause, IconPlay, IconRefresh, IconTrash } from "../components/icons";
import { Pager, slicePage } from "../components/Pager";
import { Card, Drawer, Empty, Field, Loading, Modal, Problem, StatusBadge, StructuredValue, useUi } from "../components/ui";
import { batchTasks, listPlans, listQueues, listSources, listSourceTypes, listTasks, pauseQueue, purgeTasks, resumeQueue, runDerivations, runTask } from "../lib/api";
import { t } from "../lib/dictionary";
import { count, kindLabel, relative, stamp, statusLabel } from "../lib/format";
import { ProjectScopeBadge } from "../lib/project";
import { navigate, useRoute } from "../lib/router";
import { subjectFromLocation } from "../lib/subject";
import type { Plan, Queue, SourceTypeRecord, Task } from "../lib/types";

// 任务扫描窗口：与后端队列扫描口径一致，超出部分提示「N+」。
const TASK_WINDOW = 200;

export function TasksPage() {
  const { params } = useRoute();
  const { notify, run } = useUi();
  const [tasks, setTasks] = useState<Task[] | null>(null);
  const [capped, setCapped] = useState(false);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(50);
  const [queues, setQueues] = useState<Queue[]>([]);
  const [plans, setPlans] = useState<Plan[]>([]);
  const [planFilter, setPlanFilter] = useState("all");
  const [status, setStatus] = useState(params.get("status") ?? "all");
  const [kind, setKind] = useState(params.get("kind") ?? "all");
  const [typeFilter, setTypeFilter] = useState("all");
  const [typeRecords, setTypeRecords] = useState<SourceTypeRecord[]>([]);
  const [sourceTypes, setSourceTypes] = useState<Map<string, string>>(new Map());
  const [search, setSearch] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);
  const [selection, setSelection] = useState<string[]>([]);
  const [detail, setDetail] = useState<Task | null>(null);
  const [busy, setBusy] = useState(false);
  const [purgeOpen, setPurgeOpen] = useState(false);

  const load = useCallback(
    async () => {
      setLoading(true);
      try {
        const [pageResult, queuePage, planPage, sourcePage, typePage] = await Promise.all([
          listTasks({
            limit: TASK_WINDOW,
            status: status === "all" ? undefined : status,
            kind: kind === "all" ? undefined : kind,
          }),
          listQueues(),
          listPlans({ limit: 200 }),
          listSources({ limit: 200 }),
          listSourceTypes(),
        ]);
        setTasks(pageResult.items);
        setCapped((pageResult.total ?? pageResult.items.length) >= TASK_WINDOW || pageResult.items.length >= TASK_WINDOW);
        setQueues(queuePage.items as unknown as Queue[]);
        setPlans(planPage.items);
        // 信源类型经 sourceId 派生（不落库）：构建 id → 类型映射供筛选与徽标。
        setSourceTypes(new Map(sourcePage.items.map((source) => [source.id, source.sourceType])));
        setTypeRecords(typePage.items);
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

  // 深链：从概览或搜索结果跳过来时，直接打开对应任务的详情。
  useEffect(() => {
    if (!tasks) return;
    const id = subjectFromLocation("task");
    if (!id) return;
    const hit = tasks.find((task) => task.id === id);
    if (hit) setDetail(hit);
  }, [tasks]);

  // 打开任务详情：同时在地址栏留下 `?id=`，刷新或分享后能回到同一任务。
  const openTask = (task: Task) => {
    setDetail(task);
    navigate(`tasks?id=${encodeURIComponent(task.id)}`);
  };

  // 关闭详情时清掉深链参数，避免刷新后又被重新打开。
  const closeTask = () => {
    setDetail(null);
    if (subjectFromLocation("task")) navigate("tasks");
  };

  const planBySource = useMemo(() => {
    const map = new Map<string, Plan>();
    for (const plan of plans) {
      for (const target of plan.targets ?? []) {
        map.set(target.replace(/^source\//, ""), plan);
      }
      if (!map.has(plan.id)) map.set(plan.id, plan);
    }
    return map;
  }, [plans]);

  const planOfTask = (task: Task): Plan | undefined => {
    if (task.planId) {
      const key = task.planId.replace(/^plan\//, "");
      const byId = plans.find((plan) => plan.id === key || plan.id === task.planId);
      if (byId) return byId;
    }
    const source = (task.sourceId ?? "").replace(/^source\//, "");
    return source ? planBySource.get(source) : undefined;
  };

  const typeOfTask = (task: Task): string => {
    const id = (task.sourceId ?? "").replace(/^source\//, "");
    return sourceTypes.get(id) ?? "";
  };
  const typeLabelOf = (typeId: string): string => typeRecords.find((item) => item.id === typeId)?.label ?? typeId;

  const rows = useMemo(() => {
    const needle = search.trim().toLowerCase();
    let list = tasks ?? [];
    if (typeFilter !== "all") {
      list = list.filter((task) => {
        const id = (task.sourceId ?? "").replace(/^source\//, "");
        return sourceTypes.get(id) === typeFilter;
      });
    }
    if (planFilter !== "all") {
      list = list.filter((task) => planOfTask(task)?.id === planFilter);
    }
    if (!needle) return list;
    return list.filter((task) =>
      `${task.id} ${task.kind} ${task.title ?? ""} ${task.sourceId ?? ""} ${task.error ?? ""}`.toLowerCase().includes(needle),
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tasks, search, planFilter, planBySource, typeFilter, sourceTypes]);

  const pagedRows = useMemo(() => slicePage(rows, page, pageSize), [rows, page, pageSize]);

  // 筛选集变化后当前页可能越界，回到第一页。
  useEffect(() => {
    setPage(1);
  }, [search, planFilter, status, kind, typeFilter, tasks?.length]);

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

  // 立即执行：绕开调度闸门，直接按任务号补跑一条排队/失败的任务。
  async function runNow(id: string) {
    setBusy(true);
    let started = false;
    try {
      const operation = await runTask(id);
      started = true;
      await run(operation, "立即执行");
      await load();
    } catch (problem) {
      if (!started) {
        notify("bad", "立即执行失败", problem instanceof Error ? problem.message : undefined);
      }
    } finally {
      setBusy(false);
    }
  }

  // 暂停/恢复队列派发：能力此前只读不写，这里补上控制入口。
  async function toggleQueue(queue: Queue) {
    setBusy(true);
    try {
      if (queue.paused) await resumeQueue(queue.id);
      else await pauseQueue(queue.id);
      notify("ok", queue.paused ? `恢复 ${queue.id}` : `暂停 ${queue.id}`);
      await load();
    } catch (problem) {
      notify("bad", "队列操作失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  // 立即派生：手动跑一轮声明式派生（否则只由 30s 后台循环触发）。
  async function deriveNow() {
    setBusy(true);
    let started = false;
    try {
      const operation = await runDerivations();
      started = true;
      await run(operation, "立即派生");
      await load();
    } catch (problem) {
      if (!started) {
        notify("bad", "派生失败", problem instanceof Error ? problem.message : undefined);
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
          <h1 className="page-title">任务与执行记录</h1>
          <p className="page-lede">
            采集、内容加工与投递的任务状态和执行记录。内部租约与句柄不展示；批量操作逐项执行并保留成功项。
          </p>
        </div>
        <div className="page-actions">
          <ProjectScopeBadge />
          <button type="button" className="btn" onClick={() => void load()} disabled={loading}>
            <IconRefresh size={14} /> 刷新
          </button>
          <button type="button" className="btn" onClick={() => void deriveNow()} disabled={busy}>
            <IconPlay size={14} /> 立即派生
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
              <span className="queue-name">{t("task.queue", queue.id, queue.id)}</span>
              <span className={`badge ${queue.paused || queue.stalled ? "warn" : "ok"}`}>
                {queue.paused ? "暂停" : queue.stalled ? queue.reason || "排队受阻" : "运行中"}
              </span>
              <span className="fill" />
              <span className="small muted">{queue.running}/{queue.capacity}</span>
              <button
                type="button"
                className="btn btn-sm"
                onClick={() => void toggleQueue(queue)}
                disabled={busy}
                title={queue.paused ? "恢复队列" : "暂停队列"}
              >
                {queue.paused ? <IconPlay size={12} /> : <IconPause size={12} />}
              </button>
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
        </select>
        <select className="select" value={typeFilter} onChange={(event) => setTypeFilter(event.target.value)} aria-label="信源类型">
          <option value="all">全部信源类型</option>
          {typeRecords.map((item) => (
            <option key={item.id} value={item.id}>{item.label}</option>
          ))}
        </select>
        <select className="select" value={planFilter} onChange={(event) => setPlanFilter(event.target.value)} aria-label="采集计划">
          <option value="all">全部计划</option>
          {plans.map((plan) => (
            <option key={plan.id} value={plan.id}>{plan.name ?? plan.id}</option>
          ))}
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
                  <th>计划</th>
                  <th>状态</th>
                  <th>队列</th>
                  <th>创建</th>
                  <th>耗时/错误</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {pagedRows.map((task) => (
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
                        <button
                          type="button"
                          className="row-subject"
                          onClick={() => openTask(task)}
                        >
                          {task.title || task.sourceId || task.id}
                        </button>
                        {typeOfTask(task) ? (
                          <span className="badge neutral">{typeLabelOf(typeOfTask(task))}</span>
                        ) : null}
                        <span className="source-sub mono">{task.id}</span>
                      </div>
                    </td>
                    <td><span className="tag">{kindLabel(task.kind)}</span></td>
                    <td className="small">{planOfTask(task)?.name ?? planOfTask(task)?.id ?? "—"}</td>
                    <td>
                      <div className="cell-main">
                        <StatusBadge status={task.status} />
                        {task.attempt && task.attempt > 1 ? <span className="source-sub">第 {task.attempt} 次</span> : null}
                      </div>
                    </td>
                    <td className="mono small">{task.queue ? t("task.queue", task.queue) : "—"}</td>
                    <td className="small muted">{relative(task.createdAt)}</td>
                    <td className="small">
                      {task.error ? (
                        <span style={{ color: "var(--danger)" }} title={task.error}>{task.error.slice(0, 42)}{task.error.length > 42 ? "…" : ""}</span>
                      ) : (
                        <span className="muted">{task.retryAt ? `重试 ${relative(task.retryAt)}` : "—"}</span>
                      )}
                    </td>
                    <td className="actions">
                      {["queued", "failed"].includes(task.status) ? (
                        <button type="button" className="btn btn-sm" onClick={() => void runNow(task.id)} disabled={busy}>
                          <IconPlay size={12} /> 立即执行
                        </button>
                      ) : null}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        {rows.length > 0 ? (
          <Pager
            page={page}
            pageSize={pageSize}
            total={rows.length}
            capped={capped}
            onPage={setPage}
            onPageSize={(size) => {
              setPageSize(size);
              setPage(1);
            }}
          />
        ) : null}
      </Card>

      {detail ? (
        <TaskDrawer
          task={detail}
          planLabel={planOfTask(detail)?.name ?? planOfTask(detail)?.id ?? detail.planId ?? null}
          onClose={closeTask}
        />
      ) : null}
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

function TaskDrawer({ task, planLabel, onClose }: { task: Task; planLabel: string | null; onClose: () => void }) {
  const correlation = task.runId || task.id;
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
            <span>{kindLabel(task.kind)}</span>
            {task.queue ? <span>队列 {t("task.queue", task.queue)}</span> : null}
          </div>
        </div>
        <dl className="kv">
          <dt>来源</dt><dd className="mono">{task.sourceId ? task.sourceId.replace(/^source\//, "") : "—"}</dd>
          <dt>采集计划</dt><dd>{planLabel || "—"}</dd>
          <dt>原因</dt><dd>{task.reason ? t("task.reason", task.reason) : "—"}</dd>
          <dt>创建</dt><dd>{stamp(task.createdAt)}</dd>
          <dt>更新</dt><dd>{stamp(task.updatedAt)}</dd>
          <dt>重试时间</dt><dd>{stamp(task.retryAt)}</dd>
          <dt>采集运行</dt><dd className="mono">{task.runId || "—"}</dd>
          <dt>错误</dt><dd style={{ color: task.error ? "var(--danger)" : undefined }}>{task.error || "—"}</dd>
        </dl>
        {task.locator ? (
          <div className="drawer-section">
            <h3>定位</h3>
            <StructuredValue value={task.locator} />
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
