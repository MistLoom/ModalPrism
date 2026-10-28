import { useCallback, useEffect, useState } from "react";

import { IconAlert, IconChevronRight, IconRefresh } from "../components/icons";
import { Card, Empty, Loading, Problem, StatusBadge } from "../components/ui";
import { systemOverview } from "../lib/api";
import { contentKindLabel, count, relative, statusTone } from "../lib/format";
import { ProjectScopeBadge } from "../lib/project";
import { navigate } from "../lib/router";
import type { Overview } from "../lib/types";

export function OverviewPage() {
  const [data, setData] = useState<Overview | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setData(await systemOverview());
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
          <h1 className="page-title">今天的采集、生产与投递</h1>
          <p className="page-lede">
            先处理阻碍采集与投递的事项，再决定是否扩大范围。每个数字都能跳到对应的过滤结果；本页不提供绕过治理的写入口。
          </p>
        </div>
        <div className="page-actions">
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
            <Stat label="待处理" value={data.counts.attention} note="健康、发布与告警" onClick={() => navigate("logs?level=warn")} alert={data.counts.attention > 0} />
            <Stat label="执行中任务" value={data.counts.tasksRunning} note={`${data.counts.tasksQueued} 个排队`} onClick={() => navigate("tasks?status=running")} />
            <Stat label="已入库内容" value={data.counts.content} note={`${data.counts.tombstoned} 条历史残留待清理`} onClick={() => navigate("content")} />
          </div>

          <div className="split-2">
            <Card title="需要注意" sub="按影响排序；点击进入已过滤的页面" flush>
              {data.attention.length === 0 ? (
                <Empty title="没有需要立即处理的事项" text="采集、生产与投递都在正常范围内。" />
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
                        else if (item.kind === "publication") navigate("outbound?tab=publications&status=failed");
                        else navigate("logs?level=warn");
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

            <Card title="队列" sub="暂停只影响新一轮派发" action={<button type="button" className="btn btn-sm" onClick={() => navigate("tasks")}>任务</button>}>
              {data.queues.length === 0 ? (
                <Empty title="还没有队列" text="调度器完成第一次派发后出现。" />
              ) : (
                <div className="stack">
                  {data.queues.map((queue) => {
                    const total = Math.max(1, queue.capacity || 1);
                    const pct = Math.min(100, Math.round((queue.running / total) * 100));
                    return (
                      <div className={`queue-card${queue.paused ? " is-paused" : ""}`} key={queue.id}>
                        <div className="queue-head">
                          <span className="queue-name mono">{queue.id}</span>
                          <span className={`badge ${queue.paused ? "warn" : "ok"}`}>{queue.paused ? "暂停" : "派发中"}</span>
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
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </Card>
          </div>

          <div className="split-2">
            <Card
              title="信源健康"
              sub="最近一批信源的产出与下一次运行"
              action={<button type="button" className="btn btn-sm" onClick={() => navigate("sources")}>全部信源</button>}
              flush
            >
              {data.sources.length === 0 ? (
                <Empty title="还没有信源" text="在信源页新建或导入一份订阅源。" action={<button type="button" className="btn btn-primary btn-sm" onClick={() => navigate("sources?new=1")}>新建信源</button>} />
              ) : (
                <div className="list">
                  {data.sources.map((source) => (
                    <button
                      type="button"
                      key={source.id}
                      className="list-row clickable"
                      style={{ background: "transparent", border: 0, borderBottom: "1px solid var(--line-soft)", textAlign: "left", width: "100%" }}
                      onClick={() => navigate(`sources?query=${encodeURIComponent(source.id)}`)}
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
                      <IconChevronRight size={14} />
                    </button>
                  ))}
                </div>
              )}
            </Card>

            <Card title="最近操作" sub="异步动作的持久化记录" action={<button type="button" className="btn btn-sm" onClick={() => navigate("logs")}>日志</button>} flush>
              {data.recentOperations.length === 0 ? (
                <Empty title="还没有操作" text="采集、生产、发布与维护都会留下 Operation。" />
              ) : (
                <div className="list">
                  {data.recentOperations.map((operation) => (
                    <button
                      type="button"
                      key={operation.operationId}
                      className="list-row clickable"
                      style={{ background: "transparent", border: 0, borderBottom: "1px solid var(--line-soft)", textAlign: "left", width: "100%" }}
                      onClick={() => navigate(`logs?correlationId=${encodeURIComponent(operation.operationId)}`)}
                    >
                      <span className="list-main">
                        <span className="list-title">{operation.kind}</span>
                        <span className="list-meta">
                          <span className="mono">{operation.operationId.slice(0, 14)}</span>
                          <span>{relative(operation.createdAt)}</span>
                        </span>
                      </span>
                      <span className={`badge ${statusTone(operation.status)}`}>{operation.status}</span>
                    </button>
                  ))}
                </div>
              )}
            </Card>
          </div>

          <Card title="内容形态" sub="只统计当前可见内容" flush>
            {data.contentKinds.length === 0 ? (
              <Empty title="还没有内容" text="信源第一次采集完成后出现。" />
            ) : (
              <div className="card-body grid cols-4">
                {data.contentKinds.map((kind) => (
                  <div className="stat" key={kind.variant}>
                    <span className="stat-label">{contentKindLabel(kind.variant)}</span>
                    <span className="stat-value">{count(kind.total)}</span>
                  </div>
                ))}
              </div>
            )}
          </Card>
        </>
      ) : null}
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
