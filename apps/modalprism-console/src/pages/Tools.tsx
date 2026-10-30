import { useEffect, useState } from "react";

import { Badge, Card, Empty, StructuredValue, useUi } from "../components/ui";
import { fetchUrl, getPreconditions, listSearchEngines, parseInput, runSearch } from "../lib/api";
import type { FetchResult, PreconditionReport, SearchEngine, SearchResult } from "../lib/types";

/**
 * 调试工具：试搜 / 试取正文 / 试解析。
 * 这些是开发者自助验证产品链路的手段，原先嵌在项目页里；项目页只负责项目本身，
 * 调试能力集中到这一页，避免把「用项目」和「验证接口」两件事混在一起。
 */
export function ToolsPage() {
  const { notify } = useUi();
  const [engines, setEngines] = useState<SearchEngine[]>([]);
  const [engine, setEngine] = useState("");
  const [query, setQuery] = useState("");
  const [searchResult, setSearchResult] = useState<SearchResult | null>(null);
  const [url, setUrl] = useState("");
  const [fetchResult, setFetchResult] = useState<FetchResult | null>(null);
  const [parseResult, setParseResult] = useState<Record<string, unknown> | null>(null);
  const [preconditions, setPreconditions] = useState<PreconditionReport | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void listSearchEngines()
      .then((page) => setEngines(page.items))
      .catch(() => setEngines([]));
    void getPreconditions("search")
      .then(setPreconditions)
      .catch(() => setPreconditions(null));
  }, []);

  async function search() {
    const trimmed = query.trim();
    if (!trimmed) return;
    setBusy(true);
    try {
      setSearchResult(await runSearch({ query: trimmed, engine: engine || undefined, limit: 10 }));
    } catch (problem) {
      setSearchResult(null);
      notify("bad", "试搜失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  async function fetchPage() {
    const trimmed = url.trim();
    if (!trimmed) return;
    setBusy(true);
    try {
      setFetchResult(await fetchUrl(trimmed));
    } catch (problem) {
      setFetchResult(null);
      notify("bad", "获取失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  async function parsePage() {
    const trimmed = url.trim();
    if (!trimmed) return;
    setBusy(true);
    try {
      setParseResult(await parseInput({ url: trimmed }));
      notify("ok", "解析完成");
    } catch (problem) {
      setParseResult(null);
      notify("bad", "解析失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  const unmet = preconditions
    ? preconditions.scopes
        .filter((item) => !item.met)
        .flatMap((item) => item.requirements.map((req) => `${req.message}（${req.where}）`))
    : [];

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <div className="kicker">tools · 调试工具</div>
          <h1 className="page-title">调试工具</h1>
          <p className="page-lede">
            以下入口复用采集链路；前置条件不满足时返回 412 并给出配置位置。
          </p>
        </div>
      </header>

      {preconditions && !preconditions.met ? (
        <Card title="前置未就绪" sub="补齐后可返回结果。">
          <div className="stack" style={{ gap: 4 }}>
            {unmet.map((text, index) => (
              <div key={index} className="row" style={{ gap: 6 }}>
                <Badge tone="warn">待配置</Badge>
                <span className="small muted">{text}</span>
              </div>
            ))}
          </div>
        </Card>
      ) : null}

      <div className="grid cols-2">
        <Card title="试搜" sub="使用产品搜索链路：DDG 免密钥；空结果回退至 DDG Lite。">
          <div className="stack">
            <div className="row" style={{ gap: 8 }}>
              <select className="input" style={{ width: 200 }} value={engine} onChange={(event) => setEngine(event.target.value)}>
                <option value="">默认引擎</option>
                {engines.map((item) => (
                  <option key={item.id} value={item.id}>
                    {item.name}
                    {item.requiresKeyRef ? "（需密钥）" : ""}
                  </option>
                ))}
              </select>
              <input
                className="input"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder="搜索关键词"
                onKeyDown={(event) => {
                  if (event.key === "Enter") void search();
                }}
              />
              <button type="button" className="btn btn-primary" onClick={() => void search()} disabled={busy}>
                搜索
              </button>
            </div>
            {searchResult ? (
              <div className="stack" style={{ gap: 6 }}>
                <span className="small muted">
                  引擎 {searchResult.engine}
                  {searchResult.requestedEngine && searchResult.requestedEngine !== searchResult.engine
                    ? `（请求 ${searchResult.requestedEngine}，已回退）`
                    : ""}{" "}
                  · {searchResult.count} 条结果
                </span>
                {searchResult.degraded ? (
                  <div className="row" style={{ gap: 6 }}>
                    <Badge tone="warn">无结果</Badge>
                    <span className="small muted">{searchResult.note}</span>
                  </div>
                ) : null}
                {searchResult.results.length === 0 && !searchResult.degraded ? (
                  <Empty title="没有命中" text="调整关键词，或在设置中配置带密钥引擎。" />
                ) : null}
                {searchResult.results.map((hit, index) => (
                  <div key={index} className="stack" style={{ gap: 1 }}>
                    <a href={hit.url ?? "#"} target="_blank" rel="noreferrer" className="mono small">
                      {hit.title ?? hit.url ?? "(无标题)"}
                    </a>
                    {hit.snippet ? <span className="small muted">{hit.snippet}</span> : null}
                  </div>
                ))}
              </div>
            ) : null}
          </div>
        </Card>

        <Card title="试取正文" sub="POST /fetch：URL → 正文，回环地址也可用于本机验证。">
          <div className="stack">
            <div className="row" style={{ gap: 8 }}>
              <input
                className="input"
                value={url}
                onChange={(event) => setUrl(event.target.value)}
                placeholder="https://example.com/page"
                onKeyDown={(event) => {
                  if (event.key === "Enter") void fetchPage();
                }}
              />
              <button type="button" className="btn btn-primary" onClick={() => void fetchPage()} disabled={busy}>
                获取
              </button>
            </div>
            {fetchResult ? (
              <div className="stack" style={{ gap: 4 }}>
                <span className="small muted">
                  HTTP {fetchResult.status} · {fetchResult.contentType || "未知类型"}
                  {fetchResult.truncated ? " · 已截断" : ""}
                </span>
                <pre className="mono small" style={{ maxHeight: 240, overflow: "auto", whiteSpace: "pre-wrap" }}>
                  {fetchResult.text.slice(0, 4000)}
                </pre>
              </div>
            ) : null}
          </div>
        </Card>

        <Card title="试解析" sub="POST /parse：URL → 结构化抽取（rawprism 未接入时返回 412）。">
          <div className="stack">
            <div className="row" style={{ gap: 8 }}>
              <input
                className="input"
                value={url}
                onChange={(event) => setUrl(event.target.value)}
                placeholder="https://example.com/page"
                onKeyDown={(event) => {
                  if (event.key === "Enter") void parsePage();
                }}
              />
              <button type="button" className="btn btn-primary" onClick={() => void parsePage()} disabled={busy}>
                解析
              </button>
            </div>
            {parseResult ? (
              <div style={{ maxHeight: 280, overflow: "auto" }}>
                <StructuredValue value={parseResult} />
              </div>
            ) : null}
          </div>
        </Card>
      </div>
    </div>
  );
}
