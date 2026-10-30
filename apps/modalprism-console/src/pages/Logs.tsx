import { useCallback, useEffect, useState } from "react";

import { IconDownload, IconRefresh } from "../components/icons";
import { Pager } from "../components/Pager";
import { Card, Drawer, Empty, Field, Loading, Modal, Problem, StatusBadge, StructuredValue, Tabs, useUi } from "../components/ui";
import {
  exportObservability,
  listAuditEvents,
  listEvents,
  listHealthHistory,
  listLogs,
  recomputeHealth,
} from "../lib/api";
import { relative, stamp } from "../lib/format";
import { useRoute } from "../lib/router";
import type { HealthHistory, LogRecord } from "../lib/types";

type Tab = "logs" | "events" | "audit" | "health";

export function LogsPage() {
  const { params } = useRoute();
  const { notify, run } = useUi();
  const [tab, setTab] = useState<Tab>("logs");
  const [records, setRecords] = useState<LogRecord[] | null>(null);
  const [health, setHealth] = useState<HealthHistory[] | null>(null);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(50);
  const [total, setTotal] = useState(0);
  const [query, setQuery] = useState("");
  const [level, setLevel] = useState("");
  const [correlation, setCorrelation] = useState(params.get("correlationId") ?? "");
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);
  const [detail, setDetail] = useState<LogRecord | null>(null);
  const [exportOpen, setExportOpen] = useState(false);

  const load = useCallback(
    async (target = page) => {
      setLoading(true);
      const offset = (Math.max(1, target) - 1) * pageSize;
      try {
        if (tab === "health") {
          const result = await listHealthHistory({ limit: pageSize, offset, sourceId: correlation || undefined });
          setHealth(result.items);
          setTotal(result.total ?? result.items.length + offset);
        } else {
          const filters = {
            limit: pageSize,
            offset,
            query: query.trim() || undefined,
            level: level || undefined,
            correlationId: correlation.trim() || undefined,
          };
          const result =
            tab === "events"
              ? await listEvents(filters)
              : tab === "audit"
                ? await listAuditEvents(filters)
                : await listLogs(filters);
          setRecords(result.items);
          setTotal(result.total ?? result.items.length + offset);
        }
        setError(null);
      } catch (problem) {
        setError(problem);
      } finally {
        setLoading(false);
      }
    },
    [tab, query, level, correlation, page, pageSize],
  );

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    setCorrelation(params.get("correlationId") ?? "");
    setLevel(params.get("level") ?? "");
  }, [params]);

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <div className="kicker">logs · 日志</div>
          <h1 className="page-title">运行日志与审计记录</h1>
          <p className="page-lede">
            三类观测流分工明确：日志可清理，事件只追加，审计不可修改。requestId、operationId、taskId、runId 可以互相追踪；其他页面只带过滤条件跳到这里。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn" onClick={() => setExportOpen(true)}>
            <IconDownload size={14} /> 导出
          </button>
          <button type="button" className="btn" onClick={() => void load()} disabled={loading}>
            <IconRefresh size={14} /> 刷新
          </button>
          {tab === "health" ? (
            <button type="button" className="btn btn-primary" onClick={() => void (async () => {
              let started = false;
              try {
                const operation = await recomputeHealth();
                started = true;
                await run(operation, "重算健康");
                await load();
              } catch (problem) {
                if (!started) {
                  notify("bad", "重算健康失败", problem instanceof Error ? problem.message : undefined);
                }
              }
            })()}>
              重算健康
            </button>
          ) : null}
        </div>
      </header>

      {error ? <Problem error={error} onRetry={() => void load()} /> : null}

      <Tabs
        value={tab}
        onChange={(value) => {
          setTab(value as Tab);
          setPage(1);
        }}
        items={[
          { id: "logs", label: "日志" },
          { id: "events", label: "事件" },
          { id: "audit", label: "审计" },
          { id: "health", label: "健康历史" },
        ]}
      />

      <div className="toolbar-band">
        <label className="search">
          <span className="muted" aria-hidden="true">⌕</span>
          <input
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setPage(1);
            }}
            placeholder="全文搜索消息"
            aria-label="搜索日志"
          />
        </label>
        {tab !== "health" ? (
          <select className="select" value={level} onChange={(event) => {
              setLevel(event.target.value);
              setPage(1);
            }} aria-label="级别筛选">
            <option value="">全部级别</option>
            <option value="trace">trace</option>
            <option value="debug">debug</option>
            <option value="info">info</option>
            <option value="warn">warn</option>
            <option value="error">error</option>
          </select>
        ) : null}
        <input className="input mono" style={{ maxWidth: 320 }} value={correlation} onChange={(event) => {
            setCorrelation(event.target.value);
            setPage(1);
          }} placeholder="requestId / operationId / taskId / runId" aria-label="关联标识" />
        {correlation ? (
          <button type="button" className="btn btn-sm" onClick={() => {
              setCorrelation("");
              setPage(1);
            }}>清除关联</button>
        ) : null}
        <span className="fill" />
        <span className="small muted">{total} 条</span>
      </div>

      {tab === "health" ? (
        health === null ? <Loading lines={5} /> : health.length === 0 ? <Card title="健康历史"><Empty title="还没有健康评估" text="点击“重算健康”生成第一条不可变记录。" /></Card> : (
          <div className="stack">
            <div className="grid cols-2">
              {health.map((item, index) => (
                <Card key={`${item.evaluatedAt}-${index}`} title={stamp(item.evaluatedAt)} sub={`${item.sources.length} 个信源 · ${item.status}`} flush>
                  <div className="list">
                    {item.sources.map((source) => (
                      <div className="list-row" key={source.sourceId}>
                        <span className="list-main">
                          <span className="list-title mono">{source.sourceId}</span>
                          <span className="list-meta">{source.reason || "无异常"} · 今日 {source.todayYield ?? 0}</span>
                        </span>
                        <StatusBadge status={source.status} />
                      </div>
                    ))}
                  </div>
                </Card>
              ))}
            </div>
            <Pager
              page={page}
              pageSize={pageSize}
              total={total}
              onPage={setPage}
              onPageSize={(size) => {
                setPageSize(size);
                setPage(1);
              }}
            />
          </div>
        )
      ) : records === null ? (
        <Loading lines={5} />
      ) : records.length === 0 ? (
        <Card title="记录"><Empty title="没有匹配的记录" text="调整关键词、级别或关联标识。" /></Card>
      ) : (
        <Card title={tab === "audit" ? "审计事件" : tab === "events" ? "结构化事件" : "运行日志"} sub="点击一行查看完整关联字段" flush>
          <div className="table-wrap">
            <table className="table log-table">
              <thead><tr><th>时间</th><th>级别</th><th>种类</th><th>消息</th><th>关联</th></tr></thead>
              <tbody>
                {records.map((record) => (
                  <tr key={`${record.stream}-${record.cursor}`} onClick={() => setDetail(record)} style={{ cursor: "pointer" }}>
                    <td className="small muted">{stamp(record.timestamp)}</td>
                    <td><span className={`level ${record.level}`}>{record.level}</span></td>
                    <td><span className="tag">{record.kind}</span></td>
                    <td className="message">{record.message}</td>
                    <td className="mono small">{record.operationId || record.taskId || record.runId || record.requestId || "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <Pager
            page={page}
            pageSize={pageSize}
            total={total}
            onPage={setPage}
            onPageSize={(size) => {
              setPageSize(size);
              setPage(1);
            }}
          />
        </Card>
      )}

      {detail ? <RecordDrawer record={detail} onClose={() => setDetail(null)} /> : null}
      {exportOpen ? <ExportModal onClose={() => setExportOpen(false)} /> : null}
    </div>
  );
}

function RecordDrawer({ record, onClose }: { record: LogRecord; onClose: () => void }) {
  const pairs: [string, string | null | undefined][] = [
    ["requestId", record.requestId],
    ["operationId", record.operationId],
    ["taskId", record.taskId],
    ["sourceId", record.sourceId],
    ["contentId", record.contentId],
    ["runId", record.runId],
    ["attemptId", record.attemptId],
    ["deliveryId", record.deliveryId],
    ["configGeneration", record.configGeneration === null || record.configGeneration === undefined ? null : String(record.configGeneration)],
  ];
  return (
    <Drawer
      title={record.message}
      sub={`${record.kind} · ${stamp(record.timestamp)}`}
      onClose={onClose}
      footer={<button type="button" className="btn" onClick={onClose}>关闭</button>}
    >
      <div className="stack" style={{ gap: 14 }}>
        <div className="detail-meta">
          <span className={`level ${record.level}`}>{record.level}</span>
          <span className="tag">{record.stream}</span>
          <span>{relative(record.timestamp)}</span>
        </div>
        <dl className="kv">
          {pairs.map(([label, value]) => (
            <div key={label} style={{ display: "contents" }}>
              <dt>{label}</dt>
              <dd className="mono">{value || "—"}</dd>
            </div>
          ))}
        </dl>
        {record.data ? (
          <div className="drawer-section">
            <h3>数据</h3>
            <StructuredValue value={record.data} />
          </div>
        ) : null}
      </div>
    </Drawer>
  );
}

function ExportModal({ onClose }: { onClose: () => void }) {
  const { notify, run } = useUi();
  const [stream, setStream] = useState("logs");
  const [format, setFormat] = useState("jsonl");
  const [busy, setBusy] = useState(false);
  const [content, setContent] = useState<string | null>(null);

  async function start() {
    setBusy(true);
    let started = false;
    try {
      const operation = await exportObservability(stream, format, {});
      started = true;
      const finished = await run(operation, "导出观测流", { silentSuccess: true });
      const summary = (finished.resultSummary ?? {}) as Record<string, unknown>;
      setContent(String(summary.content ?? ""));
      notify("ok", "导出已生成", `${String(summary.count ?? 0)} 条，24 小时内可下载。`);
    } catch (problem) {
      if (!started) {
        notify("bad", "导出失败", problem instanceof Error ? problem.message : undefined);
      }
    } finally {
      setBusy(false);
    }
  }

  function download() {
    if (content === null) return;
    const blob = new Blob([content], { type: format === "csv" ? "text/csv;charset=utf-8" : "application/x-ndjson;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = `modalprism-${stream}-${Date.now()}.${format}`;
    anchor.click();
    URL.revokeObjectURL(url);
  }

  return (
    <Modal
      title="导出观测流"
      sub="只包含本次选择的流与过滤结果；审计导出本身会写入审计"
      onClose={onClose}
      wide
      footer={<><button type="button" className="btn" onClick={onClose}>关闭</button>{content !== null ? <button type="button" className="btn btn-primary" onClick={download}><IconDownload size={14} /> 下载</button> : <button type="button" className="btn btn-primary" onClick={() => void start()} disabled={busy}>{busy ? "导出中…" : "开始导出"}</button>}</>}
    >
      <div className="stack" style={{ gap: 12 }}>
        <div className="grid cols-2">
          <Field label="流">
            <select className="select" value={stream} onChange={(event) => setStream(event.target.value)}>
              <option value="logs">日志</option>
              <option value="events">事件</option>
              <option value="audit">审计</option>
            </select>
          </Field>
          <Field label="格式">
            <select className="select" value={format} onChange={(event) => setFormat(event.target.value)}>
              <option value="jsonl">JSONL</option>
              <option value="csv">CSV</option>
            </select>
          </Field>
        </div>
        {content !== null ? <textarea className="textarea export-box" readOnly value={content} /> : <div className="alert info">导出会继承页面脱敏规则，不包含密钥或代理凭据。</div>}
      </div>
    </Modal>
  );
}
