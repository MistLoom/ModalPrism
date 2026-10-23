// 任务页：任务队列状态、重试与调度。
import { useCallback, useEffect, useMemo, useState } from "react";

import { IconQueue, IconRefresh } from "../components/icons";
import {
  Card,
  Empty,
  FilterSelect,
  Pagination,
  Pill,
  SearchInput,
  Seg,
  Skeleton,
} from "../components/ui";
import { plannerTick, taskList, taskRetry } from "../lib/api";
import { useConsole } from "../lib/console";
import { relative } from "../lib/format";
import type { TaskRow } from "../lib/types";

/** 一次拉取的任务上限；前端再分页与筛选。 */
const FETCH_LIMIT = 500;
/** 每页展示的任务数。 */
const PAGE_SIZE = 50;

type Filter = "all" | "queued" | "leased" | "done";

const REASONS: Record<string, string> = {
  schedule: "调度",
  follow: "跟进",
  render_upgrade: "渲染升级",
  detail: "详情",
  manual: "手动",
  replay: "回放",
};

function statusTone(status: TaskRow["status"]): "ok" | "info" | "idle" {
  if (status === "leased") {
    return "ok";
  }
  if (status === "queued") {
    return "info";
  }
  return "idle";
}

function statusLabel(status: TaskRow["status"]): string {
  if (status === "leased") {
    return "执行中";
  }
  if (status === "queued") {
    return "排队中";
  }
  return "已完成";
}

function locatorOf(row: TaskRow): string {
  const url = row.locator?.url;
  return typeof url === "string" ? url : "—";
}

/** 关键字匹配：任务号、信源、目标地址、错误信息与来源说明。 */
function matches(row: TaskRow, needle: string): boolean {
  if (!needle) {
    return true;
  }
  const url = row.locator?.url;
  return (
    row.task_id.toLowerCase().includes(needle) ||
    row.source_id.toLowerCase().includes(needle) ||
    (typeof url === "string" && url.toLowerCase().includes(needle)) ||
    (row.last_error ?? "").toLowerCase().includes(needle) ||
    (REASONS[row.reason] ?? row.reason).toLowerCase().includes(needle)
  );
}

export function TasksPage() {
  const { notify } = useConsole();
  const [rows, setRows] = useState<TaskRow[] | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [source, setSource] = useState("all");
  const [text, setText] = useState("");
  const [needle, setNeedle] = useState("");
  const [page, setPage] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [ticking, setTicking] = useState(false);

  const load = useCallback(async () => {
    try {
      setRows(await taskList({ limit: FETCH_LIMIT }));
      setError(null);
    } catch (problem) {
      setError(problem instanceof Error ? problem.message : "读取任务队列失败");
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // 关键字防抖，与内容库一致；输入变化时回到第一页。
  useEffect(() => {
    const timer = window.setTimeout(() => {
      setNeedle(text.trim().toLowerCase());
      setPage(0);
    }, 320);
    return () => window.clearTimeout(timer);
  }, [text]);

  const tick = async () => {
    setTicking(true);
    try {
      const tasks = await plannerTick();
      notify("ok", `本轮调度派出了 ${tasks.length} 个任务`, "到期信源已进入队列，执行结果会写入内容库。");
      await load();
    } catch (problem) {
      notify("bad", "调度失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setTicking(false);
    }
  };

  const retry = async (taskId: string) => {
    setBusyId(taskId);
    try {
      const ok = await taskRetry(taskId);
      notify(
        ok ? "ok" : "info",
        ok ? "已恢复为排队状态" : "该任务不处于排队状态",
        ok ? "调度器下一轮租约时会执行它。" : "只有排队中的任务可以重试。",
      );
      await load();
    } catch (problem) {
      notify("bad", "重试失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusyId(null);
    }
  };

  const matched = (rows ?? []).filter(
    (row) =>
      (filter === "all" || row.status === filter) &&
      (source === "all" || row.source_id === source) &&
      matches(row, needle),
  );
  const pageCount = Math.max(1, Math.ceil(matched.length / PAGE_SIZE));
  const safePage = Math.min(page, pageCount - 1);
  const visible = matched.slice(safePage * PAGE_SIZE, (safePage + 1) * PAGE_SIZE);
  const queued = rows?.filter((row) => row.status === "queued").length ?? 0;
  const leased = rows?.filter((row) => row.status === "leased").length ?? 0;
  const done = rows?.filter((row) => row.status === "done").length ?? 0;
  const waiting = rows?.filter((row) => row.status === "queued" && row.retry_at !== null).length ?? 0;

  const sourceOptions = useMemo(() => {
    const ids = [...new Set((rows ?? []).map((row) => row.source_id))].sort();
    return [{ value: "all", label: "全部来源" }, ...ids.map((id) => ({ value: id, label: id }))];
  }, [rows]);

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <span className="page-kicker">tasks · 任务队列</span>
          <h1 className="page-title">任务队列与调度</h1>
          <p className="page-lede">
            调度器把到期信源的任务放入队列，runner 按租约执行。这里展示最近的 {FETCH_LIMIT}
            条任务，可按来源、关键字与状态筛选；排队中的任务可以清除等待时间，立即参与下一轮租约。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn" onClick={() => void load()}>
            <IconRefresh size={13} /> 刷新
          </button>
          <button type="button" className="btn btn-primary" onClick={() => void tick()} disabled={ticking}>
            <IconQueue size={13} /> {ticking ? "调度中…" : "立即调度一轮"}
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
          <span className="stat-value">{rows === null ? "—" : queued}</span>
          <span className="stat-foot">其中 {waiting} 个在等待重试时间</span>
        </div>
        <div className="stat">
          <span className="stat-label">执行中</span>
          <span className="stat-value">{rows === null ? "—" : leased}</span>
          <span className="stat-foot">runner 持有租约</span>
        </div>
        <div className="stat">
          <span className="stat-label">已完成</span>
          <span className="stat-value">{rows === null ? "—" : done}</span>
          <span className="stat-foot">最近 {FETCH_LIMIT} 条记录内</span>
        </div>
        <div className="stat">
          <span className="stat-label">当前筛选</span>
          <span className="stat-value">{rows === null ? "—" : matched.length}</span>
          <span className="stat-foot">
            {filter === "all" ? "全部状态" : statusLabel(filter as TaskRow["status"])} · 第 {safePage + 1} / {pageCount} 页
          </span>
        </div>
      </div>

      <div className="toolbar">
        <SearchInput
          value={text}
          onChange={setText}
          placeholder="按任务号、信源、地址或错误筛选…"
          label="筛选任务"
        />
        <FilterSelect
          label="来源"
          value={source}
          onChange={(value) => {
            setSource(value);
            setPage(0);
          }}
          options={sourceOptions}
        />
        <span className="spacer" />
        <span className="faint" style={{ fontSize: 11.5 }}>
          {rows === null ? "读取中" : `${matched.length} / ${rows.length} 条`}
        </span>
      </div>

      <Card
        title="任务列表"
        sub="按创建时间倒序"
        action={
          <Seg
            value={filter}
            onChange={(value) => {
              setFilter(value);
              setPage(0);
            }}
            label="任务状态"
            options={[
              { value: "all", label: "全部" },
              { value: "queued", label: "排队" },
              { value: "leased", label: "执行中" },
              { value: "done", label: "已完成" },
            ]}
          />
        }
        tight
      >
        {rows === null ? (
          <Skeleton lines={6} />
        ) : visible.length === 0 ? (
          <Empty
            title="没有符合筛选的任务"
            text={rows.length === 0 ? "调度器派出任务后，这里会出现记录。" : "切换到「全部」查看其他状态。"}
          />
        ) : (
          <div className="table-scroll">
            <table className="table">
              <thead>
                <tr>
                  <th>任务</th>
                  <th>队列</th>
                  <th>信源</th>
                  <th>目标</th>
                  <th>状态</th>
                  <th className="align-right">尝试</th>
                  <th>创建</th>
                  <th className="align-right">操作</th>
                </tr>
              </thead>
              <tbody>
                {visible.map((row) => (
                  <tr key={row.task_id}>
                    <td>
                      <span className="mono" style={{ fontSize: 11 }}>
                        {row.task_id.slice(0, 10)}
                      </span>
                      {row.last_error ? (
                        <div className="faint" style={{ fontSize: 10.5, maxWidth: 260, overflowWrap: "anywhere" }}>
                          {row.last_error}
                        </div>
                      ) : null}
                    </td>
                    <td className="mono">{row.queue}</td>
                    <td>
                      <span className="mono" style={{ fontSize: 11 }}>
                        {row.source_id}
                      </span>
                    </td>
                    <td>
                      <span className="cell-meta" style={{ fontSize: 11 }} title={locatorOf(row)}>
                        {locatorOf(row)}
                      </span>
                    </td>
                    <td>
                      <Pill tone={statusTone(row.status)} dot>
                        {statusLabel(row.status)}
                      </Pill>
                      {row.status === "queued" && row.retry_at ? (
                        <div className="faint" style={{ fontSize: 10.5 }}>
                          {relative(row.retry_at)}可租约
                        </div>
                      ) : null}
                    </td>
                    <td className="num align-right">{row.attempt}</td>
                    <td className="muted">{relative(row.created_at)}</td>
                    <td className="align-right">
                      {row.status === "queued" ? (
                        <button
                          type="button"
                          className="btn btn-sm"
                          disabled={busyId === row.task_id}
                          onClick={() => void retry(row.task_id)}
                        >
                          {busyId === row.task_id ? "处理中…" : "重试"}
                        </button>
                      ) : (
                        <span className="faint">{REASONS[row.reason] ?? row.reason}</span>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Card>

      {matched.length > PAGE_SIZE ? (
        <Pagination
          page={safePage}
          hasPrev={safePage > 0}
          hasNext={safePage < pageCount - 1}
          onChange={setPage}
        />
      ) : null}

      <p className="faint" style={{ fontSize: 11.5, lineHeight: 1.7 }}>
        任务按信源与目标去重：同一信源指向同一地址的活跃任务只有一个。「重试」只作用于排队状态的任务；执行中的任务由租约到期后自动回收，已完成的任务不参与重试。
      </p>
    </div>
  );
}
