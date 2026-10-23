import { useMemo } from "react";

import { navigate } from "../lib/router";
import { useConsole } from "../lib/console";
import { cadence, count, explainError, relative, until } from "../lib/format";
import type { Source } from "../lib/types";
import {
  Card,
  Empty,
  NextRun,
  Skeleton,
  Stat,
  StatusPill,
  YieldBar,
} from "../components/ui";
import { IconAlert, IconArrowRight, IconRefresh, IconShield } from "../components/icons";

export function OverviewPage() {
  const { state, sources, timeline, now, refresh, busy, snapshot } = useConsole();
  const loading = state === "loading";

  const active = sources.filter((source) => source.enabled);
  const attention = active.filter((source) => source.alert || source.health !== "ok");
  const yields = active.reduce((sum, source) => sum + source.yields7d, 0);
  const lastCommit = timeline.length ? timeline[timeline.length - 1] : null;
  const maxYield = Math.max(1, ...active.map((source) => source.yields7d));

  const upcoming = useMemo(
    () =>
      [...active]
        .filter((source) => source.nextAt)
        .sort((left, right) => (Date.parse(left.nextAt!) || 0) - (Date.parse(right.nextAt!) || 0))
        .slice(0, 8),
    [active],
  );

  const watch = useMemo(
    () =>
      [...active]
        .filter((source) => source.alert || source.health !== "ok" || source.lastError)
        .sort((left, right) => Number(Boolean(right.lastError)) - Number(Boolean(left.lastError)))
        .slice(0, 5),
    [active],
  );

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <span className="page-kicker">overview · 总览</span>
          <h1 className="page-title">
            {loading ? (
              "正在读取状态…"
            ) : attention.length > 0 ? (
              <>
                有 <em>{attention.length}</em> 个信源需要处理。
              </>
            ) : (
              <>
                所有信源按<em>节律</em>运行。
              </>
            )}
          </h1>
          <p className="page-lede">
            {loading
              ? "正在从采集进程读取信源清单与最近的提交。"
              : `共 ${active.length} 个信源在运行，最近 7 天采集 ${count(yields)} 条内容。本页列出需要处理的信源与最近提交。`}
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn" onClick={() => navigate("config")}>
            <IconShield size={13} />
            校验配置
          </button>
          <button type="button" className="btn btn-primary" onClick={refresh} disabled={busy}>
            <IconRefresh size={13} />
            {busy ? "同步中…" : "刷新观测"}
          </button>
        </div>
      </header>

      <div className="stat-grid">
        <Stat
          label="在守信源"
          value={loading ? "—" : active.length}
          unit={sources.length > active.length ? `/ ${sources.length} 总计` : "个"}
          foot={`${snapshot?.catalog.modules.filter((module) => module.kind === "source").length ?? 0} 个信源已登记`}
        />
        <Stat
          label="需要留意"
          value={loading ? "—" : attention.length}
          unit={attention.length > 0 ? "个" : ""}
          alert={attention.length > 0}
          foot={attention.length > 0 ? "健康、产出或配置达到告警线" : "没有异常信号"}
        />
        <Stat
          label="7 日产出"
          value={loading ? "—" : count(yields)}
          unit="条"
          foot={lastCommit ? `最近提交于 ${relative(lastCommit.publishedAt, now)}` : "尚无提交记录"}
        />
        <Stat
          label="快照游标"
          value={loading ? "—" : `#${lastCommit?.snapshot ?? "—"}`}
          foot={lastCommit ? `${lastCommit.commitIds.length} 个变更集，可回放` : "每次提交都会留下快照"}
        />
      </div>

      <div className="overview-grid">
        <div className="overview-main">
          <Card
            title="接下来"
            sub="按预计出发时间排序，点开就把该信源的细节拉到眼前"
            action={
              <button type="button" className="btn btn-quiet btn-sm" onClick={() => navigate("sources")}>
                全部信源
                <IconArrowRight size={12} />
              </button>
            }
            tight
          >
            {loading ? (
              <div style={{ padding: 16 }}>
                <Skeleton lines={4} />
              </div>
            ) : upcoming.length === 0 ? (
              <Empty
                title="还没有排期"
                text="信源的 schedule 里没有可用的间隔，或者所有任务都已完成，等待下一个周期。"
                action={
                  <button type="button" className="btn btn-sm" onClick={() => navigate("sources")}>
                    检查信源设置
                  </button>
                }
              />
            ) : (
              <div className="rail-list">
                {upcoming.map((source) => (
                  <button
                    key={source.id}
                    type="button"
                    className="rail-card"
                    onClick={() => navigate(`sources/${encodeURIComponent(source.id)}`)}
                  >
                    <span className={`rail-when${source.alert ? "" : " is-calm"}`}>
                      {source.alert ? <IconAlert size={11} /> : null}
                      {until(source.nextAt, now)}
                    </span>
                    <span className="rail-name">{source.name}</span>
                    <span className="rail-meta">
                      {source.accessorLabel} · {cadence(source.intervalMs)}
                    </span>
                  </button>
                ))}
              </div>
            )}
          </Card>

          <Card
            title="信源一览"
            sub="产出按最近 7 天统计；状态词与采集进程一致"
            tight
          >
            {loading ? (
              <div style={{ padding: 16 }}>
                <Skeleton lines={6} />
              </div>
            ) : active.length === 0 ? (
              <Empty
                title="还没有启用的信源"
                text="在 config/sources 下添加信源文件，或启用已登记的信源。"
              />
            ) : (
              <table className="table">
                <thead>
                  <tr>
                    <th>信源</th>
                    <th>通道</th>
                    <th>下次</th>
                    <th className="align-right">7 日产出</th>
                    <th>状态</th>
                  </tr>
                </thead>
                <tbody>
                  {active.slice(0, 8).map((source) => (
                    <tr
                      key={source.id}
                      className="row-button"
                      onClick={() => navigate(`sources/${encodeURIComponent(source.id)}`)}
                    >
                      <td>
                        <div className="cell-main">
                          <span className="cell-name">{source.name}</span>
                          <span className="cell-meta">{cadence(source.intervalMs)}</span>
                        </div>
                      </td>
                      <td className="muted">{source.accessorLabel}</td>
                      <td>
                        <NextRun at={source.nextAt} now={now} />
                      </td>
                      <td>
                        <YieldBar value={source.yields7d} max={maxYield} />
                      </td>
                      <td>
                        <StatusPill health={source.health} enabled={source.enabled} />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </Card>
        </div>

        <div className="overview-side">
          <Card
            title="需要留意"
            sub={attention.length > 0 ? `${attention.length} 个信源偏离节律` : "没有偏离节律的信源"}
            tight
          >
            {loading ? (
              <div style={{ padding: 16 }}>
                <Skeleton lines={3} />
              </div>
            ) : watch.length === 0 ? (
              <Empty title="一切安静" text="没有告警、暂停或连续失败的信源。下一轮巡检会继续盯着。" />
            ) : (
              <div>
                {watch.map((source) => (
                  <WatchItem key={source.id} source={source} now={now} />
                ))}
              </div>
            )}
          </Card>

          <Card title="最近提交" sub="每次发布都会生成可回放的快照" tight>
            {loading ? (
              <div style={{ padding: 16 }}>
                <Skeleton lines={3} />
              </div>
            ) : timeline.length === 0 ? (
              <Empty title="还没有提交" text="第一次采集成功后，这里会出现快照，可在底部游标回放。" />
            ) : (
              <div className="commit-list">
                {[...timeline]
                  .reverse()
                  .slice(0, 6)
                  .map((commit) => (
                    <div className="commit-row" key={commit.snapshot}>
                      <span className="commit-id">#{commit.snapshot}</span>
                      <span className="commit-summary">
                        {commit.commitIds.length} 个变更集写入内容库
                      </span>
                      <span className="commit-time">{relative(commit.publishedAt, now)}</span>
                    </div>
                  ))}
              </div>
            )}
          </Card>

          <Card title="采集进程" tight>
            <div style={{ padding: "12px 16px", display: "grid", gap: 8 }}>
              <ProcRow label="运维端口" value={snapshot ? `G-${snapshot.catalog.generation}` : "—"} />
              <ProcRow
                label="上次同步"
                value={lastCommit ? relative(lastCommit.publishedAt, now) : "刚刚"}
              />
              <p className="faint" style={{ fontSize: 11, lineHeight: 1.6 }}>
                控制台不直接读写数据目录，所有数值来自采集进程的运维面；进程停止时界面会明确提示，而不是显示旧值。
              </p>
            </div>
          </Card>
        </div>
      </div>
    </div>
  );
}

function WatchItem({ source, now }: { source: Source; now: number }) {
  const reason =
    explainError(source.lastError) ??
    (source.skipReason ? skipText(source.skipReason) : null) ??
    (source.health === "paused"
      ? "连续失败后暂停，修复后可手动触发一次"
      : source.health === "misconfigured"
        ? "配置无法生成采集计划"
        : "最近一次采集的产出低于预期");
  return (
    <button
      type="button"
      className="watch-item"
      onClick={() => navigate(`sources/${encodeURIComponent(source.id)}`)}
    >
      <span className={`watch-mark${source.health === "misconfigured" ? " is-bad" : ""}`} />
      <span className="watch-text">
        <span className="watch-name">{source.name}</span>
        <span className="watch-why">{reason}</span>
      </span>
      <span className="watch-when">{source.lastOkAt ? relative(source.lastOkAt, now) : "未成功"}</span>
    </button>
  );
}

function ProcRow({ label, value }: { label: string; value: string }) {
  return (
    <div style={{ display: "flex", justifyContent: "space-between", gap: 10, fontSize: 12 }}>
      <span className="faint">{label}</span>
      <span className="mono" style={{ color: "var(--ink-soft)" }}>
        {value}
      </span>
    </div>
  );
}

function skipText(reason: string): string {
  const table: Record<string, string> = {
    budget: "本次因预算让路，等待下一轮",
    paused: "健康状态为暂停，不再自动出发",
    misconfigured: "配置有误，任务无法生成",
    robots: "站点守则禁止抓取",
    deadline: "上次超出时间预算",
  };
  return table[reason] ?? `上次跳过：${reason}`;
}
