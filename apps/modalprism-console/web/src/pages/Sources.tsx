import { useCallback, useEffect, useMemo, useState } from "react";

import { BatchBar } from "../components/batch";
import { DeclarationDrawer, TemplatePicker, type DeclarationSeed } from "../components/declaration";
import { Drawer } from "../components/Drawer";
import { IconCopy, IconFile, IconPause, IconPlay, IconPlus, IconRefresh, IconSource, IconTrash } from "../components/icons";
import {
  Card,
  Definitions,
  Empty,
  FieldName,
  NextRun,
  Pill,
  SearchInput,
  SectionTitle,
  Seg,
  Skeleton,
  StatusPill,
  YieldBar,
} from "../components/ui";
import { explain, healthReport, plannerNext, triggerSource } from "../lib/api";
import {
  batchDeclarations,
  declarations,
  locatorText,
  proxyOf,
  proxyLabel,
  toggleDeclaration,
  type BatchAction,
  type Declaration,
  type Declarations,
} from "../lib/catalog";
import { useConsole } from "../lib/console";
import { cadence, explainError, healthOf, relative, stamp } from "../lib/format";
import { navigate, useRoute } from "../lib/router";
import { useSelection } from "../lib/selection";
import type { FieldOrigin, HealthItem, HealthReport, Source } from "../lib/types";

type Filter = "all" | "ok" | "attention" | "off";

interface Row {
  decl: Declaration;
  /** 调度器给的状态；信源刚写入、还没被调度时为空。 */
  health: Source | null;
}

export function SourcesPage() {
  const { sources, state, now, notify, refresh, snapshot } = useConsole();
  const { param } = useRoute();
  const [list, setList] = useState<Declarations | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const selection = useSelection<string>();
  const [busy, setBusy] = useState(false);
  const [toggling, setToggling] = useState<string | null>(null);
  const [picking, setPicking] = useState(false);
  const [editor, setEditor] = useState<{ item: Declaration | null; seed: DeclarationSeed | null } | null>(null);
  const loading = state === "loading" || (list === null && failure === null);
  const generation = snapshot?.catalog.generation ?? null;

  const reload = useCallback(async () => {
    try {
      setList(await declarations());
      setFailure(null);
    } catch (problem) {
      setFailure(problem instanceof Error ? problem.message : "读取声明失败");
    }
  }, []);

  // 配置代际变化说明目录被重新加载过，声明列表跟着刷一遍。
  useEffect(() => {
    void reload();
  }, [reload, generation]);

  const healthById = useMemo(() => new Map(sources.map((source) => [source.id, source])), [sources]);

  const rows = useMemo<Row[]>(() => {
    const text = query.trim().toLowerCase();
    return (list?.sources ?? [])
      .map((decl) => ({ decl, health: healthById.get(decl.id) ?? null }))
      .filter((row) => {
        const health = row.health;
        if (filter === "ok" && (!row.decl.enabled || (health && health.health !== "ok"))) {
          return false;
        }
        if (filter === "attention" && !(health?.alert || (health && health.health !== "ok"))) {
          return false;
        }
        if (filter === "off" && row.decl.enabled) {
          return false;
        }
        if (!text) {
          return true;
        }
        return (
          row.decl.name.toLowerCase().includes(text) ||
          row.decl.id.toLowerCase().includes(text) ||
          (row.decl.accessor ?? "").toLowerCase().includes(text) ||
          locatorText(row.decl.doc).toLowerCase().includes(text)
        );
      })
      .sort((left, right) => rank(left) - rank(right) || left.decl.name.localeCompare(right.decl.name, "zh-Hans-CN"));
  }, [list, healthById, query, filter]);

  const maxYield = Math.max(1, ...sources.map((source) => source.yields7d));
  const picked = rows.filter((row) => selection.has(row.decl.slug)).length;
  const detail = param ? (list?.sources.find((item) => item.id === param) ?? null) : null;
  const detailHealth = detail ? (healthById.get(detail.id) ?? null) : null;

  function pickAll(on: boolean) {
    if (!on) {
      selection.clear();
      return;
    }
    rows.forEach((row) => {
      if (!selection.has(row.decl.slug)) {
        selection.toggle(row.decl.slug);
      }
    });
  }

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

  async function batch(action: BatchAction) {
    const slugs = selection.list();
    setBusy(true);
    try {
      const result = await batchDeclarations("source", action, slugs);
      const verb = action === "remove" ? "删除" : action === "enable" ? "启用" : "停用";
      const failed = result.results.filter((item) => !item.ok);
      notify(
        result.failed === 0 ? "ok" : "warn",
        `${verb}完成：${result.total - result.failed} / ${result.total}`,
        failed.length
          ? `未完成：${failed
              .slice(0, 3)
              .map((item) => `${item.slug}（${item.error ?? "失败"}）`)
              .join("；")}`
          : "改动已写回配置目录。",
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

  function openEdit(decl: Declaration) {
    navigate("sources");
    setEditor({ item: decl, seed: null });
  }

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <span className="page-kicker">sources · 信源</span>
          <h1 className="page-title">信源清单与调度</h1>
          <p className="page-lede">
            每个信源对应一个站点或推送通道。用模板新建、批量启停或删除，点开任意一行查看采集节律、最近结果与配置来源。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn" onClick={() => void reload()} disabled={loading}>
            <IconRefresh size={13} /> 刷新
          </button>
          <button type="button" className="btn btn-primary" onClick={() => setPicking(true)}>
            <IconPlus size={13} /> 新建信源
          </button>
        </div>
      </header>

      <div className="toolbar">
        <SearchInput value={query} onChange={setQuery} placeholder="按名称、通道或地址筛选…" label="筛选信源" />
        <Seg
          value={filter}
          onChange={setFilter}
          label="信源状态"
          options={[
            { value: "all", label: "全部" },
            { value: "ok", label: "正常" },
            { value: "attention", label: "需要留意" },
            { value: "off", label: "已停用" },
          ]}
        />
        <span className="spacer" />
        <span className="faint">
          {loading ? "读取中" : `${rows.length} / ${list?.sources.length ?? 0} 个信源`}
        </span>
      </div>

      <BatchBar
        count={picked}
        busy={busy}
        onClear={selection.clear}
        actions={[
          { key: "enable", label: "启用", icon: <IconPlay size={12} />, onClick: () => void batch("enable") },
          { key: "disable", label: "停用", icon: <IconPause size={12} />, onClick: () => void batch("disable") },
          {
            key: "remove",
            label: "删除",
            icon: <IconTrash size={12} />,
            danger: true,
            confirm: `删除选中的 ${picked} 个信源？被引用的声明会被校验拦下并自动恢复。`,
            onClick: () => void batch("remove"),
          },
        ]}
      />

      {failure ? (
        <div className="note is-bad">
          <strong>读取声明失败</strong>
          <span>{failure}</span>
        </div>
      ) : null}

      <Card tight>
        {loading ? (
          <div style={{ padding: 18 }}>
            <Skeleton lines={8} />
          </div>
        ) : rows.length === 0 ? (
          <Empty
            title={list && list.sources.length === 0 ? "还没有信源" : "没有符合条件的信源"}
            text={
              list && list.sources.length === 0
                ? "点右上角「新建信源」，从 RSS、网页列表或搜索查询起步，保存后几秒内热加载。"
                : "换个关键词，或者把筛选切回「全部」。"
            }
          />
        ) : (
          <table className="table source-table">
            <thead>
              <tr>
                <th className="pick-col">
                  <input
                    type="checkbox"
                    aria-label="全选"
                    checked={rows.length > 0 && picked === rows.length}
                    onChange={(event) => pickAll(event.target.checked)}
                  />
                </th>
                <th>信源</th>
                <th>通道</th>
                <th>节律</th>
                <th>下次</th>
                <th className="align-right">7 日产出</th>
                <th>状态</th>
                <th className="align-right">操作</th>
              </tr>
            </thead>
            <tbody>
              {rows.map(({ decl, health }) => {
                const proxy = proxyOf(decl.doc);
                return (
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
                        <div className="source-title">
                          <span>{decl.name}</span>
                          {proxy ? <span className="tag">代理 · {proxyLabel(proxy)}</span> : null}
                        </div>
                        <span className="cell-meta mono">{decl.id}</span>
                      </div>
                    </td>
                    <td>
                      <div className="cell-main">
                        <span>{shortId(decl.accessor)}</span>
                        <span className="cell-meta">{locatorText(decl.doc) || "—"}</span>
                      </div>
                    </td>
                    <td className="muted">
                      {health ? cadence(health.intervalMs) : decl.schedule ?? "未排期"}
                    </td>
                    <td>{health ? <NextRun at={health.nextAt} now={now} /> : <span className="faint">未排期</span>}</td>
                    <td>
                      <YieldBar value={health?.yields7d ?? 0} max={maxYield} />
                    </td>
                    <td>
                      <StatusPill health={health?.health ?? (decl.enabled ? "idle" : "paused")} enabled={decl.enabled} />
                    </td>
                    <td className="align-right" onClick={(event) => event.stopPropagation()}>
                      <span className="row-actions">
                        <button type="button" className="btn btn-sm btn-quiet" onClick={() => openEdit(decl)}>
                          编辑
                        </button>
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
                      </span>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </Card>

      <SourceDrawer
        decl={detail}
        health={detailHealth}
        onClose={() => navigate("sources")}
        onEdit={detail ? () => openEdit(detail) : undefined}
      />

      {picking ? (
        <TemplatePicker
          kind="source"
          onClose={() => setPicking(false)}
          onPick={(seed) => {
            setPicking(false);
            setEditor({ item: null, seed });
          }}
        />
      ) : null}

      {editor ? (
        <DeclarationDrawer
          key={editor.item?.id ?? `new-${editor.seed?.slug ?? "blank"}`}
          kind="source"
          item={editor.item}
          seed={editor.seed}
          accessors={list?.accessors ?? []}
          onClose={() => setEditor(null)}
          onSaved={(message) => {
            notify("ok", message, "采集进程约 2 秒内热加载；代际变化后这一页会自动刷新。");
            refresh();
            void reload();
          }}
        />
      ) : null}

      <p className="faint">
        <IconSource size={11} /> 声明写在 config/sources 下；这里的新建、启停与删除都走同一条校验路径，失败会恢复原文。
      </p>
    </div>
  );
}

function SourceDrawer({
  decl,
  health,
  onClose,
  onEdit,
}: {
  decl: Declaration | null;
  health: Source | null;
  onClose: () => void;
  onEdit?: () => void;
}) {
  const { notify, now, refresh } = useConsole();
  // 字段来源按信源 id 缓存：轮询会不断重建对象，
  // 若直接依赖它，已展开的抽屉会反复回退到骨架屏造成闪烁。
  const [originState, setOriginState] = useState<{ id: string; fields: FieldOrigin[] } | null>(null);
  const [originError, setOriginError] = useState<string | null>(null);
  const [runtime, setRuntime] = useState<HealthReport | null>(null);
  const [pending, setPending] = useState(false);
  const [scheduling, setScheduling] = useState(false);
  const id = decl?.id ?? null;
  const origins = originState && id !== null && originState.id === id ? originState.fields : null;

  useEffect(() => {
    if (!id) {
      return;
    }
    let alive = true;
    explain(id)
      .then((fields) => {
        if (alive) {
          setOriginState({ id, fields });
          setOriginError(null);
        }
      })
      .catch((error: Error) => {
        if (alive) {
          setOriginError(error.message);
        }
      });
    return () => {
      alive = false;
    };
  }, [id]);

  useEffect(() => {
    if (!id) {
      return;
    }
    let alive = true;
    healthReport()
      .then((report) => {
        if (alive) {
          setRuntime(report);
        }
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [id]);

  if (!decl) {
    return null;
  }

  const enabled = decl.enabled;
  const state = health?.health ?? (enabled ? "idle" : "paused");
  const detail = healthOf(state, enabled);
  const reason = explainError(health?.lastError ?? null);
  // 只看这条信源真正引用的通道与解析器；它们的运行态写在 health state 里。
  const referenced = decl.adapters ?? [];
  const runtimeRows: { kind: string; item: HealthItem }[] = runtime
    ? [
        ...runtime.accessors
          .filter((item) => shortId(decl.accessor) === item.id)
          .map((item) => ({ kind: "通道", item })),
        ...runtime.adapters
          .filter((item) => referenced.some((ref) => shortId(ref) === item.id))
          .map((item) => ({ kind: "解析器", item })),
      ]
    : [];

  async function run() {
    if (!decl) {
      return;
    }
    setPending(true);
    try {
      const round = await triggerSource(decl.id);
      notify(
        round.executed > 0 ? "ok" : "info",
        round.executed > 0 ? `已执行 ${round.executed} 个任务` : "已标记为到期",
        round.leased > 0
          ? `租约 ${round.leased} 条${round.failed > 0 ? `，失败 ${round.failed} 条` : ""}；结果写入内容库后才生成快照。`
          : "没有到期的任务，调度器会在下一轮巡检时再看一次。",
      );
      refresh();
    } catch (problem) {
      notify("bad", "无法触发这次采集", problem instanceof Error ? problem.message : "运维端口拒绝了这个操作");
    } finally {
      setPending(false);
    }
  }

  /** 立即查询调度器，确认这一信源的下一轮出发时刻。 */
  async function queryNext() {
    if (!decl) {
      return;
    }
    setScheduling(true);
    try {
      const next = await plannerNext(decl.id);
      notify(
        "info",
        next.nextAt ? `下一轮：${stamp(next.nextAt)}` : "没有安排下一轮",
        next.nextAt
          ? `${relative(next.nextAt, now)} · 节律 ${cadence(next.intervalMs)}${next.skipReason ? ` · 跳过原因：${next.skipReason}` : ""}`
          : next.skipReason ?? "稍后再试，或点「立即采集一次」。",
      );
    } catch (problem) {
      notify("bad", "查询失败", problem instanceof Error ? problem.message : "调度器没有响应");
    } finally {
      setScheduling(false);
    }
  }

  return (
    <Drawer
      key={decl.id}
      open
      kicker={`信源 · ${shortId(decl.accessor)}`}
      title={decl.name}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn btn-primary" onClick={run} disabled={pending || !enabled}>
            <IconPlay size={12} />
            {pending ? "正在派发…" : "立即采集一次"}
          </button>
          <button type="button" className="btn btn-quiet" onClick={onEdit}>
            编辑声明
          </button>
          <span className="spacer" />
          <button type="button" className="btn btn-quiet btn-sm" onClick={refresh}>
            <IconRefresh size={12} /> 刷新
          </button>
        </>
      }
    >
      <div className="row">
        <StatusPill health={state} enabled={enabled} />
        <Pill tone="info">{health ? cadence(health.intervalMs) : decl.schedule ?? "未排期"}</Pill>
        {proxyOf(decl.doc) ? <Pill tone="warn">代理 · {proxyLabel(proxyOf(decl.doc))}</Pill> : null}
      </div>

      <p className="muted" style={{ fontSize: "var(--fs-sm)", lineHeight: 1.65 }}>
        {detail.detail}
        {decl.description ? ` ${decl.description}` : ""}
      </p>

      <div>
        <SectionTitle title="节律与结果" note="时间以采集进程的时钟为准" />
        <div className="row" style={{ marginBottom: 10 }}>
          <button type="button" className="btn btn-sm" onClick={() => void queryNext()} disabled={scheduling}>
            {scheduling ? "查询中…" : "查下一轮"}
          </button>
          <button
            type="button"
            className="btn btn-sm btn-quiet"
            onClick={() => {
              void navigator.clipboard?.writeText(locatorText(decl.doc) || decl.id);
              notify("info", "已复制信源地址");
            }}
          >
            <IconCopy size={12} /> 复制地址
          </button>
        </div>
        <Definitions
          items={[
            ["下次出发", <NextRun at={health?.nextAt ?? null} now={now} />],
            [
              "上次成功",
              health?.lastOkAt ? `${stamp(health.lastOkAt)} · ${relative(health.lastOkAt, now)}` : "还没有成功记录",
            ],
            ["上次产出", health?.lastYieldAt ? relative(health.lastYieldAt, now) : "—"],
            ["7 日产出", `${health?.yields7d ?? 0} 条`],
            ["访问地址", <span className="mono" style={{ fontSize: "var(--fs-xs)" }}>{locatorText(decl.doc) || "—"}</span>],
            ["付费调用", health && health.budgetUsed > 0 ? `${health.budgetUsed} 次` : "未使用"],
          ]}
        />
      </div>

      {health?.lastError || health?.skipReason ? (
        <div className="note is-warn">
          <div style={{ display: "grid", gap: 2 }}>
            <strong>上次没有顺利完成</strong>
            <span>{reason ?? "这是采集进程返回的原始原因："}</span>
            {health?.lastError ? (
              <span className="mono" style={{ fontSize: "var(--fs-2xs)", opacity: 0.75, overflowWrap: "anywhere" }}>
                {health.lastError}
              </span>
            ) : null}
            {health?.skipReason ? <span className="faint">跳过原因：{health.skipReason}</span> : null}
          </div>
        </div>
      ) : null}

      <div>
        <SectionTitle title="通道与解析器健康" note="采集进程内部的运行态" />
        {runtime === null ? (
          <Skeleton lines={2} />
        ) : runtimeRows.length === 0 ? (
          <Empty
            title="还没有运行记录"
            text="这条信源引用到的通道与解析器跑过一次之后，这里会出现状态。"
          />
        ) : (
          <table className="table">
            <thead>
              <tr>
                <th>对象</th>
                <th>类型</th>
                <th>状态</th>
                <th>最近成功</th>
                <th className="align-right">拦截 / 选择器未命中</th>
              </tr>
            </thead>
            <tbody>
              {runtimeRows.map(({ kind, item }) => (
                <tr key={`${kind}-${item.id}`}>
                  <td className="mono" style={{ fontSize: "var(--fs-xs)" }}>
                    {item.id}
                  </td>
                  <td className="muted">{kind}</td>
                  <td>
                    <StatusPill health={item.status} />
                  </td>
                  <td className="muted">{relative(item.lastOkAt, now)}</td>
                  <td className="align-right mono faint">
                    {item.blockedHits} / {item.selectorMiss}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {runtimeRows.some((row) => row.item.lastError) ? (
          <p className="faint" style={{ fontSize: "var(--fs-2xs)", lineHeight: 1.6, marginTop: 6 }}>
            {runtimeRows
              .filter((row) => row.item.lastError)
              .map((row) => `${row.item.id}：${row.item.lastError}`)
              .slice(0, 3)
              .join("；")}
          </p>
        ) : null}
      </div>

      <div>
        <SectionTitle title="配置来源" note="每个生效值来自哪一行" />
        <div className="row" style={{ marginBottom: 8 }}>
          <button type="button" className="btn btn-sm btn-quiet" onClick={() => navigate(`config/${decl.file}`)}>
            <IconFile size={13} /> 在配置页打开源文件
          </button>
        </div>
        {originError ? (
          <div className="note is-warn">无法读取字段来源：{originError}</div>
        ) : origins === null ? (
          <Skeleton lines={4} />
        ) : origins.length === 0 ? (
          <Empty title="没有可解释的字段" />
        ) : (
          <div className="origin-list">
            {origins.slice(0, 40).map((field) => (
              <div className="origin-row" key={field.path}>
                <FieldName path={field.path} />
                <span className={`origin-source${field.viaPreset ? " is-preset" : ""}`} title={`${field.file}:${field.line}`}>
                  {field.viaPreset ? `预设 ${field.presetId ?? ""} · ` : ""}
                  {shortFile(field.file)}:{field.line}
                </span>
              </div>
            ))}
          </div>
        )}
        <p className="faint" style={{ fontSize: "var(--fs-2xs)", marginTop: 8 }}>
          <IconSource size={11} /> 标为蓝色的值来自预设；灰线位置是它在 YAML 里的定义处，改完保存后热加载。
        </p>
      </div>
    </Drawer>
  );
}

function rank(row: Row): number {
  const health = row.health?.health;
  if (!row.decl.enabled) {
    return 6;
  }
  if (!health) {
    return 4;
  }
  if (health === "error" || health === "misconfigured") {
    return 0;
  }
  if (health === "warn" || health === "alert") {
    return 1;
  }
  if (health === "ok") {
    return 5;
  }
  return 3;
}

function shortId(id: string | null | undefined): string {
  if (!id) {
    return "未指定";
  }
  const index = id.indexOf("/");
  return index >= 0 ? id.slice(index + 1) : id;
}

function shortFile(file: string): string {
  const parts = file.split(/[\\/]/);
  return parts.length <= 2 ? file : parts.slice(-2).join("/");
}
