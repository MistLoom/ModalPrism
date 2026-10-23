import { useCallback, useEffect, useState } from "react";

import { Drawer } from "../components/Drawer";
import { IconAlert, IconPause, IconPlay, IconRefresh, IconShield } from "../components/icons";
import {
  Card,
  Definitions,
  Empty,
  Pill,
  SectionTitle,
  Skeleton,
} from "../components/ui";
import {
  egressCheck,
  egressReport,
  egressStatus,
  explain,
  mergeChannels,
  stripPrefix,
} from "../lib/api";
import { toggleModuleEnabled } from "../lib/commands";
import { useConsole } from "../lib/console";
import { profileLabel, relative } from "../lib/format";
import type { Channel, EgressCheck, EgressReport, EgressStatus, FieldOrigin } from "../lib/types";

export function ChannelsPage() {
  const { snapshot, state, now, notify, refresh } = useConsole();
  const [status, setStatus] = useState<EgressStatus | null>(null);
  const [report, setReport] = useState<EgressReport | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [togglingId, setTogglingId] = useState<string | null>(null);
  const loading = state === "loading";

  const modules = snapshot?.catalog.modules ?? [];
  const channels = mergeChannels(modules);
  const adapters = modules.filter((module) => module.kind === "adapter");
  const selected = selectedId ? channels.find((channel) => channel.id === selectedId) ?? null : null;

  const toggle = async (
    item: { id: string; name: string; file: string; enabled: boolean },
    enable: boolean,
    kind: string,
  ) => {
    setTogglingId(item.id);
    await toggleModuleEnabled(item, enable, kind, notify, refresh);
    setTogglingId(null);
  };

  const reload = useCallback(async () => {
    try {
      const [next, log] = await Promise.all([egressStatus(), egressReport()]);
      setStatus(next);
      setReport(log);
      setFailure(null);
    } catch (problem) {
      setFailure(problem instanceof Error ? problem.message : "读取通道状态失败");
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <span className="page-kicker">channels · 采集通道</span>
          <h1 className="page-title">出站通道与访问策略</h1>
          <p className="page-lede">
            通道定义取数方式：订阅、网页、接口、浏览器或外部推送。出站策略与站点守则决定请求是否放行，以及被站点拒绝的概率。
          </p>
        </div>
      </header>

      <div>
        <SectionTitle title="通道" note={`${channels.length} 种取数方式`} />
        {loading ? (
          <Skeleton lines={4} />
        ) : channels.length === 0 ? (
          <Empty title="还没有配置通道" text="在 config/accessors 下添加 accessor，信源就可以引用它。" />
        ) : (
          <div className="channel-grid">
            {channels.map((channel) => (
              <div
                key={channel.id}
                role="button"
                tabIndex={0}
                className="channel-card is-clickable"
                onClick={() => setSelectedId(channel.id)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    setSelectedId(channel.id);
                  }
                }}
              >
                <span className="channel-head">
                  <span className="channel-name">{channel.name}</span>
                  <button
                    type="button"
                    className="btn btn-sm btn-quiet"
                    disabled={togglingId === channel.id}
                    onClick={(event) => {
                      event.stopPropagation();
                      void toggle(channel, !channel.enabled, "通道");
                    }}
                  >
                    {togglingId === channel.id ? (
                      "写入中…"
                    ) : channel.enabled ? (
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
                <span className="channel-kind">{channel.implementationLabel || channel.implementation}</span>
                <dl className="channel-specs">
                  <div className="channel-spec">
                    <dt>超时</dt>
                    <dd>{channel.timeout}</dd>
                  </div>
                  <div className="channel-spec">
                    <dt>并发</dt>
                    <dd>{channel.concurrency}</dd>
                  </div>
                  <div className="channel-spec">
                    <dt>重试</dt>
                    <dd>{channel.retryAttempts}</dd>
                  </div>
                </dl>
                <span className="faint" style={{ fontSize: 10.5 }}>
                  出站 · {profileLabel(channel.profile)} · {channel.file.split(/[\\/]/).slice(-2).join("/")}
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
              <button
                type="button"
                className="btn btn-quiet btn-sm"
                onClick={() => void reload()}
              >
                <IconRefresh size={12} />
                重新读取
              </button>
            }
            tight
          >
            {failure ? (
              <div style={{ padding: 16 }}>
                <div className="note is-warn">
                  <IconAlert size={14} />
                  <span>{failure}</span>
                </div>
              </div>
            ) : report === null ? (
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
                  {robotsView(report).map((row) => (
                    <tr key={row.origin}>
                      <td className="mono" style={{ fontSize: 11.5 }}>
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
                      <td className="mono" style={{ fontSize: 11.5 }}>
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
          <Card title="出站方式" tight>
            <div style={{ padding: "12px 16px", display: "grid", gap: 7 }}>
              {status === null ? (
                <Skeleton lines={3} />
              ) : (
                <>
                  <div style={{ display: "flex", justifyContent: "space-between", fontSize: 12 }}>
                    <span className="faint">当前模式</span>
                    <span>{profileLabel(status.mode)}</span>
                  </div>
                  <div style={{ display: "flex", justifyContent: "space-between", fontSize: 12 }}>
                    <span className="faint">上游代理</span>
                    <span className="mono" style={{ fontSize: 11 }}>
                      {status.upstream?.url ?? "未使用"}
                    </span>
                  </div>
                  {status.upstream ? (
                    <div style={{ display: "flex", justifyContent: "space-between", fontSize: 12 }}>
                      <span className="faint">代理来源</span>
                      <span>{status.upstream.source}</span>
                    </div>
                  ) : null}
                  <div style={{ display: "flex", justifyContent: "space-between", fontSize: 12 }}>
                    <span className="faint">本机代理端口</span>
                    <span className="mono" style={{ fontSize: 11 }}>
                      {status.proxy ?? "未开启"}
                    </span>
                  </div>
                </>
              )}
            </div>
            <div className="card-foot">
              信源在配置里可以选择直连或经代理；改动后热加载，不影响正在执行的任务。
            </div>
          </Card>

          <AddressChecker />
        </div>
      </div>

      <div>
        <SectionTitle title="解析器" note={`${adapters.length} 个`} />
        {adapters.length === 0 ? (
          <Empty title="还没有解析器" text="解析器把一次访问的结果变成条目或字段。" />
        ) : (
          <div className="channel-grid">
            {adapters.map((adapter) => (
              <div className="module-card" key={adapter.id}>
                <div className="module-top">
                  <span className="module-name">{adapter.name ?? stripPrefix(adapter.id)}</span>
                  <button
                    type="button"
                    className="btn btn-sm btn-quiet"
                    disabled={togglingId === adapter.id}
                    onClick={() =>
                      void toggle(
                        {
                          id: adapter.id,
                          name: adapter.name ?? stripPrefix(adapter.id),
                          file: adapter.file,
                          enabled: adapter.enabled,
                        },
                        !adapter.enabled,
                        "解析器",
                      )
                    }
                  >
                    {togglingId === adapter.id ? (
                      "写入中…"
                    ) : adapter.enabled ? (
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
                  {adapter.file.split(/[\\/]/).slice(-2).join("/")}:{adapter.line}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>

      <ChannelDrawer channel={selected} onClose={() => setSelectedId(null)} />

      <p className="faint" style={{ fontSize: 11 }}>
        <IconShield size={11} /> 控制台只读取这些状态，不发起抓取；自检以采集进程的网络身份做一次地址判断。
      </p>
    </div>
  );

  function robotsView(input: EgressReport) {
    return input.robots
      .slice()
      .sort((left, right) => right.fetchedAt - left.fetchedAt)
      .slice(0, 10)
      .map((row) => ({
        origin: row.origin,
        label: row.failed
          ? "读取失败 · 按默认策略"
          : row.status === 200
            ? "允许"
            : `HTTP ${row.status}`,
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
      const check = await egressCheck(value);
      setResult(check);
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
              <span className="faint" style={{ fontSize: 11.5, fontWeight: 400 }}>
                {result.proxied ? "经上游代理" : "直连"}
              </span>
            </div>
            <div className="mono faint" style={{ fontSize: 11, overflowWrap: "anywhere" }}>
              {result.addresses.length ? result.addresses.join(" · ") : "没有解析到地址"}
            </div>
            <p className="faint" style={{ fontSize: 11, lineHeight: 1.6 }}>
              {result.blocked
                ? "目标解析到私网或本机地址，出站策略默认拦截。确需访问时，请在系统策略里放开私网，或改走代理。"
                : "地址落在允许范围内。真正访问时仍会遵守站点的 robots 与限速。"}
            </p>
          </div>
        ) : (
          <p className="faint" style={{ fontSize: 11, lineHeight: 1.6 }}>
            使用采集进程的解析与私网规则判断，结果与真实采集一致。
          </p>
        )}
      </div>
    </Card>
  );
}

function ChannelDrawer({ channel, onClose }: { channel: Channel | null; onClose: () => void }) {
  const { notify, refresh } = useConsole();
  const [origins, setOrigins] = useState<FieldOrigin[] | null>(null);
  const [toggling, setToggling] = useState(false);

  useEffect(() => {
    setOrigins(null);
    if (!channel) {
      return;
    }
    let alive = true;
    explain(channel.id)
      .then((fields) => {
        if (alive) {
          setOrigins(fields);
        }
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [channel]);

  if (!channel) {
    return null;
  }

  return (
    <Drawer
      key={channel.id}
      open
      kicker={`采集通道 · ${channel.implementationLabel || channel.implementation}`}
      title={channel.name}
      onClose={onClose}
      footer={
        <button
          type="button"
          className="btn btn-quiet"
          disabled={toggling}
          onClick={async () => {
            if (!channel) {
              return;
            }
            setToggling(true);
            await toggleModuleEnabled(channel, !channel.enabled, "通道", notify, refresh);
            setToggling(false);
          }}
        >
          {toggling ? (
            "写入中…"
          ) : channel.enabled ? (
            <>
              <IconPause size={12} /> 停用通道
            </>
          ) : (
            <>
              <IconPlay size={12} /> 启用通道
            </>
          )}
        </button>
      }
    >
      <div className="row">
        <Pill tone={channel.enabled ? "ok" : "idle"} dot>
          {channel.enabled ? "启用" : "停用"}
        </Pill>
        <Pill>{profileLabel(channel.profile)}</Pill>
        {channel.requires ? <Pill tone="warn">额外依赖 · {channel.requires}</Pill> : null}
      </div>

      <div>
        <SectionTitle title="访问约束" />
        <Definitions
          items={[
            ["单次超时", channel.timeout],
            ["并发上限", `${channel.concurrency} 个任务`],
            ["重试次数", `${channel.retryAttempts} 次`],
            ["出站方式", profileLabel(channel.profile)],
            ["定义位置", <span className="mono" style={{ fontSize: 11 }}>{channel.file}:{channel.line}</span>],
          ]}
        />
      </div>

      <div>
        <SectionTitle title="字段来源" />
        {origins === null ? (
          <Skeleton lines={4} />
        ) : origins.length === 0 ? (
          <Empty title="没有可解释的字段" />
        ) : (
          <div className="origin-list">
            {origins.slice(0, 30).map((field) => (
              <div className="origin-row" key={field.path}>
                <span className="origin-path">{field.path}</span>
                <span
                  className={`origin-source${field.viaPreset ? " is-preset" : ""}`}
                  title={`${field.file}:${field.line}`}
                >
                  {shortFile(field.file)}:{field.line}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>

      <div>
        <SectionTitle title="完整配置" />
        <pre className="code">{JSON.stringify(channel.config, null, 2)}</pre>
      </div>
    </Drawer>
  );
}

function shortFile(file: string): string {
  const parts = file.split(/[\\/]/);
  return parts.length <= 2 ? file : parts.slice(-2).join("/");
}
