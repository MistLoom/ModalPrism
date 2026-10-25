import { useCallback, useEffect, useState } from "react";

import { BatchBar } from "../components/batch";
import { DeclarationDrawer, ProxyInput, TemplatePicker, type DeclarationSeed } from "../components/declaration";
import { IconAlert, IconPause, IconPlay, IconPlus, IconRefresh, IconShield } from "../components/icons";
import { Card, Empty, Pill, SectionTitle, Skeleton } from "../components/ui";
import { egressCheck, egressReport, egressStatus, healthReport, implementationLabel } from "../lib/api";
import {
  batchDeclarations,
  declarations,
  proxyLabel,
  proxyMode,
  proxyOf,
  setGlobalProxy,
  toggleDeclaration,
  type BatchAction,
  type Declaration,
  type Declarations,
} from "../lib/catalog";
import { useConsole } from "../lib/console";
import { healthOf, profileLabel, relative } from "../lib/format";
import { useSelection } from "../lib/selection";
import type { EgressCheck, EgressReport, EgressStatus, HealthItem, HealthReport } from "../lib/types";

type Editor = { kind: "accessor" | "adapter"; item: Declaration | null; seed: DeclarationSeed | null };

export function ChannelsPage() {
  const { snapshot, state, now, notify, refresh } = useConsole();
  const [list, setList] = useState<Declarations | null>(null);
  const [status, setStatus] = useState<EgressStatus | null>(null);
  const [report, setReport] = useState<EgressReport | null>(null);
  const [runtime, setRuntime] = useState<HealthReport | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [editor, setEditor] = useState<Editor | null>(null);
  const [picking, setPicking] = useState<"accessor" | "adapter" | null>(null);
  const [busy, setBusy] = useState(false);
  const [toggling, setToggling] = useState<string | null>(null);
  const channelPick = useSelection<string>();
  const adapterPick = useSelection<string>();
  const loading = state === "loading" || (list === null && failure === null);
  const generation = snapshot?.catalog.generation ?? null;

  const reload = useCallback(async () => {
    try {
      const [next, egress, log] = await Promise.all([declarations(), egressStatus(), egressReport()]);
      setList(next);
      setStatus(egress);
      setReport(log);
      setFailure(null);
    } catch (problem) {
      setFailure(problem instanceof Error ? problem.message : "读取通道状态失败");
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload, generation]);

  // 运行态自己跑一条 30 秒的心跳：通道状态变了，但配置代际没变。
  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const next = await healthReport();
        if (alive) {
          setRuntime(next);
        }
      } catch {
        // 运行态读不到不影响通道管理。
      }
    };
    void tick();
    const timer = window.setInterval(() => void tick(), 30_000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, []);

  const accessors = list?.accessors ?? [];
  const adapters = list?.adapters ?? [];
  const runtimeRows = [...(runtime?.accessors ?? []), ...(runtime?.adapters ?? [])];
  const abnormal = runtimeRows.filter((item) => item.alert || item.status !== "ok").length;

  async function toggleRow(kind: "accessor" | "adapter", item: Declaration) {
    setToggling(`${kind}:${item.slug}`);
    try {
      await toggleDeclaration(kind, item.slug, !item.enabled);
      notify("ok", item.enabled ? `已停用 ${item.name}` : `已启用 ${item.name}`, "采集进程约 2 秒内热加载。");
      refresh();
      await reload();
    } catch (problem) {
      notify("bad", "写入失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setToggling(null);
    }
  }

  async function batch(kind: "accessor" | "adapter", action: BatchAction, slugs: string[]) {
    let targets = slugs;
    if (action === "remove") {
      const removable = new Set(
        (kind === "adapter" ? adapters : accessors).filter((item) => item.removable !== false).map((item) => item.slug),
      );
      const skipped = slugs.filter((slug) => !removable.has(slug));
      targets = slugs.filter((slug) => removable.has(slug));
      if (skipped.length > 0) {
        notify("info", `跳过 ${skipped.length} 份内置声明`, `内置适配器不可删除：${skipped.join("、")}。可以停用或改写。`);
      }
    }
    if (targets.length === 0) {
      return;
    }
    if (action === "remove" && !window.confirm(`删除选中的 ${targets.length} 份${kind === "adapter" ? "解析器" : "通道"}？`)) {
      return;
    }
    setBusy(true);
    try {
      const result = await batchDeclarations(kind, action, targets);
      const verb = action === "remove" ? "删除" : action === "enable" ? "启用" : "停用";
      const failed = result.results.filter((item) => !item.ok);
      notify(
        result.failed === 0 ? "ok" : "warn",
        `${verb}完成：${result.total - result.failed} / ${result.total}`,
        failed.length ? failed.slice(0, 3).map((item) => `${item.slug}：${item.error ?? "失败"}`).join("；") : undefined,
      );
      channelPick.clear();
      adapterPick.clear();
      refresh();
      await reload();
    } catch (problem) {
      notify("bad", "批量操作失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <span className="page-kicker">channels · 采集通道</span>
          <h1 className="page-title">出站通道与访问策略</h1>
          <p className="page-lede">
            通道定义取数方式，解析器把结果变成条目与字段。两者都可以在页面上新建、改写、启停与删除，内置解析器只保护删除。
          </p>
        </div>
        <div className="page-actions">
          {runtimeRows.length > 0 ? (
            <Pill tone={abnormal > 0 ? "warn" : "ok"} dot>
              {abnormal > 0 ? `${abnormal} 项运行态异常` : "运行态正常"}
            </Pill>
          ) : null}
          <button type="button" className="btn" onClick={() => void reload()} disabled={loading}>
            <IconRefresh size={13} /> 刷新
          </button>
          <button type="button" className="btn btn-primary" onClick={() => setPicking("accessor")}>
            <IconPlus size={13} /> 新建通道
          </button>
        </div>
      </header>

      {failure ? (
        <div className="note is-bad">
          <IconAlert size={14} />
          <span>{failure}</span>
        </div>
      ) : null}

      <div>
        <SectionTitle title="通道" note={`${accessors.length} 种取数方式`} />
        <BatchBar
          count={channelPick.size}
          busy={busy}
          onClear={channelPick.clear}
          actions={[
            { key: "enable", label: "启用", icon: <IconPlay size={12} />, onClick: () => void batch("accessor", "enable", channelPick.list()) },
            { key: "disable", label: "停用", icon: <IconPause size={12} />, onClick: () => void batch("accessor", "disable", channelPick.list()) },
            { key: "remove", label: "删除", danger: true, onClick: () => void batch("accessor", "remove", channelPick.list()) },
          ]}
        />
        {loading ? (
          <Skeleton lines={4} />
        ) : accessors.length === 0 ? (
          <Empty
            title="还没有配置通道"
            text="从 HTTP 访问器起步，信源就能引用它。也可以直接新建一份声明手写参数。"
            action={
              <button type="button" className="btn btn-sm" onClick={() => setPicking("accessor")}>
                新建通道
              </button>
            }
          />
        ) : (
          <div className="channel-grid">
            {accessors.map((channel) => (
              <div
                key={channel.id}
                role="button"
                tabIndex={0}
                className={`channel-card is-clickable${channel.enabled ? "" : " is-dim"}`}
                onClick={() => setEditor({ kind: "accessor", item: channel, seed: null })}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    setEditor({ kind: "accessor", item: channel, seed: null });
                  }
                }}
              >
                <span className="channel-head">
                  <label className="pick-inline" onClick={(event) => event.stopPropagation()}>
                    <input
                      type="checkbox"
                      aria-label={`选择 ${channel.name}`}
                      checked={channelPick.has(channel.slug)}
                      onChange={() => channelPick.toggle(channel.slug)}
                    />
                  </label>
                  <span className="channel-name">{channel.name}</span>
                  <button
                    type="button"
                    className="btn btn-sm btn-quiet"
                    disabled={toggling === `accessor:${channel.slug}`}
                    onClick={(event) => {
                      event.stopPropagation();
                      void toggleRow("accessor", channel);
                    }}
                  >
                    {channel.enabled ? (
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
                <span className="channel-kind">
                  {implementationLabel(channel.implementation ?? "") || channel.implementation || "自定义"}
                </span>
                <dl className="channel-specs">
                  <div className="channel-spec">
                    <dt>超时</dt>
                    <dd>{channel.timeout ?? "默认"}</dd>
                  </div>
                  <div className="channel-spec">
                    <dt>并发</dt>
                    <dd>{channel.concurrency ?? 1}</dd>
                  </div>
                  <div className="channel-spec">
                    <dt>出站</dt>
                    <dd>{proxyOf(channel.doc) ? proxyLabel(proxyOf(channel.doc)) : profileLabel(channel.profile ?? "direct")}</dd>
                  </div>
                  <div className="channel-spec">
                    <dt>运行态</dt>
                    <dd>
                      <RuntimeCell item={runtimeOf(runtime?.accessors, channel.slug)} />
                    </dd>
                  </div>
                </dl>
                <span className="faint">
                  {channel.references} 个信源在用 · {channel.file.split(/[\\/]/).slice(-2).join("/")}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>

      <div className="overview-grid">
        <div className="overview-main">
          <Card
            title="站点守则"
            sub="采集前读取 robots，再决定是否访问"
            action={
              <button type="button" className="btn btn-quiet btn-sm" onClick={() => void reload()}>
                <IconRefresh size={12} /> 重新读取
              </button>
            }
            tight
          >
            {report === null ? (
              <div style={{ padding: 16 }}>
                <Skeleton lines={4} />
              </div>
            ) : report.robots.length === 0 ? (
              <Empty title="还没有站点记录" text="第一次访问站点后，这里会记录抓取许可与最近一次读取时间。" />
            ) : (
              <table className="table">
                <thead>
                  <tr>
                    <th>站点</th>
                    <th>许可</th>
                    <th>读取时间</th>
                    <th className="align-right">大小</th>
                  </tr>
                </thead>
                <tbody>
                  {robotsView(report, now).map((row) => (
                    <tr key={row.origin}>
                      <td className="mono" style={{ fontSize: "var(--fs-xs)" }}>
                        {row.origin}
                      </td>
                      <td>
                        <Pill tone={row.tone}>{row.label}</Pill>
                      </td>
                      <td className="muted">{row.fetched}</td>
                      <td className="align-right mono faint">{row.size}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </Card>

          <Card title="限流与熔断" sub="连续失败会进入冷却，降低对目标站点的请求压力" tight>
            {report === null ? (
              <div style={{ padding: 16 }}>
                <Skeleton lines={3} />
              </div>
            ) : report.circuits.length === 0 && report.rates.length === 0 ? (
              <Empty title="没有正在冷却的站点" text="熔断与令牌桶只在出现压力时才会写入记录。" />
            ) : (
              <table className="table">
                <thead>
                  <tr>
                    <th>对象</th>
                    <th>最近状态</th>
                    <th>冷却至</th>
                    <th className="align-right">次数</th>
                  </tr>
                </thead>
                <tbody>
                  {circuitView(report, now).map((row) => (
                    <tr key={row.key}>
                      <td className="mono" style={{ fontSize: "var(--fs-xs)" }}>
                        {row.key}
                      </td>
                      <td>
                        <Pill tone={row.tone}>{row.state}</Pill>
                      </td>
                      <td className="muted">{row.until}</td>
                      <td className="align-right mono faint">{row.count}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </Card>
        </div>

        <div className="overview-side">
          <GlobalProxy
            declared={proxyOf(snapshot?.catalog.system ?? {})}
            status={status}
            onSaved={() => {
              refresh();
              void reload();
            }}
          />
          <AddressChecker />
        </div>
      </div>

      <div>
        <SectionTitle title="解析器" note={`${adapters.length} 个`} />
        <div className="row" style={{ marginBottom: 10 }}>
          <button type="button" className="btn btn-sm" onClick={() => setPicking("adapter")}>
            <IconPlus size={12} /> 新建解析器
          </button>
          <span className="faint">内置解析器可以改写与停用，但不能删除</span>
        </div>
        <BatchBar
          count={adapterPick.size}
          busy={busy}
          onClear={adapterPick.clear}
          actions={[
            { key: "enable", label: "启用", icon: <IconPlay size={12} />, onClick: () => void batch("adapter", "enable", adapterPick.list()) },
            { key: "disable", label: "停用", icon: <IconPause size={12} />, onClick: () => void batch("adapter", "disable", adapterPick.list()) },
            { key: "remove", label: "删除", danger: true, onClick: () => void batch("adapter", "remove", adapterPick.list()) },
          ]}
        />
        {adapters.length === 0 ? (
          <Empty title="还没有解析器" text="解析器把一次访问的结果变成条目或字段。" />
        ) : (
          <div className="channel-grid">
            {adapters.map((adapter) => (
              <div
                key={adapter.id}
                role="button"
                tabIndex={0}
                className={`module-card is-clickable${adapter.enabled ? "" : " is-dim"}`}
                onClick={() => setEditor({ kind: "adapter", item: adapter, seed: null })}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    setEditor({ kind: "adapter", item: adapter, seed: null });
                  }
                }}
              >
                <div className="module-top">
                  <label className="pick-inline" onClick={(event) => event.stopPropagation()}>
                    <input
                      type="checkbox"
                      aria-label={`选择 ${adapter.name}`}
                      checked={adapterPick.has(adapter.slug)}
                      onChange={() => adapterPick.toggle(adapter.slug)}
                    />
                  </label>
                  <span className="module-name">{adapter.name}</span>
                  {adapter.builtin ? <Pill>内置</Pill> : <Pill tone="info">自定义</Pill>}
                  <button
                    type="button"
                    className="btn btn-sm btn-quiet"
                    disabled={toggling === `adapter:${adapter.slug}`}
                    onClick={(event) => {
                      event.stopPropagation();
                      void toggleRow("adapter", adapter);
                    }}
                  >
                    {adapter.enabled ? (
                      <>
                        <IconPause size={12} /> 停用
                      </>
                    ) : (
                      <>
                        <IconPlay size={12} /> 启用
                      </>
                    )}
                  </button>
                </div>
                <span className="module-id">{adapter.id}</span>
                <span className="module-origin">
                  {adapter.references} 份声明在用 · {adapter.file.split(/[\\/]/).slice(-2).join("/")}
                </span>
                <span className="module-origin">
                  运行态 <RuntimeCell item={runtimeOf(runtime?.adapters, adapter.slug)} />
                </span>
              </div>
            ))}
          </div>
        )}
      </div>

      {picking ? (
        <TemplatePicker
          kind={picking}
          onClose={() => setPicking(null)}
          onPick={(seed) => {
            const kind = picking;
            setPicking(null);
            setEditor({ kind, item: null, seed });
          }}
        />
      ) : null}

      {editor ? (
        <DeclarationDrawer
          key={editor.item?.id ?? `new-${editor.kind}`}
          kind={editor.kind}
          item={editor.item}
          seed={editor.seed}
          accessors={accessors}
          onClose={() => setEditor(null)}
          onSaved={(message) => {
            notify("ok", message, "采集进程约 2 秒内热加载；代际变化后这一页会自动刷新。");
            refresh();
            void reload();
          }}
        />
      ) : null}

      <p className="faint">
        <IconShield size={11} /> 控制台只读取这些状态，不发起抓取；自检以采集进程的网络身份做一次地址判断。
      </p>
    </div>
  );
}

/** 全局出站代理：写进 system.yaml 的 policy.egress.proxy，与单条声明的覆盖叠加。 */
function GlobalProxy({
  declared,
  status,
  onSaved,
}: {
  declared: string | null;
  status: EgressStatus | null;
  onSaved: () => void;
}) {
  const { notify } = useConsole();
  const [draft, setDraft] = useState<string | null>(declared);
  const [touched, setTouched] = useState(false);
  const [saving, setSaving] = useState(false);
  const value = touched ? draft : declared;

  useEffect(() => {
    if (!touched) {
      setDraft(declared);
    }
  }, [declared, touched]);

  async function submit(next: string | null) {
    setSaving(true);
    try {
      const saved = await setGlobalProxy(next ?? "");
      setTouched(false);
      notify(
        "ok",
        saved ? "全局代理已更新" : "已回到跟随系统",
        saved ? `所有未单独设置的声明会经 ${saved} 出站。` : "出站按环境变量与操作系统设置解析。",
      );
      onSaved();
    } catch (problem) {
      notify("bad", "代理没有写入", problem instanceof Error ? problem.message : undefined);
    } finally {
      setSaving(false);
    }
  }

  return (
    <Card title="出站方式" sub="全局代理，可被单条声明的代理覆盖" tight>
      <div className="card-body">
        <div className="proxy-summary">
          <span className="faint">当前生效</span>
          <span className="mono">{status?.upstream?.url ?? (proxyMode(declared) === "off" ? "直连" : "未使用代理")}</span>
          {status?.upstream ? <span className="faint">来源 {status.upstream.source}</span> : null}
        </div>
        <ProxyInput
          value={value}
          onChange={(next) => {
            setTouched(true);
            setDraft(next);
          }}
          note="不覆盖时按环境变量与系统设置出站"
        />
        <div className="row">
          <button
            type="button"
            className="btn btn-sm btn-primary"
            disabled={saving || !touched}
            onClick={() => void submit(value)}
          >
            {saving ? "写入中…" : "保存全局代理"}
          </button>
          {touched ? (
            <button
              type="button"
              className="btn btn-sm btn-quiet"
              onClick={() => {
                setTouched(false);
                setDraft(declared);
              }}
            >
              还原
            </button>
          ) : null}
        </div>
        <p className="form-hint">
          写入 system.yaml 的 policy.egress.proxy。信源与通道也可以在各自声明里填 <span className="mono">off</span> 或代理地址，逐条覆盖这里。
        </p>
      </div>
    </Card>
  );
}

function AddressChecker() {
  const { notify } = useConsole();
  const [url, setUrl] = useState("https://example.com/");
  const [result, setResult] = useState<EgressCheck | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function run() {
    const value = url.trim();
    if (!value) {
      return;
    }
    setPending(true);
    setError(null);
    setResult(null);
    try {
      setResult(await egressCheck(value));
    } catch (problem) {
      setError(problem instanceof Error ? problem.message : "自检失败");
      notify("bad", "地址自检失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setPending(false);
    }
  }

  return (
    <Card
      title="地址自检"
      sub="访问前先确认这个地址会被放行"
      action={
        <button type="button" className="btn btn-sm" onClick={run} disabled={pending}>
          {pending ? "检查中…" : "检查"}
        </button>
      }
    >
      <div className="checker">
        <div className="checker-row">
          <label className="field-input">
            <span className="sr-only">要检查的地址</span>
            <input
              type="url"
              value={url}
              placeholder="https://"
              onChange={(event) => setUrl(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  void run();
                }
              }}
              spellCheck={false}
            />
          </label>
        </div>
        {error ? <div className="note is-warn">{error}</div> : null}
        {result ? (
          <div className="check-result">
            <div className="check-verdict">
              <Pill tone={result.blocked ? "bad" : "ok"} dot>
                {result.blocked ? "会被拦截" : "可以访问"}
              </Pill>
              <span className="faint" style={{ fontSize: "var(--fs-xs)", fontWeight: 400 }}>
                {result.proxied ? "经上游代理" : "直连"}
              </span>
            </div>
            <div className="mono faint" style={{ fontSize: "var(--fs-2xs)", overflowWrap: "anywhere" }}>
              {result.addresses.length ? result.addresses.join(" · ") : "没有解析到地址"}
            </div>
            <p className="faint" style={{ fontSize: "var(--fs-2xs)", lineHeight: 1.6 }}>
              {result.blocked
                ? "目标解析到私网或本机地址，出站策略默认拦截。确需访问时，请在系统策略里放开私网，或改走代理。"
                : "地址落在允许范围内。真正访问时仍会遵守站点的 robots 与限速。"}
            </p>
          </div>
        ) : (
          <p className="faint" style={{ fontSize: "var(--fs-2xs)", lineHeight: 1.6 }}>
            使用采集进程的解析与私网规则判断，结果与真实采集一致。
          </p>
        )}
      </div>
    </Card>
  );
}

function robotsView(input: EgressReport, now: number) {
  return input.robots
    .slice()
    .sort((left, right) => right.fetchedAt - left.fetchedAt)
    .slice(0, 10)
    .map((row) => ({
      origin: row.origin,
      label: row.failed ? "读取失败 · 按默认策略" : row.status === 200 ? "允许" : `HTTP ${row.status}`,
      tone: (row.failed ? "warn" : row.status === 200 ? "ok" : "info") as "warn" | "ok" | "info",
      fetched: row.fetchedAt ? relative(new Date(row.fetchedAt).toISOString(), now) : "—",
      size: row.bodyBytes ? `${Math.round(row.bodyBytes / 1024)} KB` : "—",
    }));
}

function circuitView(input: EgressReport, clock: number) {
  const rows = input.circuits.slice(0, 8).map((row) => {
    const cooling = row.openUntil !== null && row.openUntil > clock;
    return {
      key: row.host,
      state: cooling ? "冷却中" : "已恢复",
      tone: (cooling ? "warn" : "ok") as "warn" | "ok",
      until: cooling && row.openUntil ? relative(new Date(row.openUntil).toISOString(), clock) : "—",
      count: `${row.consecutive} 次`,
    };
  });
  const extra = input.rates.slice(0, 4).map((row) => ({
    key: row.key,
    state: row.inFlight > 0 ? "正在访问" : "空闲",
    tone: "info" as const,
    until: row.notBefore ? relative(new Date(row.notBefore).toISOString(), clock) : "—",
    count: `${Math.round(row.tokens)}`,
  }));
  return [...rows, ...extra];
}

/** 一条声明对应的运行态；没有记录说明还没跑过。 */
function runtimeOf(items: HealthItem[] | undefined, slug: string): HealthItem | null {
  return items?.find((item) => item.id === slug) ?? null;
}

/** 运行态单元格：状态点 + 最近一次的关键计数。 */
function RuntimeCell({ item }: { item: HealthItem | null }) {
  if (!item) {
    return <span className="faint">还没跑过</span>;
  }
  const state = healthOf(item.status, true);
  const tone = item.alert || state.tone === "bad" ? "bad" : state.tone === "warn" ? "warn" : state.tone === "ok" ? "ok" : "idle";
  const detail = item.lastError
    ? `拦截 ${item.blockedHits} 次`
    : item.selectorMiss > 0
      ? `选择器未命中 ${item.selectorMiss} 次`
      : `最近成功 ${relative(item.lastOkAt)}`;
  return (
    <span className="row" style={{ gap: 6 }}>
      <Pill tone={tone} dot>
        {state.label}
      </Pill>
      <span className="faint" style={{ fontSize: "var(--fs-2xs)" }}>
        {detail}
      </span>
    </span>
  );
}
