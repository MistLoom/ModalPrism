// 治理页：出站节律、熔断与站点守则的处置台。
import { useCallback, useEffect, useState } from "react";

import { IconRefresh } from "../components/icons";
import { Card, Empty, Pill, Skeleton } from "../components/ui";
import { committerFlush, committerStatus, egressReport, egressReset } from "../lib/api";
import { bytes, relative } from "../lib/format";
import { useConsole } from "../lib/console";
import type { CommitterStatus, EgressReport } from "../lib/types";

function relativeMs(ms: number | null): string {
  if (!ms) {
    return "—";
  }
  return relative(new Date(ms).toISOString());
}

function circuitTone(circuit: {
  openUntil: number | null;
  consecutive: number;
}): { tone: "ok" | "warn" | "bad" | "idle"; label: string } {
  if (circuit.openUntil !== null && circuit.openUntil > Date.now()) {
    return { tone: "bad", label: "熔断中" };
  }
  if (circuit.consecutive > 0) {
    return { tone: "warn", label: "连续失败" };
  }
  return { tone: "ok", label: "正常" };
}

export function GovernancePage() {
  const { notify, refresh } = useConsole();
  const [report, setReport] = useState<EgressReport | null>(null);
  const [store, setStore] = useState<CommitterStatus | null>(null);
  const [resetting, setResetting] = useState<string | null>(null);
  const [flushing, setFlushing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [next, status] = await Promise.all([egressReport(), committerStatus()]);
      setReport(next);
      setStore(status);
      setError(null);
    } catch (problem) {
      setError(problem instanceof Error ? problem.message : "读取失败");
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const reset = async (host: string) => {
    setResetting(host);
    try {
      const result = await egressReset(host);
      notify(
        "ok",
        result.existed ? `已重置 ${host} 的熔断` : `${host} 没有熔断记录`,
        result.existed ? "下一轮采集会直接尝试该站点" : undefined,
      );
      await load();
    } catch (problem) {
      notify("bad", "重置失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setResetting(null);
    }
  };

  const flush = async () => {
    setFlushing(true);
    try {
      await committerFlush();
      notify("ok", "暂存区已提交", "内容立即写入内容库");
      refresh();
    } catch (problem) {
      notify("bad", "提交失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setFlushing(false);
    }
  };

  const circuits = report?.circuits ?? [];
  const robots = report?.robots ?? [];
  const rates = report?.rates ?? [];
  const audits = report?.audits ?? [];
  const openCircuits = circuits.filter(
    (item) => item.openUntil !== null && item.openUntil > Date.now(),
  ).length;
  const busiest = circuits.reduce(
    (max, item) => (item.consecutive > max ? item.consecutive : max),
    0,
  );

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <span className="page-kicker">governance · 治理</span>
          <h1 className="page-title">
            出站<em>治理</em>
          </h1>
          <p className="page-lede">
            出站请求的约束机制：私网拦截、站点守则、令牌桶限速与失败熔断。熔断可以在页面手动复位，其余由策略自动执行。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn btn-sm" onClick={() => void load()}>
            <IconRefresh size={13} /> 刷新
          </button>
          <button type="button" className="btn btn-sm btn-quiet" onClick={() => void flush()} disabled={flushing}>
            {flushing ? "提交中…" : "立即提交暂存"}
          </button>
        </div>
      </header>

      {error ? (
        <div className="note is-bad">
          <strong>治理数据读取失败</strong>
          <span>{error}</span>
        </div>
      ) : null}

      <div className="stat-grid">
        <div className="stat">
          <span className="stat-label">熔断中的主机</span>
          <span className="stat-value">{report === null ? "—" : openCircuits}</span>
          <span className="stat-foot">{openCircuits ? "可以在下方手动重置" : "出站通畅"}</span>
        </div>
        <div className="stat">
          <span className="stat-label">最多连续失败</span>
          <span className="stat-value">{report === null ? "—" : busiest}</span>
          <span className="stat-foot">{busiest >= 3 ? "接近熔断线，建议检查" : "都在安全范围"}</span>
        </div>
        <div className="stat">
          <span className="stat-label">已知站点守则</span>
          <span className="stat-value">{report === null ? "—" : robots.length}</span>
          <span className="stat-foot">{robots.filter((item) => item.failed).length} 份拉取失败</span>
        </div>
        <div className="stat">
          <span className="stat-label">速率桶</span>
          <span className="stat-value">{report === null ? "—" : rates.length}</span>
          <span className="stat-foot">按 host 与通道限速</span>
        </div>
      </div>

      <div className="gov-grid">
        <div className="gov-main">
          <Card
            title="出站域"
            sub="熔断状态与手动复位"
            tight
            foot={<span className="faint">连续失败达到阈值自动熔断，冷却期指数退避；重置会把计数与冷却归零。</span>}
          >
            {report === null ? (
              <Skeleton lines={4} />
            ) : circuits.length === 0 ? (
              <Empty title="还没有出站记录" text="等采集跑起来，这里会出现每个站点的访问状态。" />
            ) : (
              <div className="table-scroll">
                <table className="table domain-table">
                  <thead>
                    <tr>
                      <th>主机</th>
                      <th>状态</th>
                      <th className="align-right">连续失败</th>
                      <th className="align-right">熔断次数</th>
                      <th className="align-right">冷却</th>
                      <th className="align-right">操作</th>
                    </tr>
                  </thead>
                  <tbody>
                    {circuits.map((circuit) => {
                      const tone = circuitTone(circuit);
                      const isOpen = circuit.openUntil !== null && circuit.openUntil > Date.now();
                      return (
                        <tr key={circuit.host}>
                          <td>
                            <span className="mono cell-strong">{circuit.host}</span>
                            {isOpen ? (
                              <div className="faint">开放至 {relativeMs(circuit.openUntil)}</div>
                            ) : null}
                          </td>
                          <td>
                            <Pill tone={tone.tone} dot>
                              {tone.label}
                            </Pill>
                          </td>
                          <td className="num align-right">{circuit.consecutive}</td>
                          <td className="num align-right">{circuit.openings}</td>
                          <td className="num align-right">{Math.round(circuit.cooldownMs / 1000)}s</td>
                          <td className="align-right">
                            <button
                              type="button"
                              className="btn btn-sm"
                              disabled={resetting === circuit.host}
                              onClick={() => void reset(circuit.host)}
                            >
                              {resetting === circuit.host ? "重置中…" : "重置熔断"}
                            </button>
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            )}
          </Card>

          <Card title="速率" sub="令牌桶与在途请求" tight>
            {report === null ? (
              <Skeleton lines={3} />
            ) : rates.length === 0 ? (
              <Empty title="没有限速记录" text="所有目标都在免费额度内，或尚未触发限速。" />
            ) : (
              <ul className="rate-list">
                {rates.map((rate) => (
                  <li key={rate.key} className="rate-row">
                    <span className="mono rate-key">{rate.key}</span>
                    <span className="rate-nums">
                      令牌 <strong className="num">{rate.tokens}</strong>
                      <span className="sep" aria-hidden="true" />
                      在途 <strong className="num">{rate.inFlight}</strong>
                      {rate.notBefore !== null ? (
                        <span className="faint"> · 等待至 {relativeMs(rate.notBefore)}</span>
                      ) : null}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </Card>
        </div>

        <div className="gov-side">
          <Card title="站点守则" sub="robots.txt 缓存" tight>
            {report === null ? (
              <Skeleton lines={3} />
            ) : robots.length === 0 ? (
              <Empty title="还没有守则缓存" text="首次访问站点时才会拉取 robots.txt。" />
            ) : (
              <ul className="robots-list">
                {robots.map((item) => (
                  <li key={item.origin} className="robots-item">
                    <div className="robots-head">
                      <span className="mono cell-strong">{item.origin}</span>
                      <Pill tone={item.failed ? "warn" : item.status === 200 ? "ok" : "idle"} dot>
                        {item.failed ? "拉取失败" : `HTTP ${item.status}`}
                      </Pill>
                    </div>
                    <div className="faint">
                      {relativeMs(item.fetchedAt)} · {bytes(item.bodyBytes)}
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </Card>

          <Card title="出站审计" sub="最近 20 条拦截与放行" tight>
            {report === null ? (
              <Skeleton lines={4} />
            ) : audits.length === 0 ? (
              <Empty title="还没有审计记录" text="被拦下的请求会在这里留下痕迹。" />
            ) : (
              <ul className="audit-list">
                {audits.map((audit, index) => (
                  <li key={`${audit.at_ms}-${index}`} className="audit-row">
                    <div className="audit-line">
                      <Pill tone={audit.category === "blocked" ? "bad" : "info"}>{audit.category}</Pill>
                      <span className="mono audit-origin">{audit.origin}</span>
                    </div>
                    <div className="faint audit-why">
                      {audit.reason} · {relativeMs(audit.at_ms)}
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </Card>

          <Card title="存储" sub="控制库 / 正文 / 暂存">
            {store === null ? (
              <Skeleton lines={3} />
            ) : (
              <dl className="defs">
                <dt>控制库</dt>
                <dd className="mono">{store.control}</dd>
                <dt>正文块</dt>
                <dd className="mono">{store.blobs}</dd>
                <dt>暂存区</dt>
                <dd className="mono">{store.staging}</dd>
              </dl>
            )}
          </Card>
        </div>
      </div>
    </div>
  );
}
