import { useEffect, useMemo, useState } from "react";

import { Drawer } from "../components/Drawer";
import { IconCopy, IconFile, IconPause, IconPlay, IconRefresh, IconSource } from "../components/icons";
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
import { explain, plannerNext, triggerSource } from "../lib/api";
import { toggleModuleEnabled } from "../lib/commands";
import { useConsole } from "../lib/console";
import { cadence, explainError, healthOf, relative, stamp } from "../lib/format";
import { navigate, useRoute } from "../lib/router";
import type { FieldOrigin, Source } from "../lib/types";

type Filter = "all" | "ok" | "attention" | "off";

export function SourcesPage() {
  const { sources, state, now, notify, refresh } = useConsole();
  const { param } = useRoute();
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [togglingId, setTogglingId] = useState<string | null>(null);
  const loading = state === "loading";

  const rows = useMemo(() => {
    const text = query.trim().toLowerCase();
    return sources.filter((source) => {
      if (filter === "ok" && (source.health !== "ok" || !source.enabled)) {
        return false;
      }
      if (filter === "attention" && !(source.alert || source.health !== "ok")) {
        return false;
      }
      if (filter === "off" && source.enabled) {
        return false;
      }
      if (!text) {
        return true;
      }
      return (
        source.name.toLowerCase().includes(text) ||
        source.id.toLowerCase().includes(text) ||
        source.accessorLabel.toLowerCase().includes(text) ||
        Object.values(source.labels).some((value) => value.toLowerCase().includes(text))
      );
    });
  }, [sources, query, filter]);

  const maxYield = Math.max(1, ...sources.map((source) => source.yields7d));

  const toggleRow = async (source: Source) => {
    setTogglingId(source.id);
    await toggleModuleEnabled(source, !source.enabled, "信源", notify, refresh);
    setTogglingId(null);
  };
  const selected = param ? sources.find((source) => source.id === param) ?? null : null;

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <span className="page-kicker">sources · 信源</span>
          <h1 className="page-title">信源清单与调度</h1>
          <p className="page-lede">
            每个信源对应一个站点或推送通道。点开任意一行查看采集节律、最近结果与配置字段来源；行内开关可直接暂停或恢复采集。
          </p>
        </div>
      </header>

      <div className="toolbar">
        <SearchInput
          value={query}
          onChange={setQuery}
          placeholder="按名称、标签或通道筛选…"
          label="筛选信源"
        />
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
        <span className="faint" style={{ fontSize: 11.5 }}>
          {loading ? "读取中" : `${rows.length} / ${sources.length} 个信源`}
        </span>
      </div>

      <Card tight>
        {loading ? (
          <div style={{ padding: 18 }}>
            <Skeleton lines={8} />
          </div>
        ) : rows.length === 0 ? (
          <Empty
            title={sources.length === 0 ? "还没有信源" : "没有符合条件的信源"}
            text={
              sources.length === 0
                ? "在 config/sources 下新增一个 YAML，保存后采集进程会在几秒内热加载。"
                : "换个关键词，或者把筛选切回「全部」。"
            }
          />
        ) : (
          <table className="table source-table">
            <thead>
              <tr>
                <th>信源</th>
                <th>通道</th>
                <th>节律</th>
                <th>下次</th>
                <th className="align-right">7 日产出</th>
                <th>状态</th>
                <th className="align-right">开关</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((source) => (
                <tr
                  key={source.id}
                  className="row-button"
                  onClick={() => navigate(`sources/${encodeURIComponent(source.id)}`)}
                >
                  <td>
                    <div className="source-name-cell">
                      <div className="source-title">
                        <span>{source.name}</span>
                        {source.labels.tier ? <span className="tag">{source.labels.tier}</span> : null}
                      </div>
                      <span className="cell-meta mono">{source.id}</span>
                    </div>
                  </td>
                  <td>
                    <div className="cell-main">
                      <span>{source.accessorLabel}</span>
                      <span className="cell-meta">{source.locator}</span>
                    </div>
                  </td>
                  <td className="muted">{cadence(source.intervalMs)}</td>
                  <td>
                    <NextRun at={source.nextAt} now={now} />
                  </td>
                  <td>
                    <YieldBar value={source.yields7d} max={maxYield} />
                  </td>
                  <td>
                    <StatusPill health={source.health} enabled={source.enabled} />
                  </td>
                  <td className="align-right" onClick={(event) => event.stopPropagation()}>
                    <button
                      type="button"
                      className="btn btn-sm btn-quiet"
                      disabled={togglingId === source.id}
                      onClick={() => void toggleRow(source)}
                    >
                      {togglingId === source.id ? (
                        "写入中…"
                      ) : source.enabled ? (
                        <>
                          <IconPause size={12} /> 暂停
                        </>
                      ) : (
                        <>
                          <IconPlay size={12} /> 恢复
                        </>
                      )}
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>

      <SourceDrawer source={selected} onClose={() => navigate("sources")} />
    </div>
  );
}

function SourceDrawer({ source, onClose }: { source: Source | null; onClose: () => void }) {
  const { notify, now, refresh } = useConsole();
  // 字段来源按信源 id 缓存：轮询会不断重建 source 对象，
  // 若直接依赖它，已展开的抽屉会反复回退到骨架屏造成闪烁。
  const [originState, setOriginState] = useState<{ id: string; fields: FieldOrigin[] } | null>(null);
  const [originError, setOriginError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [pausing, setPausing] = useState(false);
  const [scheduling, setScheduling] = useState(false);
  const open = Boolean(source);
  const sourceId = source?.id ?? null;
  const origins = originState && sourceId !== null && originState.id === sourceId ? originState.fields : null;

  useEffect(() => {
    if (!sourceId) {
      return;
    }
    let alive = true;
    // 已有同 id 缓存时静默刷新；首次打开才展示骨架。
    explain(sourceId)
      .then((fields) => {
        if (alive) {
          setOriginState({ id: sourceId, fields });
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
  }, [sourceId]);

  if (!source) {
    return null;
  }

  const health = healthOf(source.health, source.enabled);
  const reason = explainError(source.lastError);
  // run/togglePause/queryNext 读取的是当前轮的 source，行为不变。

  async function run() {
    if (!source) {
      return;
    }
    setPending(true);
    try {
      const tasks = await triggerSource(source.id);
      notify(
        "ok",
        tasks.length > 0 ? `已派出 ${tasks.length} 个任务` : "已标记为到期",
        tasks.length > 0 ? "任务会在本轮执行，结果写入内容库后生成快照。" : "调度器会在下一轮巡检时把它派出。",
      );
      refresh();
    } catch (problem) {
      notify(
        "bad",
        "无法触发这次采集",
        problem instanceof Error ? problem.message : "运维端口拒绝了这个操作",
      );
    } finally {
      setPending(false);
    }
  }

  /** 暂停/恢复：写回源 YAML 的顶层 enabled 开关，热加载后生效。 */
  async function togglePause() {
    if (!source) {
      return;
    }
    setPausing(true);
    await toggleModuleEnabled(source, !source.enabled, "信源", notify, refresh);
    setPausing(false);
  }

  /** 立即查询调度器，确认这一信源的下一轮出发时刻。 */
  async function queryNext() {
    if (!source) {
      return;
    }
    setScheduling(true);
    try {
      const next = await plannerNext(source.id);
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
      key={source.id}
      open={open}
      kicker={`信源 · ${source.accessorLabel}`}
      title={source.name}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn btn-primary" onClick={run} disabled={pending || !source.enabled}>
            <IconPlay size={12} />
            {pending ? "正在派发…" : "立即采集一次"}
          </button>
          <button
            type="button"
            className="btn btn-quiet"
            onClick={() => {
              void navigator.clipboard?.writeText(source.locator || source.id);
              notify("info", "已复制信源地址");
            }}
          >
            <IconCopy size={12} />
            复制地址
          </button>
          <span className="spacer" />
          <button type="button" className="btn btn-quiet btn-sm" onClick={refresh}>
            <IconRefresh size={12} />
            刷新
          </button>
        </>
      }
    >
      <div className="row">
        <StatusPill health={source.health} enabled={source.enabled} />
        <Pill tone="info">{cadence(source.intervalMs)}</Pill>
        {source.labels.tier ? <Pill>层级 {source.labels.tier}</Pill> : null}
        {Object.entries(source.labels)
          .filter(([key]) => key !== "tier")
          .map(([key, value]) => (
            <Pill key={key}>
              {key} · {value}
            </Pill>
          ))}
      </div>

      <p className="muted" style={{ fontSize: 12.5, lineHeight: 1.65 }}>
        {health.detail}
        {source.description ? ` ${source.description}` : ""}
      </p>

      <div>
        <SectionTitle title="节律与结果" note="时间以采集进程的时钟为准" />
        <div className="row" style={{ marginBottom: 10 }}>
          <button type="button" className="btn btn-sm" onClick={() => void togglePause()} disabled={pausing}>
            {source.enabled ? <IconPause size={13} /> : <IconPlay size={13} />}
            {pausing ? "写入中…" : source.enabled ? "暂停采集" : "恢复采集"}
          </button>
          <button type="button" className="btn btn-sm" onClick={() => void queryNext()} disabled={scheduling}>
            {scheduling ? "查询中…" : "查下一轮"}
          </button>
        </div>
        <Definitions
          items={[
            ["下次出发", <NextRun at={source.nextAt} now={now} />],
            ["上次成功", source.lastOkAt ? `${stamp(source.lastOkAt)} · ${relative(source.lastOkAt, now)}` : "还没有成功记录"],
            ["上次产出", source.lastYieldAt ? relative(source.lastYieldAt, now) : "—"],
            ["7 日产出", `${source.yields7d} 条`],
            ["访问地址", <span className="mono" style={{ fontSize: 11.5 }}>{source.locator || "—"}</span>],
            ["付费调用", source.budgetUsed > 0 ? `${source.budgetUsed} 次` : "未使用"],
          ]}
        />
      </div>

      {source.lastError || source.skipReason ? (
        <div className="note is-warn">
          <div style={{ display: "grid", gap: 2 }}>
            <strong>上次没有顺利完成</strong>
            <span>{reason ?? "这是采集进程返回的原始原因："}</span>
            {source.lastError ? (
              <span className="mono" style={{ fontSize: 11, opacity: 0.75, overflowWrap: "anywhere" }}>
                {source.lastError}
              </span>
            ) : null}
            {source.skipReason ? <span className="faint">跳过原因：{source.skipReason}</span> : null}
          </div>
        </div>
      ) : null}

      <div>
        <SectionTitle title="配置来源" note="每个生效值来自哪一行" />
        <div className="row" style={{ marginBottom: 8 }}>
          <button
            type="button"
            className="btn btn-sm btn-quiet"
            onClick={() => navigate(`config/${source.file}`)}
          >
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
                <span
                  className={`origin-source${field.viaPreset ? " is-preset" : ""}`}
                  title={`${field.file}:${field.line}`}
                >
                  {field.viaPreset ? `预设 ${field.presetId ?? ""} · ` : ""}
                  {shortFile(field.file)}:{field.line}
                </span>
              </div>
            ))}
          </div>
        )}
        <p className="faint" style={{ fontSize: 11, marginTop: 8 }}>
          <IconSource size={11} /> 标为蓝色的值来自预设；灰线位置是它在 YAML 里的定义处，改完保存后热加载。
        </p>
      </div>
    </Drawer>
  );
}

function shortFile(file: string): string {
  const parts = file.split(/[\\/]/);
  return parts.length <= 2 ? file : parts.slice(-2).join("/");
}
