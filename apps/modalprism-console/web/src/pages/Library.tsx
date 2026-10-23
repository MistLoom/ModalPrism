import { useEffect, useMemo, useState } from "react";

import { Drawer } from "../components/Drawer";
import { IconAlert, IconLibrary } from "../components/icons";
import {
  Card,
  Empty,
  FilterSelect,
  Pagination,
  Pill,
  SearchInput,
  SectionTitle,
  Seg,
  Skeleton,
} from "../components/ui";
import { contentBody, contentKinds, library, structCell } from "../lib/api";
import { useConsole } from "../lib/console";
import { asReadable, relative, stamp } from "../lib/format";
import { navigate, useRoute } from "../lib/router";
import type { ContentKind, LibraryRow } from "../lib/types";

/** 一条内容 = 资源行 + 它的内容类型。 */
interface Entry {
  row: LibraryRow;
  variant: string;
}

const PAGE_SIZE = 40;

/** 时间范围筛选；ms 是相对现在的偏移。 */
const SPANS: { value: string; label: string; ms: number }[] = [
  { value: "all", label: "全部时间", ms: 0 },
  { value: "24h", label: "最近 24 小时", ms: 3_600_000 * 24 },
  { value: "7d", label: "最近 7 天", ms: 3_600_000 * 24 * 7 },
  { value: "30d", label: "最近 30 天", ms: 3_600_000 * 24 * 30 },
];

const BODY_OPTIONS = [
  { value: "any", label: "不限正文" },
  { value: "with", label: "仅有正文" },
];

export function LibraryPage() {
  const { snapshot, cursor, state } = useConsole();
  const { param } = useRoute();
  const [kinds, setKinds] = useState<ContentKind[] | null>(null);
  const [kind, setKind] = useState("all");
  const [text, setText] = useState("");
  const [query, setQuery] = useState("");
  const [sourceId, setSourceId] = useState("all");
  const [span, setSpan] = useState("all");
  const [body, setBody] = useState("any");
  const [page, setPage] = useState(0);
  // keyset 锚点：next 向更早翻（seq < 锚点）；prev 向更新翻（seq > 锚点，升序拉取后反转）。
  const [anchor, setAnchor] = useState<{ seq: number; dir: "next" | "prev" } | null>(null);
  const [rows, setRows] = useState<Entry[] | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Entry | null>(null);

  useEffect(() => {
    let alive = true;
    contentKinds()
      .then((list) => {
        if (alive) {
          setKinds(list);
        }
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [cursor]);

  useEffect(() => {
    const timer = window.setTimeout(() => setQuery(text.trim()), 320);
    return () => window.clearTimeout(timer);
  }, [text]);

  // 从 Strata 页带着 RID 跳来时只看这一条；路径参数变化时回到第一页。
  useEffect(() => {
    setPage(0);
    setAnchor(null);
  }, [param]);

  useEffect(() => {
    let alive = true;
    setRows(null);
    setError(null);
    const known = kinds?.map((item) => item.variant) ?? [];
    if (known.length === 0) {
      setRows([]);
      setHasMore(false);
      return;
    }
    const spanMs = SPANS.find((item) => item.value === span)?.ms ?? 0;
    library({
      variant: kind === "all" ? null : kind,
      variants: known,
      search: query,
      snapshot: cursor,
      limit: PAGE_SIZE,
      sourceId: sourceId === "all" ? null : sourceId,
      since: spanMs > 0 ? new Date(Date.now() - spanMs).toISOString() : null,
      withBody: body === "with",
      before: anchor?.dir === "next" ? anchor.seq : null,
      after: anchor?.dir === "prev" ? anchor.seq : null,
      rid: param,
    })
      .then((result) => {
        if (alive) {
          setRows(result.rows.map((row) => ({ row, variant: variantOf(row, known) })));
          setHasMore(result.hasMore);
        }
      })
      .catch((problem: Error) => {
        if (alive) {
          setError(problem.message);
          setRows([]);
        }
      });
    return () => {
      alive = false;
    };
  }, [kind, query, cursor, kinds, sourceId, span, body, anchor, param]);

  // hasMore 描述的是本次拉取的方向：向上翻时它说明「更新的方向」还有没有。
  const hasPrev = param ? false : anchor?.dir === "prev" ? hasMore : page > 0;
  const hasNext = param ? false : anchor?.dir === "prev" ? true : hasMore;
  const total = useMemo(() => rows?.length ?? 0, [rows]);

  function resetPage() {
    setPage(0);
    setAnchor(null);
  }

  function goNext() {
    const last = rows?.[rows.length - 1]?.row.seq;
    if (last === undefined) {
      return;
    }
    setAnchor({ seq: last, dir: "next" });
    setPage(page + 1);
  }

  function goPrev() {
    const first = rows?.[0]?.row.seq;
    if (first === undefined) {
      return;
    }
    setAnchor({ seq: first, dir: "prev" });
    setPage(page - 1);
  }

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <span className="page-kicker">library · 内容库</span>
          <h1 className="page-title">已入库的内容</h1>
          <p className="page-lede">
            内容库是面向阅读的视图：每条内容保留最新的可读正文与结构化字段，可筛选、检索与回放历史快照。
            要看一条内容的原始字节、版本历史与血缘，去 Strata 页——两边用 RID 互链，抽屉里有直达按钮。
          </p>
        </div>
      </header>

      {param ? (
        <div className="note">
          <IconLibrary size={14} />
          <div style={{ display: "grid", gap: 2 }}>
            <strong>正在定位单条资源</strong>
            <span>
              从 Strata 页跳转而来，只显示这一条内容。
              <button
                type="button"
                className="btn btn-sm"
                style={{ marginLeft: 8 }}
                onClick={() => navigate("library")}
              >
                返回完整列表
              </button>
            </span>
          </div>
        </div>
      ) : null}

      <div className="toolbar">
        <SearchInput
          value={text}
          onChange={setText}
          placeholder="按标题或正文检索…"
          label="检索内容"
        />
        {kinds !== null && kinds.length > 0 ? (
          <Seg
            value={kind}
            onChange={(value) => {
              setKind(value);
              resetPage();
            }}
            label="内容类型"
            options={[
              { value: "all", label: "全部" },
              ...kinds.map((item) => ({ value: item.variant, label: item.label })),
            ]}
          />
        ) : null}
        <span className="spacer" />
        {cursor !== null ? (
          <Pill tone="warn">历史快照 #{cursor}</Pill>
        ) : (
          <span className="faint" style={{ fontSize: 11.5 }}>
            {rows === null ? "读取中" : `${total} 条内容`}
          </span>
        )}
      </div>

      {!param ? (
        <div className="filter-row">
          <FilterSelect
            label="来源"
            value={sourceId}
            onChange={(value) => {
              setSourceId(value);
              resetPage();
            }}
            options={[
              { value: "all", label: "全部来源" },
              ...(snapshot?.sources ?? []).map((source) => ({
                value: source.id,
                label: source.name,
              })),
            ]}
          />
          <FilterSelect
            label="时间"
            value={span}
            onChange={(value) => {
              setSpan(value);
              resetPage();
            }}
            options={SPANS.map(({ value, label }) => ({ value, label }))}
          />
          <FilterSelect
            label="正文"
            value={body}
            onChange={(value) => {
              setBody(value);
              resetPage();
            }}
            options={BODY_OPTIONS}
            disabled={query.trim() !== ""}
            title={query.trim() ? "全文检索与正文筛选不能同时使用" : undefined}
          />
        </div>
      ) : null}
      {query.trim() ? (
        <p className="faint" style={{ fontSize: 11.5, margin: 0 }}>
          检索模式按相关度返回前 {PAGE_SIZE} 条，不支持翻页与正文筛选；清空检索词可恢复分页。
        </p>
      ) : null}

      <Card tight>
        {rows === null ? (
          <div style={{ padding: 18 }}>
            <Skeleton lines={7} />
          </div>
        ) : error ? (
          <div style={{ padding: 16 }}>
            <div className="note is-warn">
              <IconAlert size={14} />
              <div style={{ display: "grid", gap: 2 }}>
                <strong>读取内容库失败</strong>
                <span>{error}</span>
                <span className="faint">
                  如果内容库还是空的，先让任意一个信源完成一次采集；也可以暂时把检索词清空。
                </span>
              </div>
            </div>
          </div>
        ) : rows.length === 0 ? (
          <Empty
            title={
              param
                ? "没有找到这条资源"
                : query
                  ? "没有匹配的内容"
                  : kinds?.length
                    ? "这类内容还没有"
                    : "内容库还是空的"
            }
            text={
              param
                ? "它可能已被清理，或者 RID 不对；回到 Strata 页重新跳转试试。"
                : query
                  ? "试试更短的关键词；检索走的是内容库的全文与向量索引，命中需要内容已被发布。"
                  : kinds?.length
                    ? "切换到「全部」看看其他内容类型。"
                    : state === "ready"
                      ? "等待第一次采集完成。采集成功后，条目会带着快照序号出现在这里。"
                      : "先让采集进程连接成功，再回来看内容。"
            }
            action={
              state !== "ready" ? (
                <button type="button" className="btn btn-sm" onClick={() => navigate("sources")}>
                  去看信源
                </button>
              ) : null
            }
          />
        ) : (
          <div className="library-list">
            {rows.map((entry) => (
              <LibraryLine key={`${entry.variant}-${entry.row.rid}`} entry={entry} onOpen={setSelected} />
            ))}
          </div>
        )}
      </Card>

      {rows !== null && rows.length > 0 && !param ? (
        <Pagination
          page={page}
          hasPrev={hasPrev}
          hasNext={hasNext}
          onChange={(next) => (next < page ? goPrev() : goNext())}
          onFirst={() => {
            setAnchor(null);
            setPage(0);
          }}
        />
      ) : null}

      <ItemDrawer entry={selected} onClose={() => setSelected(null)} />
    </div>
  );
}

function LibraryLine({ entry, onOpen }: { entry: Entry; onOpen: (entry: Entry) => void }) {
  const label = kindTitle(entry.variant);
  return (
    <button type="button" className="library-row" onClick={() => onOpen(entry)}>
      <span className="library-text">
        <span className="library-title">{titleOf(entry)}</span>
        <span className="library-meta">
          <Pill tone="idle">{label}</Pill>
          {metaOf(entry).map((item, index) => (
            <span key={index} className="row" style={{ gap: 8 }}>
              {index > 0 ? <span className="sep" /> : null}
              <span>{item}</span>
            </span>
          ))}
        </span>
      </span>
      <span className="library-side">
        <span>#{entry.row.seq}</span>
        <span className="library-rid" title={entry.row.rid}>
          {entry.row.rid}
        </span>
      </span>
    </button>
  );
}

function ItemDrawer({ entry, onClose }: { entry: Entry | null; onClose: () => void }) {
  const { notify } = useConsole();
  const [readable, setReadable] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    setReadable(null);
    if (!entry) {
      return;
    }
    let alive = true;
    setLoading(true);
    contentBody(entry.row.rid)
      .then((text) => {
        if (alive && text) {
          setReadable(asReadable(text));
        }
      })
      .catch(() => undefined)
      .finally(() => {
        if (alive) {
          setLoading(false);
        }
      });
    return () => {
      alive = false;
    };
  }, [entry]);

  if (!entry) {
    return null;
  }

  const struct = structCell(entry.row, entry.variant) ?? {};
  const url = urlOf(entry);
  const hints = struct.hints as Record<string, unknown> | undefined;
  const published = typeof hints?.publishedAt === "string" ? hints.publishedAt : null;
  const summary = typeof hints?.summary === "string" ? hints.summary : null;

  return (
    <Drawer
      key={`${entry.row.rid}#${entry.row.seq}`}
      open
      kicker={`${kindTitle(entry.variant)} · 快照 #${entry.row.seq}`}
      title={titleOf(entry)}
      onClose={onClose}
      footer={
        <>
          {url ? (
            <a className="btn btn-primary" href={url} target="_blank" rel="noreferrer">
              打开原文
            </a>
          ) : null}
          <button
            type="button"
            className="btn"
            onClick={() => navigate(`strata/${entry.row.rid}`)}
            title="在 Strata 页查看这条内容的存储分层、版本与血缘"
          >
            查看分层
          </button>
          <button
            type="button"
            className="btn btn-quiet"
            onClick={() => {
              void navigator.clipboard?.writeText(entry.row.rid);
              notify("info", "已复制 RID");
            }}
          >
            复制 RID
          </button>
          <span className="spacer" />
          <span className="faint mono" style={{ fontSize: 10.5 }}>
            #{entry.row.seq}
          </span>
        </>
      }
    >
      <div className="row">
        <Pill tone="ok">{kindTitle(entry.variant)}</Pill>
        {published ? (
          <span className="faint" style={{ fontSize: 11.5 }}>
            {stamp(published)} · {relative(published)}
          </span>
        ) : null}
        {typeof hints?.author === "string" && hints.author ? (
          <span className="faint" style={{ fontSize: 11.5 }}>
            {hints.author}
          </span>
        ) : null}
      </div>

      {loading ? <Skeleton lines={6} /> : null}

      <div className="reader">
        {summary ? <div className="note">{summary}</div> : null}
        <SectionTitle title="正文" note={readable ? "抓取到的网页文本" : "尚未抓到正文"} />
        {readable ? (
          <div className="reader-body">{readable}</div>
        ) : loading ? null : (
          <Empty
            title="还没有可读正文"
            text="这条内容目前只有结构化字段；正文会在详情抓取完成后出现。"
          />
        )}
      </div>

      <div>
        <SectionTitle title="结构化字段" />
        <pre className="code">{JSON.stringify(struct, null, 2)}</pre>
      </div>

      <p className="faint" style={{ fontSize: 11, lineHeight: 1.6 }}>
        <IconLibrary size={11} /> 内容按 RID 归并，每次提交产生新版本；点「查看分层」可以在 Strata
        页看到这条内容的原文、正文与结构化各层，以及它们的版本与血缘。
      </p>
    </Drawer>
  );
}

/** 行里第一个带内容的类型，就是这条内容的类型。 */
function variantOf(row: LibraryRow, known: string[]): string {
  const found = known.find((name) => row[`struct:${name}`] !== undefined);
  return found ?? known[0] ?? "discovery";
}

function kindTitle(variant: string): string {
  const table: Record<string, string> = {
    discovery: "采集条目",
    article: "文章",
    "hotlist.snapshot": "热榜快照",
    hotlist: "热榜",
  };
  return table[variant] ?? variant;
}

function titleOf(entry: Entry): string {
  const struct = structCell(entry.row, entry.variant) ?? {};
  const hints = struct.hints as Record<string, unknown> | undefined;
  if (typeof hints?.title === "string" && hints.title.trim()) {
    return hints.title;
  }
  if (typeof struct.title === "string" && struct.title.trim()) {
    return struct.title;
  }
  return urlOf(entry) ?? entry.row.rid;
}

function urlOf(entry: Entry): string | null {
  const struct = structCell(entry.row, entry.variant) ?? {};
  const locator = struct.locator as Record<string, unknown> | undefined;
  if (typeof locator?.url === "string") {
    return locator.url;
  }
  if (typeof struct.url === "string") {
    return struct.url;
  }
  return null;
}

function metaOf(entry: Entry): string[] {
  const struct = structCell(entry.row, entry.variant) ?? {};
  const hints = struct.hints as Record<string, unknown> | undefined;
  const items: string[] = [];
  if (typeof hints?.author === "string" && hints.author.trim()) {
    items.push(hints.author);
  }
  if (typeof hints?.publishedAt === "string") {
    items.push(relative(hints.publishedAt));
  }
  if (typeof hints?.summary === "string" && hints.summary.trim()) {
    items.push(hints.summary.slice(0, 60));
  }
  if (!items.length) {
    const url = urlOf(entry);
    if (url) {
      items.push(url);
    }
  }
  return items.slice(0, 3);
}
