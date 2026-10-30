import { useCallback, useEffect, useMemo, useState } from "react";

import { IconCheck, IconExternal, IconRefresh } from "../components/icons";
import { PlatformIcon } from "../components/platforms";
import { Card, Empty, Loading, Problem, Tabs, useUi } from "../components/ui";
import { getExternalProtocols } from "../lib/api";
import type { ApiDocGroup, ApiDocItem, ExternalCatalog, ExternalProtocol } from "../lib/types";

const STATUS_LABEL: Record<string, string> = {
  enabled: "已启用",
  internal: "仅进程内",
  disabled: "未启用",
};

const STATUS_TONE: Record<string, string> = {
  enabled: "ok",
  internal: "info",
  disabled: "neutral",
};

const METHOD_TONE: Record<string, string> = {
  GET: "info",
  POST: "ok",
  PATCH: "warn",
  PUT: "warn",
  DELETE: "bad",
};

/** 外部调用：按协议分段列出本进程实际提供的入口，先选协议再看方法。 */
export function ExternalPage() {
  const { notify } = useUi();
  const [catalog, setCatalog] = useState<ExternalCatalog | null>(null);
  const [protocol, setProtocol] = useState("http");
  const [query, setQuery] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const data = await getExternalProtocols();
      setCatalog(data);
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

  async function copy(text: string, label: string) {
    try {
      await navigator.clipboard.writeText(text);
      notify("ok", `${label}已复制`);
    } catch {
      notify("warn", "浏览器不允许写剪贴板", "请手动选择复制。");
    }
  }

  const active = useMemo(
    () => catalog?.protocols.find((item) => item.id === protocol) ?? catalog?.protocols[0] ?? null,
    [catalog, protocol],
  );

  const groups = useMemo(() => {
    if (!active) return [];
    const needle = query.trim().toLowerCase();
    if (!needle) return active.groups;
    return active.groups
      .map((group) => ({ ...group, items: group.items.filter((item) => matches(item, needle)) }))
      .filter((group) => group.items.length > 0);
  }, [active, query]);

  const matched = useMemo(
    () => groups.reduce((total, group) => total + group.items.length, 0),
    [groups],
  );

  if (loading && !catalog) {
    return (
      <div className="page">
        <Loading lines={6} />
      </div>
    );
  }

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <div className="kicker">external · 外部调用</div>
          <h1 className="page-title">外部调用</h1>
          <p className="page-lede">
            按协议列出入口、方法与示例；未启用的协议标注为不可用。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn" onClick={() => void load()} disabled={loading}>
            <IconRefresh size={14} /> 刷新
          </button>
          {catalog ? (
            <button type="button" className="btn" onClick={() => void copy(catalog.baseUrl, "接口地址")}>
              复制基础地址
            </button>
          ) : null}
        </div>
      </header>

      {error ? <Problem error={error} onRetry={() => void load()} /> : null}

      {catalog ? (
        <>
          <Tabs
            value={protocol}
            onChange={(next) => {
              setProtocol(next);
              setQuery("");
            }}
            items={catalog.protocols.map((item) => ({
              id: item.id,
              label: item.name,
              count: item.methods,
            }))}
          />

          {active ? (
            <ProtocolPanel
              protocol={active}
              groups={groups}
              matched={matched}
              query={query}
              onQuery={setQuery}
              onCopy={copy}
              baseUrl={catalog.baseUrl}
            />
          ) : (
            <Empty title="协议目录不可用" text="collector 在线后重试。" />
          )}

          {/* 通用约定不属于任何单一协议：收在页尾折叠区，不挡住按协议查阅。 */}
          <details className="stack" style={{ gap: 8 }}>
            <summary className="small muted" style={{ cursor: "pointer" }}>
              附：跨协议调用约定与上手步骤
            </summary>
            <div className="stack" style={{ gap: 12 }}>
              <Card title="调用约定" sub="请求头、错误、分页与异步语义由各协议共用" flush>
                <div className="table-wrap">
                  <table className="table">
                    <thead>
                      <tr>
                        <th style={{ width: 180 }}>请求头</th>
                        <th style={{ width: 90 }}>是否必带</th>
                        <th>说明</th>
                      </tr>
                    </thead>
                    <tbody>
                      {catalog.conventions.headers.map((header) => (
                        <tr key={header.name}>
                          <td className="mono small">{header.name}</td>
                          <td className="small">{header.optional ? "可选" : "必带"}</td>
                          <td className="small">{header.note}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
                <div className="card-body stack" style={{ gap: 6 }}>
                  <span className="small muted">错误：{catalog.conventions.errors}</span>
                  <span className="small muted">分页：{catalog.conventions.paging}</span>
                  <span className="small muted">异步：{catalog.conventions.async}</span>
                  <span className="small muted">版本：{catalog.conventions.etag}</span>
                </div>
              </Card>

              <Card title="上手步骤" sub="从服务在线到写入第一条声明" flush>
                <div className="list">
                  {catalog.quickStart.map((step) => (
                    <div className="list-row" key={step.title}>
                      <span className="list-main">
                        <span className="list-title">{step.title}</span>
                        <span className="list-meta">
                          <span>{step.body}</span>
                        </span>
                        <div className="stack" style={{ gap: 4, marginTop: 6 }}>
                          {step.commands.map((command) => (
                            <button
                              key={command}
                              type="button"
                              className="code-copy is-quiet"
                              onClick={() => void copy(command, "命令")}
                            >
                              <span className="mono small">{command}</span>
                              <IconCheck size={12} />
                            </button>
                          ))}
                        </div>
                      </span>
                    </div>
                  ))}
                </div>
              </Card>
            </div>
          </details>
        </>
      ) : (
        <Empty title="协议目录不可用" text="collector 在线后重试。" />
      )}
    </div>
  );
}

function matches(item: ApiDocItem, needle: string): boolean {
  return `${item.method} ${item.path} ${item.summary}`.toLowerCase().includes(needle);
}

function ProtocolPanel({
  protocol,
  groups,
  matched,
  query,
  onQuery,
  onCopy,
  baseUrl,
}: {
  protocol: ExternalProtocol;
  groups: ApiDocGroup[];
  matched: number;
  query: string;
  onQuery: (next: string) => void;
  onCopy: (text: string, label: string) => Promise<void>;
  baseUrl: string;
}) {
  return (
    <div className="stack" style={{ gap: 16 }}>
      <Card
        title={protocol.name}
        sub={protocol.note}
        action={
          <div className="row" style={{ gap: 8 }}>
            <span className={`badge ${STATUS_TONE[protocol.status] ?? "neutral"}`}>
              {STATUS_LABEL[protocol.status] ?? protocol.status}
            </span>
            <span className="badge neutral">{protocol.methods} 个方法</span>
          </div>
        }
      >
        <div className="stack" style={{ gap: 8 }}>
          <div className="row wrap" style={{ gap: 8 }}>
            <PlatformIcon
              icon={protocol.id === "cli" ? "endpoint" : protocol.id === "http" ? "webhook" : protocol.id}
              label={protocol.name}
              size={26}
            />
            {protocol.endpoint ? (
              <button type="button" className="code-copy" onClick={() => void onCopy(protocol.endpoint ?? "", "地址")}>
                <span className="mono">{protocol.endpoint}</span>
                <IconCheck size={12} />
              </button>
            ) : null}
            {protocol.command ? (
              <button type="button" className="code-copy" onClick={() => void onCopy(protocol.command ?? "", "命令")}>
                <span className="mono">{protocol.command}</span>
                <IconCheck size={12} />
              </button>
            ) : null}
          </div>
          {protocol.examples.length ? (
            <div className="stack" style={{ gap: 4 }}>
              {protocol.examples.map((example) => (
                <button key={example} type="button" className="code-copy is-quiet" onClick={() => void onCopy(example, "示例")}>
                  <span className="mono small">{example}</span>
                </button>
              ))}
            </div>
          ) : null}
        </div>
      </Card>

      {protocol.groups.length ? (
        <div className="toolbar-band">
          <label className="search">
            <span className="muted" aria-hidden="true">⌕</span>
            <input
              value={query}
              onChange={(event) => onQuery(event.target.value)}
              placeholder="按路径、方法或用途检索"
              aria-label="检索接口"
            />
          </label>
          <span className="fill" />
          <span className="small muted">
            {matched} / {protocol.methods} 个接口
          </span>
        </div>
      ) : null}

      {protocol.groups.length === 0 ? (
        <Card title="方法清单" sub={protocol.note} flush>
          <div className="table-wrap">
            <table className="table">
              <thead>
                <tr>
                  <th style={{ width: 220 }}>方法</th>
                  <th>调用方式</th>
                  <th>说明</th>
                </tr>
              </thead>
              <tbody>
                {protocol.items.map((item) => (
                  <tr key={item.name}>
                    <td className="mono small">
                      <button type="button" className="btn btn-ghost btn-sm" onClick={() => void onCopy(item.usage, "用法")}>
                        {item.name}
                      </button>
                    </td>
                    <td className="mono small">{item.usage}</td>
                    <td className="small">{item.summary}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Card>
      ) : null}

      {protocol.groups.length && groups.length === 0 ? (
        <Empty title="无匹配接口" text="调整关键词，或清空检索以查看全部。" />
      ) : null}

      {groups.map((group) => (
        <Card key={group.id} title={group.label} sub={`${group.items.length} 个接口 · ${group.note}`} flush>
          <div className="list">
            {group.items.map((item) => (
              <div className="list-row" key={`${item.method}-${item.path}`}>
                <span className={`badge ${METHOD_TONE[item.method] ?? "neutral"}`}>{item.method}</span>
                <span className="list-main">
                  <span className="list-title mono">{item.path}</span>
                  <span className="list-meta">
                    <span>{item.summary}</span>
                    {item.query ? <span>参数：{item.query}</span> : null}
                    {item.body ? <span>请求体：{item.body}</span> : null}
                  </span>
                </span>
                <button
                  type="button"
                  className="btn btn-ghost btn-sm"
                  onClick={() => void onCopy(curlFor(item, baseUrl), "curl 命令")}
                >
                  <IconExternal size={12} /> curl
                </button>
              </div>
            ))}
          </div>
        </Card>
      ))}
    </div>
  );
}

/** 复制用的最小可用命令：路径参数留占位符，写操作补上幂等键与请求体占位。 */
function curlFor(item: ApiDocItem, baseUrl: string): string {
  const path = item.path.replace(/\{([^}]+)\}/g, "<$1>");
  const url = `${baseUrl}${path}`;
  if (item.method === "GET") return `curl "${url}"`;
  const headers = [`-H "content-type: application/json"`, `-H "idempotency-key: <uuid>"`];
  if (item.method === "DELETE") return `curl -X DELETE "${url}" -H "if-match: <etag>"`;
  if (item.method === "PATCH") return `curl -X PATCH "${url}" ${headers.join(" ")} -H "if-match: <etag>" -d '<body>'`;
  return `curl -X ${item.method} "${url}" ${headers.join(" ")} -d '<body>'`;
}
