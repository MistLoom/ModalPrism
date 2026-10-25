// 采集计划：每个信源的节律、上限与启停，外加队列的暂停开关。
//
// 计划与队列是两件事：这里管「什么时候该派、派多少」，任务页管「一条任务走到哪里」。
import { useCallback, useEffect, useMemo, useState } from "react";

import { BatchBar } from "../components/batch";
import { IconPause, IconPlay, IconPlus, IconRefresh, IconSource } from "../components/icons";
import {
  Card,
  Empty,
  FilterSelect,
  NextRun,
  Pill,
  SectionTitle,
  Skeleton,
  StatusPill,
  YieldBar,
} from "../components/ui";
import { plannerManual, plannerRound, queuePause, queueStates, triggerSource } from "../lib/api";
import {
  batchDeclarations,
  declarations,
  toggleDeclaration,
  type BatchAction,
  type Declaration,
  type Declarations,
} from "../lib/catalog";
import { useConsole } from "../lib/console";
import { count } from "../lib/format";
import { navigate } from "../lib/router";
import { useSelection } from "../lib/selection";
import type { QueueState, Source } from "../lib/types";

/** 调度器可能的跳过原因。未知值原样展示，不让界面出现裸术语。 */
const SKIPS: Record<string, string> = {
  "daily-cap": "已到每日上限",
  "queue-paused": "队列已暂停",
  budget: "付费额度用尽",
};

interface Row {
  decl: Declaration;
  plan: Source | null;
}

export function PlansPage() {
  const { sources, state, now, notify, refresh, snapshot } = useConsole();
  const [list, setList] = useState<Declarations | null>(null);
  const [queues, setQueues] = useState<QueueState[] | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [toggling, setToggling] = useState<string | null>(null);
  const [running, setRunning] = useState<string | null>(null);
  const [pausing, setPausing] = useState<string | null>(null);
  const [dispatch, setDispatch] = useState(false);
  const [pick, setPick] = useState("");
  const [targetUrl, setTargetUrl] = useState("");
  const [runNow, setRunNow] = useState(true);
  const [creating, setCreating] = useState(false);
  const selection = useSelection<string>();
  const loading = state === "loading" || (list === null && failure === null);
  const generation = snapshot?.catalog.generation ?? null;

  const reload = useCallback(async () => {
    try {
      const [next, states] = await Promise.all([declarations(), queueStates()]);
      setList(next);
      setQueues(states);
      setFailure(null);
    } catch (problem) {
      setFailure(problem instanceof Error ? problem.message : "读取采集计划失败");
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload, generation]);

  const planById = useMemo(() => new Map(sources.map((source) => [source.id, source])), [sources]);

  const rows = useMemo<Row[]>(
    () =>
      (list?.sources ?? [])
        .map((decl) => ({ decl, plan: planById.get(decl.id) ?? null }))
        .sort((left, right) => due(left) - due(right) || left.decl.name.localeCompare(right.decl.name, "zh-Hans-CN")),
    [list, planById],
  );

  const maxYield = Math.max(1, ...sources.map((source) => source.todayYield));
  const selected = selection.list();
  const pausedQueues = (queues ?? []).filter((item) => item.paused).length;

  async function toggleRow(decl: Declaration) {
    setToggling(decl.slug);
    try {
      await toggleDeclaration("source", decl.slug, !decl.enabled);
      notify("ok", decl.enabled ? `已停用 ${decl.name}` : `已启用 ${decl.name}`, "采集进程约 2 秒内热加载。");
      refresh();
      await reload();
    } catch (problem) {
      notify("bad", "写入失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setToggling(null);
    }
  }

  async function runOne(decl: Declaration) {
    setRunning(decl.id);
    try {
      const round = await triggerSource(decl.id);
      notify(
        round.executed > 0 ? "ok" : "info",
        round.executed > 0 ? `${decl.name}：已执行 ${round.executed} 个任务` : `${decl.name}：已标记为到期`,
        round.leased > 0
          ? `本轮租约 ${round.leased} 条${round.failed > 0 ? `，失败 ${round.failed} 条` : ""}；结果会写入内容库。`
          : "没有到期的任务，调度器会在下一轮巡检时再看一次。",
      );
      refresh();
      await reload();
    } catch (problem) {
      notify("bad", "无法触发这次采集", problem instanceof Error ? problem.message : undefined);
    } finally {
      setRunning(null);
    }
  }

  async function runRound() {
    setDispatch(true);
    try {
      const round = await plannerRound();
      notify(
        round.executed.length > 0 ? "ok" : "info",
        `本轮执行 ${round.executed.length} 个任务`,
        `租约 ${round.leased} 条 · 跟进 ${round.follows} 条${round.failed.length > 0 ? ` · 失败 ${round.failed.length} 条` : ""}`,
      );
      refresh();
      await reload();
    } catch (problem) {
      // 长任务超时不是失败：派发已在采集进程内发起，任务会继续跑到终态。
      const message = problem instanceof Error ? problem.message : "";
      if (/timeout|超时|timed out/i.test(message)) {
        notify("info", "派发已发起，结果稍后刷新", "这一轮任务较多，后台仍在执行。可在任务页看到进度。");
        refresh();
        void reload();
      } else {
        notify("bad", "调度失败", message || undefined);
      }
    } finally {
      setDispatch(false);
    }
  }

  async function toggleQueue(row: QueueState) {
    setPausing(row.queue);
    try {
      const states = await queuePause(row.queue, !row.paused);
      setQueues(states);
      notify(row.paused ? "ok" : "warn", row.paused ? `${row.queue} 已恢复派发` : `${row.queue} 已暂停派发`, "暂停只影响新一轮派发，已在执行的任务照常完成。");
      refresh();
    } catch (problem) {
      notify("bad", "队列状态没有写入", problem instanceof Error ? problem.message : undefined);
    } finally {
      setPausing(null);
    }
  }

  async function batch(action: BatchAction) {
    if (selected.length === 0) {
      return;
    }
    setBusy(true);
    try {
      const result = await batchDeclarations("source", action, selected);
      const failed = result.results.filter((item) => !item.ok);
      notify(
        result.failed === 0 ? "ok" : "warn",
        `${action === "enable" ? "启用" : "停用"}完成：${result.total - result.failed} / ${result.total}`,
        failed.length ? failed.slice(0, 3).map((item) => `${item.slug}：${item.error ?? "失败"}`).join("；") : undefined,
      );
      selection.clear();
      refresh();
      await reload();
    } catch (problem) {
      notify("bad", "批量操作失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  async function create() {
    if (!pick) {
      return;
    }
    setCreating(true);
    try {
      const task = await plannerManual({ source: pick, runNow, locator: targetUrl.trim() || undefined });
      notify(
        "ok",
        runNow ? "已新建并执行一条任务" : "已新建一条任务",
        runNow ? `任务 ${task.task_id.slice(0, 10)} 的执行结果会写入内容库。` : "任务在队列里等待下一轮租约。",
      );
      refresh();
      await reload();
      navigate("tasks");
    } catch (problem) {
      notify("bad", "新建任务失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setCreating(false);
    }
  }

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <span className="page-kicker">plans · 采集计划</span>
          <h1 className="page-title">每信源的节律、上限与启停</h1>
          <p className="page-lede">
            计划决定「什么时候派、每轮派多少」，任务页决定「一条任务走到哪里」。这里的启停与上限立即写入配置目录，采集进程约 2 秒内热加载。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn" onClick={() => void reload()} disabled={loading}>
            <IconRefresh size={13} /> 刷新
          </button>
          <button type="button" className="btn btn-primary" onClick={() => void runRound()} disabled={dispatch}>
            <IconPlay size={13} /> {dispatch ? "派发中…" : "立即派发一轮"}
          </button>
        </div>
      </header>

      {failure ? (
        <div className="note is-bad">
          <strong>读取采集计划失败</strong>
          <span>{failure}</span>
        </div>
      ) : null}

      <Card
        title="队列"
        sub="暂停只影响新一轮派发，已在执行的任务照常完成"
        action={
          pausedQueues > 0 ? (
            <Pill tone="warn" dot>
              {pausedQueues} 个队列已暂停
            </Pill>
          ) : (
            <Pill tone="ok" dot>
              全部在派发
            </Pill>
          )
        }
      >
        {queues === null ? (
          <Skeleton lines={2} />
        ) : queues.length === 0 ? (
          <Empty title="还没有队列" text="调度器第一次派发任务后，这里会出现按通道分组的队列。" />
        ) : (
          <div className="queue-grid">
            {queues.map((row) => (
              <div className={`queue-card${row.paused ? " is-paused" : ""}`} key={row.queue}>
                <div className="queue-head">
                  <span className="queue-name mono">{row.queue}</span>
                  <button
                    type="button"
                    className="btn btn-sm btn-quiet"
                    disabled={pausing === row.queue}
                    onClick={() => void toggleQueue(row)}
                  >
                    {row.paused ? (
                      <>
                        <IconPlay size={12} /> 恢复
                      </>
                    ) : (
                      <>
                        <IconPause size={12} /> 暂停
                      </>
                    )}
                  </button>
                </div>
                <dl className="queue-specs">
                  <div>
                    <dt>排队</dt>
                    <dd>{count(row.queued)}</dd>
                  </div>
                  <div>
                    <dt>执行中</dt>
                    <dd>{count(row.leased)}</dd>
                  </div>
                  <div>
                    <dt>并发上限</dt>
                    <dd>{count(row.capacity)}</dd>
                  </div>
                  <div>
                    <dt>已完成</dt>
                    <dd>{count(row.done)}</dd>
                  </div>
                </dl>
              </div>
            ))}
          </div>
        )}
      </Card>

      <div className="row" style={{ marginTop: 14, flexWrap: "wrap", gap: 8 }}>
        <FilterSelect
          label="选择信源"
          value={pick}
          onChange={setPick}
          options={[
            { value: "", label: "选择信源…" },
            ...rows.map(({ decl }) => ({ value: decl.id, label: decl.name })),
          ]}
        />
        <label className="pick-inline">
          <input type="checkbox" checked={runNow} onChange={(event) => setRunNow(event.target.checked)} />
          建完立即执行
        </label>
        <input
          type="text"
          className="form-input mono"
          style={{ maxWidth: 320 }}
          value={targetUrl}
          onChange={(event) => setTargetUrl(event.target.value)}
          placeholder="目标地址（可选，留空=整源采集）"
          aria-label="目标地址"
        />
        <button type="button" className="btn btn-sm" disabled={!pick || creating} onClick={() => void create()}>
          <IconPlus size={12} /> {creating ? "正在新建…" : "新建采集任务"}
        </button>
        <span className="faint" style={{ fontSize: "var(--fs-xs)" }}>
          新建的任务在任务页可见，控制台不直接构造任务体。指定目标地址只对按地址取数的信源生效。
        </span>
      </div>

      <div style={{ marginTop: 18 }}>
        <SectionTitle title="计划" note={`${rows.length} 个信源`} />
        <BatchBar
          count={selection.size}
          busy={busy}
          onClear={selection.clear}
          actions={[
            { key: "enable", label: "启用", icon: <IconPlay size={12} />, onClick: () => void batch("enable") },
            { key: "disable", label: "停用", icon: <IconPause size={12} />, onClick: () => void batch("disable") },
          ]}
        />
        <Card tight>
          {loading ? (
            <div style={{ padding: 18 }}>
              <Skeleton lines={8} />
            </div>
          ) : rows.length === 0 ? (
            <Empty title="还没有采集计划" text="先在信源页新建一个信源，它的节律会出现在这里。" />
          ) : (
            <div className="table-scroll">
              <table className="table">
                <thead>
                  <tr>
                    <th className="pick-col">
                      <input
                        type="checkbox"
                        aria-label="全选"
                        checked={rows.length > 0 && selection.size === rows.length}
                        onChange={(event) => {
                          if (event.target.checked) {
                            rows.forEach(({ decl }) => {
                              if (!selection.has(decl.slug)) {
                                selection.toggle(decl.slug);
                              }
                            });
                          } else {
                            selection.clear();
                          }
                        }}
                      />
                    </th>
                    <th>信源</th>
                    <th>通道</th>
                    <th>解析器</th>
                    <th>节律</th>
                    <th>下次到期</th>
                    <th>今日产出与上限</th>
                    <th>健康</th>
                    <th>跳过原因</th>
                    <th className="align-right">操作</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map(({ decl, plan }) => (
                    <tr
                      key={decl.id}
                      className={`row-button${decl.enabled ? "" : " is-dim"}`}
                      onClick={() => navigate(`sources/${encodeURIComponent(decl.id)}`)}
                    >
                      <td className="pick-col" onClick={(event) => event.stopPropagation()}>
                        <input
                          type="checkbox"
                          aria-label={`选择 ${decl.name}`}
                          checked={selection.has(decl.slug)}
                          onChange={() => selection.toggle(decl.slug)}
                        />
                      </td>
                      <td>
                        <div className="source-name-cell">
                          <span className="source-title">{decl.name}</span>
                          <span className="cell-meta mono">{decl.id}</span>
                        </div>
                      </td>
                      <td className="mono" style={{ fontSize: "var(--fs-xs)" }}>
                        {short(decl.accessor) || "未指定"}
                      </td>
                      <td className="mono" style={{ fontSize: "var(--fs-xs)" }}>
                        {(decl.adapters ?? []).map(short).join(" · ") || "—"}
                      </td>
                      <td className="muted">{rhythm(decl)}</td>
                      <td>
                        {plan ? <NextRun at={plan.nextAt} now={now} /> : <span className="faint">未排期</span>}
                      </td>
                      <td>
                        <YieldBar value={plan?.todayYield ?? 0} max={maxYield} />
                        <span className="cell-meta">
                          上限 {plan?.dailyLimit ? count(plan.dailyLimit) : "不限"}
                        </span>
                      </td>
                      <td>
                        <StatusPill health={plan?.health ?? (decl.enabled ? "idle" : "paused")} enabled={decl.enabled} />
                      </td>
                      <td className="muted" style={{ fontSize: "var(--fs-xs)" }}>
                        {plan?.skipReason ? (SKIPS[plan.skipReason] ?? plan.skipReason) : "—"}
                      </td>
                      <td className="align-right" onClick={(event) => event.stopPropagation()}>
                        <span className="row-actions">
                          <button
                            type="button"
                            className="btn btn-sm btn-quiet"
                            disabled={toggling === decl.slug}
                            onClick={() => void toggleRow(decl)}
                          >
                            {decl.enabled ? (
                              <>
                                <IconPause size={12} /> 停用
                              </>
                            ) : (
                              <>
                                <IconPlay size={12} /> 启用
                              </>
                            )}
                          </button>
                          <button
                            type="button"
                            className="btn btn-sm btn-quiet"
                            disabled={running === decl.id || !decl.enabled}
                            onClick={() => void runOne(decl)}
                          >
                            {running === decl.id ? "派发中…" : "立即采集"}
                          </button>
                          <button
                            type="button"
                            className="btn btn-sm btn-quiet"
                            onClick={() => navigate(`tasks/${encodeURIComponent(decl.id)}`)}
                          >
                            任务
                          </button>
                          <button
                            type="button"
                            className="btn btn-sm btn-quiet"
                            onClick={() => navigate(`library/${encodeURIComponent(decl.id)}`)}
                          >
                            内容
                          </button>
                        </span>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </Card>
      </div>

      <p className="faint" style={{ fontSize: "var(--fs-xs)", lineHeight: 1.7 }}>
        <IconSource size={11} /> 节律、上限与启停都写回 config/sources；采集进程热加载后新计划立刻生效。今日产出按本地日期累计，达到每日上限后当天不再派发。
      </p>
    </div>
  );
}

function short(id: string | null | undefined): string {
  if (!id) {
    return "";
  }
  const index = id.indexOf("/");
  return index >= 0 ? id.slice(index + 1) : id;
}

/** 节律：间隔为主，带上下限与自适应标记。 */
function rhythm(decl: Declaration): string {
  const schedule = (decl.doc.schedule ?? {}) as Record<string, unknown>;
  const interval = typeof schedule.interval === "string" ? schedule.interval : decl.schedule;
  if (!interval) {
    return "未排期";
  }
  const parts = [interval];
  if (typeof schedule.floor === "string") {
    parts.push(`下限 ${schedule.floor}`);
  }
  if (typeof schedule.ceiling === "string") {
    parts.push(`上限 ${schedule.ceiling}`);
  }
  if (schedule.adaptive === true) {
    parts.push("自适应");
  }
  return parts.join(" · ");
}

function due(row: Row): number {
  if (!row.plan?.nextAt) {
    return Number.MAX_SAFE_INTEGER;
  }
  const at = Date.parse(row.plan.nextAt);
  return Number.isNaN(at) ? Number.MAX_SAFE_INTEGER : at;
}
