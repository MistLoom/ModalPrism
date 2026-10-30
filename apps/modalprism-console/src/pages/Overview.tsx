import { useCallback, useEffect, useMemo, useState } from "react";

import { IconAlert, IconChevronRight, IconHistory, IconPause, IconPlay, IconRefresh } from "../components/icons";
import { RowSubject } from "../components/ListRow";
import { RunsDrawer } from "../components/RunsDrawer";
import { SettingsCheck } from "../components/SettingsCheck";
import { Card, Drawer, Empty, Loading, Problem, StatusBadge, StructuredValue, useUi } from "../components/ui";
import {
  cancelOperation,
  getOperation,
  listCollectionRuns,
  listPlans,
  listSources,
  monitorSummary,
  operationEvents,
  pauseQueue,
  resumeQueue,
  retryOperation,
  systemOverview,
} from "../lib/api";
import { contentKindLabel, count, kindLabel, monitorAbilityLabel, monitorKindLabel, monitorPhaseLabel, operationEventLabel, refKindLabel, relative, stamp, statusLabel, statusTone } from "../lib/format";
import { ProjectScopeBadge } from "../lib/project";
import { navigate } from "../lib/router";
import { openSubject, subjectFromLocation, type SubjectKind } from "../lib/subject";
import type { CollectionRun, MonitorSummary, Operation, Overview, Plan, Source } from "../lib/types";

/** 操作关联资源里能在别处打开的主体；其余类型只回日志。 */
const REF_SUBJECTS: Partial<Record<string, SubjectKind>> = {
  source: "source",
  content: "content",
  plan: "plan",
  task: "task",
};

const RUN_LABELS: Record<string, string> = {
  succeeded: "成功",
  failed: "失败",
  running: "执行中",
  queued: "排队",
  cancelled: "已取消",
};

/**
 * 工作台：先看要不要动手，再决定去哪儿动手。
 * 行内能展开的（采集记录、操作详情）一律用速览抽屉，不把用户先扔到另一个页面再让他找回来。
 */
export function OverviewPage() {
  const [data, setData] = useState<Overview | null>(null);
  const [monitor, setMonitor] = useState<MonitorSummary | null>(null);
  const [runs, setRuns] = useState<CollectionRun[]>([]);
  const [sources, setSources] = useState<Source[]>([]);
  const [plans, setPlans] = useState<Plan[]>([]);
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);
  const [record, setRecord] = useState<{ sourceId?: string; runId?: string; title: string } | null>(null);
  const [operationId, setOperationId] = useState<string | null>(null);
  const [queueBusy, setQueueBusy] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [overview, runPage, sourcePage, planPage] = await Promise.all([
        systemOverview(),
        listCollectionRuns({ limit: 6 }).catch(() => ({ items: [] })),
        listSources({ limit: 200 }).catch(() => ({ items: [] })),
        listPlans({ limit: 200 }).catch(() => ({ items: [] })),
      ]);
      setData(overview);
      setRuns(runPage.items);
      setSources(sourcePage.items);
      setPlans(planPage.items);
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

  // 实时监测：框架 monitor 的内存视图，只读取不参与采集。
  useEffect(() => {
    let alive = true;
    const poll = async () => {
      try {
        const value = await monitorSummary();
        if (alive) setMonitor(value);
      } catch {
        // 监测不可用时不影响工作台其他内容。
      }
    };
    void poll();
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void poll();
    }, 5_000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, []);

  const sourceName = useMemo(() => {
    const map = new Map(sources.map((item) => [item.id, item.name]));
    return (id: string) => map.get(id) ?? id;
  }, [sources]);

  const planName = useMemo(() => {
    const map = new Map(plans.map((item) => [item.id, item.name]));
    return (id: string) => map.get(id) ?? id;
  }, [plans]);

  // 深链：`?run=` 直接展开那次采集运行的速览（其他页面的「运行」主体就落在这里）。
  useEffect(() => {
    if (!runs.length) return;
    const runId = subjectFromLocation("run");
    if (!runId) return;
    const hit = runs.find((item) => item.id === runId);
    if (hit) setRecord({ sourceId: hit.sourceId, runId: hit.id, title: `采集记录 · ${sourceName(hit.sourceId)}` });
  }, [runs, sourceName]);

  async function toggleQueue(queueId: string, paused: boolean) {
    setQueueBusy(queueId);
    try {
      if (paused) await resumeQueue(queueId);
      else await pauseQueue(queueId);
      await load();
    } finally {
      setQueueBusy(null);
    }
  }

  if (loading && !data) {
    return (
      <div className="page">
        <Loading lines={6} />
      </div>
    );
  }

  return (
    <div className="page">
      <header className="hero-band">
        <div className="page-head-text">
          <div className="kicker">overview · 运行总览</div>
          <h1 className="page-title">运行总览</h1>
          <p className="page-lede">
            采集、内容加工与投递的运行状态、待处理事项与近期记录。
          </p>
        </div>
        <div className="page-actions">
          <SettingsCheck />
          <ProjectScopeBadge />
          <StatusBadge status={data?.status} />
          <button type="button" className="btn" onClick={() => void load()} disabled={loading}>
            <IconRefresh size={14} /> 刷新
          </button>
        </div>
      </header>

      {error ? <Problem error={error} onRetry={() => void load()} /> : null}

      {data ? (
        <>
          <div className="stat-grid">
            <Stat label="信源" value={data.counts.sources} note={`${data.counts.enabledSources} 个启用 · ${data.counts.healthySources} 个正常`} onClick={() => navigate("sources")} />
            <Stat
              label="待处理"
              value={data.counts.unreadNotifications}
              note={data.counts.attention > 0 ? `${data.counts.attention} 项信源异常` : "未读通知与系统事件"}
              onClick={() => navigate("notifications")}
              alert={data.counts.unreadNotifications > 0 || data.counts.attention > 0}
            />
            <Stat label="执行中任务" value={data.counts.tasksRunning} note={`${data.counts.tasksQueued} 个排队`} onClick={() => navigate("tasks?status=running")} />
            <Stat label="已入库内容" value={data.counts.content} note={`${data.counts.tombstoned} 条历史残留待清理`} onClick={() => navigate("content")} />
          </div>

          <Card
            title="实时运行监测"
            sub="框架内置监测：在途调用、最近事件与队列水位"
            action={<span className="small muted">每 5 秒刷新</span>}
            flush
          >
            {!monitor || !monitor.available ? (
              <Empty title="监测未接入" text={monitor?.note ?? "宿主启动后会自动接入。"} />
            ) : (
              <div className="monitor-panel">
                <div className="monitor-stats">
                  <div className="monitor-stat">
                    <span className="monitor-stat-label">在途调用</span>
                    <strong>{monitor.chains?.length ?? 0}</strong>
                  </div>
                  <div className="monitor-stat">
                    <span className="monitor-stat-label">待写入事件</span>
                    <strong>{monitor.stats?.queued ?? 0}</strong>
                  </div>
                  <div className="monitor-stat">
                    <span className="monitor-stat-label">最近事件</span>
                    <strong>{monitor.stats?.recent ?? 0}</strong>
                  </div>
                  <div className="monitor-stat">
                    <span className="monitor-stat-label">丢弃</span>
                    <strong style={{ color: monitor.stats?.dropped ? "var(--warn)" : undefined }}>{monitor.stats?.dropped ?? 0}</strong>
                  </div>
                </div>
                <div className="monitor-lists">
                  <div className="stack" style={{ gap: 6 }}>
                    <span className="small muted">在途调用</span>
                    {(monitor.chains ?? []).length === 0 ? (
                      <span className="small muted">当前没有执行中的能力调用</span>
                    ) : (
                      (monitor.chains ?? []).slice(0, 6).map((chain, index) => (
                        <div className="monitor-row" key={`${chain.chain_id ?? index}`}>
                          <span className={`dot ${chain.running === false ? "warn" : "ok"}`} />
                          <span className="small">
                            {monitorAbilityLabel(chain.instance ?? chain.ability_id, chain.method) || "—"}
                          </span>
                          <span className="fill" />
                          <span className="small muted">{chain.started_at ? relative(chain.started_at) : "—"}</span>
                        </div>
                      ))
                    )}
                  </div>
                  <div className="stack" style={{ gap: 6 }}>
                    <span className="small muted">最近事件</span>
                    {(monitor.events ?? []).length === 0 ? (
                      <span className="small muted">暂无事件</span>
                    ) : (
                      (monitor.events ?? []).slice(-6).reverse().map((event, index) => (
                        <div className="monitor-row" key={`${event.seq ?? index}`}>
                          <span className={`dot ${event.error ? "bad" : "ok"}`} />
                          <span className="small" title={`${event.instance ?? ""}${event.method ? `.${event.method}` : ""} · ${event.kind ?? ""}`}>
                            {monitorAbilityLabel(event.instance, event.method) || monitorKindLabel(event.kind)}
                          </span>
                          <span className="fill" />
                          <span className="small muted">{monitorPhaseLabel(event.phase) || monitorKindLabel(event.kind)}</span>
                        </div>
                      ))
                    )}
                  </div>
                </div>
              </div>
            )}
          </Card>

          <div className="split-2">
            <Card title="需要注意" sub="按影响排序；选择一项进入对应页面" flush>
              {data.attention.length === 0 ? (
                <Empty title="无需处理事项" text="采集、内容加工与投递状态正常。" />
              ) : (
                <div className="card-body" style={{ display: "grid", gap: 0 }}>
                  {data.attention.map((item) => (
                    <button
                      key={`${item.kind}-${item.id}`}
                      type="button"
                      className="attention-row"
                      style={{ background: "transparent", border: 0, textAlign: "left", cursor: "pointer", width: "100%" }}
                      onClick={() => {
                        if (item.kind === "source") navigate(`sources?query=${encodeURIComponent(item.id ?? "")}`);
                        else if (item.kind === "publication") navigate("tasks?kind=publication&status=failed");
                        else navigate("notifications");
                      }}
                    >
                      <span style={{ color: item.status === "failed" ? "var(--danger)" : "var(--warn)", marginTop: 2 }}>
                        <IconAlert size={16} />
                      </span>
                      <span className="attention-main">
                        <span className="attention-title">{item.title}</span>
                        <span className="attention-note">{item.message || "需要检查"}</span>
                      </span>
                      <StatusBadge status={item.status} />
                      <IconChevronRight size={14} />
                    </button>
                  ))}
                </div>
              )}
            </Card>

            <Card title="队列" sub="暂停仅影响下一轮执行" flush>
              {data.queues.length === 0 ? (
                <Empty title="暂无队列" text="调度器首次执行后出现。" />
              ) : (
                <div className="card-body">
                  <div className="queue-grid">
                    {data.queues.map((queue) => {
                      const total = Math.max(1, queue.capacity || 1);
                      const pct = Math.min(100, Math.round((queue.running / total) * 100));
                      return (
                        <div className={`queue-card${queue.paused ? " is-paused" : ""}`} key={queue.id}>
                          <div className="queue-head">
                            <span className="queue-name mono">{queue.id}</span>
                            <span className={`badge ${queue.paused ? "warn" : "ok"}`}>{queue.paused ? "暂停" : "运行中"}</span>
                            <span className="fill" />
                            <span className="small muted mono">{queue.running}/{queue.capacity}</span>
                          </div>
                          <div className="bar">
                            <span style={{ width: `${pct}%` }} />
                          </div>
                          <div className="queue-nums">
                            <span>排队 <strong>{count(queue.queued)}</strong></span>
                            <span>执行 <strong>{count(queue.running)}</strong></span>
                            <span>完成 <strong>{count(queue.done ?? 0)}</strong></span>
                            <span className="fill" />
                            <button
                              type="button"
                              className="btn btn-ghost btn-sm"
                              disabled={queueBusy === queue.id}
                              onClick={() => void toggleQueue(queue.id, queue.paused)}
                            >
                              {queue.paused ? <IconPlay size={12} /> : <IconPause size={12} />}
                              {queue.paused ? "恢复" : "暂停"}
                            </button>
                          </div>
                        </div>
                      );
                    })}
                  </div>
                </div>
              )}
            </Card>
          </div>

          <div className="split-2">
            <Card
              title="最近采集"
              sub="展开查看本次运行的上下游与产出"
              action={<button type="button" className="btn btn-sm" onClick={() => navigate("plans")}>计划</button>}
              flush
            >
              {runs.length === 0 ? (
                <Empty
                  title="暂无采集记录"
                  text="计划触发与手动采集记录。"
                  action={<button type="button" className="btn btn-primary btn-sm" onClick={() => navigate("sources")}>去信源</button>}
                />
              ) : (
                <div className="list">
                  {runs.map((run) => (
                    <button
                      key={run.id}
                      type="button"
                      className="list-row clickable"
                      style={{ background: "transparent", border: 0, borderBottom: "1px solid var(--line-soft)", textAlign: "left", width: "100%" }}
                      onClick={() => setRecord({ sourceId: run.sourceId, runId: run.id, title: `采集记录 · ${sourceName(run.sourceId)}` })}
                    >
                      <span className={`dot ${run.status === "failed" ? "bad" : run.status === "succeeded" ? "ok" : "warn"}`} />
                      <span className="list-main">
                        <span className="list-title">
                          {sourceName(run.sourceId)}
                          <span className="badge neutral" style={{ marginLeft: 6 }}>{run.mode === "manual" ? "手动" : "计划"}</span>
                        </span>
                        <span className="list-meta">
                          <span>{RUN_LABELS[run.status] ?? run.status}</span>
                          <span>{relative(run.createdAt)}</span>
                          {run.planId ? <span>计划 {planName(run.planId)}</span> : null}
                        </span>
                        {run.error ? <span className="list-meta" style={{ color: "var(--danger)" }}>{run.error}</span> : null}
                      </span>
                      <IconHistory size={14} />
                    </button>
                  ))}
                </div>
              )}
            </Card>

            <Card
              title="信源健康"
              sub="近期信源产出与下一次运行"
              action={<button type="button" className="btn btn-sm" onClick={() => navigate("sources")}>全部信源</button>}
              flush
            >
              {data.sources.length === 0 ? (
                <Empty title="暂无信源" text="在信源页新建或导入订阅源。" action={<button type="button" className="btn btn-primary btn-sm" onClick={() => navigate("sources?new=1")}>新建信源</button>} />
              ) : (
                <div className="list">
                  {data.sources.map((source) => (
                    <button
                      type="button"
                      key={source.id}
                      className="list-row clickable"
                      style={{ background: "transparent", border: 0, borderBottom: "1px solid var(--line-soft)", textAlign: "left", width: "100%" }}
                      onClick={() => setRecord({ sourceId: source.id, title: `采集记录 · ${source.name}` })}
                    >
                      <span className={`dot ${source.health === "healthy" ? "ok" : source.health === "failed" ? "bad" : "warn"}`} />
                      <span className="list-main">
                        <span className="list-title">{source.name}</span>
                        <span className="list-meta">
                          <span className="mono">{source.id}</span>
                          <span>今日 {count(source.todayYield ?? 0)} 条</span>
                          <span>下次 {relative(source.nextRunAt)}</span>
                        </span>
                      </span>
                      <StatusBadge status={source.health} />
                      <IconHistory size={14} />
                    </button>
                  ))}
                </div>
              )}
            </Card>
          </div>

          <div className="split-2">
            <Card title="最近操作" sub="展开查看进度、失败原因与关联资源" action={<button type="button" className="btn btn-sm" onClick={() => navigate("logs")}>日志</button>} flush>
              {data.recentOperations.length === 0 ? (
                <Empty title="暂无操作" text="采集、投递与维护记录为 Operation。" />
              ) : (
                <div className="list">
                  {data.recentOperations.map((operation) => (
                    <button
                      type="button"
                      key={operation.operationId}
                      className="list-row clickable"
                      style={{ background: "transparent", border: 0, borderBottom: "1px solid var(--line-soft)", textAlign: "left", width: "100%" }}
                      onClick={() => setOperationId(operation.operationId)}
                    >
                      <span className="list-main">
                        <span className="list-title">{kindLabel(operation.kind)}</span>
                        <span className="list-meta">
                          <span className="mono">{operation.operationId.slice(0, 14)}</span>
                          <span>{relative(operation.createdAt)}</span>
                          <span className="mono">{operation.resourceRefs.map((ref) => `${refKindLabel(ref.kind)}：${ref.id}`).join("、") || "无关联资源"}</span>
                        </span>
                      </span>
                      <span className={`badge ${statusTone(operation.status)}`}>{statusLabel(operation.status)}</span>
                      <IconChevronRight size={14} />
                    </button>
                  ))}
                </div>
              )}
            </Card>

            <Card title="内容形态" sub="只统计当前可见内容" flush>
              {data.contentKinds.length === 0 ? (
                <Empty title="暂无内容" text="信源首次采集完成后出现。" />
              ) : (
                <div className="card-body grid cols-fill">
                  {data.contentKinds.map((kind) => (
                    <div className="stat" key={kind.variant}>
                      <span className="stat-label">{contentKindLabel(kind.variant)}</span>
                      <span className="stat-value">{count(kind.total)}</span>
                    </div>
                  ))}
                </div>
              )}
            </Card>
          </div>
        </>
      ) : null}

      {record ? (
        <RunsDrawer
          title={record.title}
          sourceId={record.sourceId}
          initialRunId={record.runId}
          onClose={() => setRecord(null)}
        />
      ) : null}

      {operationId ? <OperationDrawer id={operationId} onClose={() => setOperationId(null)} /> : null}
    </div>
  );
}

function Stat({ label, value, note, onClick, alert }: { label: string; value: number; note: string; onClick: () => void; alert?: boolean }) {
  return (
    <button type="button" className="stat" onClick={onClick} style={{ textAlign: "left", cursor: "pointer", borderColor: alert ? "var(--warn)" : undefined }}>
      <span className="stat-label">{label}</span>
      <span className="stat-value" style={{ color: alert ? "var(--warn)" : undefined }}>{count(value)}</span>
      <span className="stat-note">{note}</span>
    </button>
  );
}

/** 操作速览：状态、进度、关联资源与失败原因，外加取消/重试与回日志的入口。 */
function OperationDrawer({ id, onClose }: { id: string; onClose: () => void }) {
  const { notify } = useUi();
  const [operation, setOperation] = useState<Operation | null>(null);
  const [events, setEvents] = useState<Record<string, unknown>[]>([]);
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      const [detail, page] = await Promise.all([
        getOperation(id),
        operationEvents(id, { limit: 20 }).catch(() => ({ items: [] })),
      ]);
      setOperation(detail);
      setEvents(page.items);
      setError(null);
    } catch (problem) {
      setError(problem);
    }
  }, [id]);

  useEffect(() => {
    void load();
  }, [load]);

  async function act(label: string, action: () => Promise<Operation>) {
    setBusy(true);
    try {
      await action();
      notify("ok", `${label}已提交`);
      await load();
    } catch (problem) {
      notify("bad", `${label}失败`, problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Drawer
      title={operation ? kindLabel(operation.kind) : "操作详情"}
      sub={operation ? `${statusLabel(operation.status)} · ${stamp(operation.createdAt)}` : id}
      onClose={onClose}
      wide
      footer={
        <>
          <button type="button" className="btn" onClick={() => navigate(`logs?correlationId=${encodeURIComponent(id)}`)}>在日志中查看</button>
          <button type="button" className="btn" onClick={onClose}>关闭</button>
          {operation?.cancellable ? (
            <button type="button" className="btn btn-danger" disabled={busy} onClick={() => void act("取消操作", () => cancelOperation(id))}>取消操作</button>
          ) : null}
          {operation?.retryable ? (
            <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void act("重试操作", () => retryOperation(id))}>重试</button>
          ) : null}
        </>
      }
    >
      {error ? <Problem error={error} onRetry={() => void load()} /> : null}
      {!operation ? (
        <Loading lines={4} />
      ) : (
        <div className="stack" style={{ gap: 14 }}>
          <div className="row wrap" style={{ gap: 8 }}>
            <span className={`badge ${statusTone(operation.status)}`}>{statusLabel(operation.status)}</span>
            <span className="small muted">进度 {Math.round(operation.progress * 100)}%</span>
            {operation.completedAt ? <span className="small muted">完成于 {stamp(operation.completedAt)}</span> : null}
            {operation.retryOf ? <span className="small muted">重试自 {operation.retryOf}</span> : null}
          </div>

          {operation.errorSummary ? (
            <Card title="失败原因">
              <div className="stack" style={{ gap: 4 }}>
                <strong style={{ color: "var(--danger)" }}>{operation.errorSummary.message ?? "未提供原因"}</strong>
                <span className="small muted mono">
                  {operation.errorSummary.code ?? ""} {operation.errorSummary.requestId ?? ""}
                </span>
              </div>
            </Card>
          ) : null}

          <Card title="关联资源" flush>
            {operation.resourceRefs.length === 0 ? (
              <Empty title="没有关联资源" text="这个操作不指向具体资源。" />
            ) : (
              <div className="list">
                {operation.resourceRefs.map((ref) => {
                  const kind = REF_SUBJECTS[ref.kind];
                  return (
                    <div className="list-row" key={`${ref.kind}:${ref.id}`}>
                      <span className="list-main">
                        <RowSubject
                          onClick={() =>
                            kind
                              ? openSubject({ kind, id: ref.id })
                              : navigate(`logs?correlationId=${encodeURIComponent(id)}`)
                          }
                          title="打开关联资源"
                        >
                          {ref.id}
                        </RowSubject>
                        <span className="list-meta"><span>{refKindLabel(ref.kind)}</span></span>
                      </span>
                    </div>
                  );
                })}
              </div>
            )}
          </Card>

          {operation.resultSummary ? (
            <Card title="结果">
              <StructuredValue value={operation.resultSummary} />
            </Card>
          ) : null}

          <Card title="过程事件" sub="按时间正序，最近 20 条" flush>
            {events.length === 0 ? (
              <Empty title="没有过程事件" text="操作结束后可能已归档。" />
            ) : (
              <div className="list">
                {events.map((event, index) => (
                  <div className="list-row" key={String(event.id ?? index)}>
                    <span className="list-main">
                      <span className="list-title">{String(event.message ?? event.kind ?? "事件")}</span>
                      <span className="list-meta">
                        {event.kind ? <span>{operationEventLabel(String(event.kind))}</span> : null}
                        {event.createdAt ? <span>{stamp(String(event.createdAt))}</span> : null}
                      </span>
                    </span>
                  </div>
                ))}
              </div>
            )}
          </Card>
        </div>
      )}
    </Drawer>
  );
}
