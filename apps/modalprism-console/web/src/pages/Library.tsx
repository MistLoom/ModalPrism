import { useEffect, useMemo, useState } from "react";

import { BatchBar } from "../components/batch";
import { Drawer } from "../components/Drawer";
import { IconAlert, IconLibrary, IconRefresh, IconTrash } from "../components/icons";
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
import { contentBody, contentCount, contentKinds, contentPurge, contentRestore, contentTag, library, strataSummary, structCell } from "../lib/api";
import { useConsole } from "../lib/console";
import { asReadable, count, relative, stamp } from "../lib/format";
import { navigate, useRoute } from "../lib/router";
import { useSelection } from "../lib/selection";
import type { ContentKind, ContentSummary, LibraryRow } from "../lib/types";

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

/** 加工层级：按 facet 存在性区分原始/可读/结构化；加工产出按 workflow 标签判定。 */
const LEVEL_OPTIONS = [
  { value: "all", label: "全部分层" },
  { value: "raw", label: "原始层" },
  { value: "md", label: "可读层" },
  { value: "struct", label: "结构化" },
  { value: "processed", label: "加工产出" },
];

export function LibraryPage() {
  const { snapshot, cursor, state, notify } = useConsole();
  const { param } = useRoute();
  const [kinds, setKinds] = useState<ContentKind[] | null>(null);
  const [summary, setSummary] = useState<ContentSummary | null>(null);
  const [kind, setKind] = useState("all");
  const [text, setText] = useState("");
  const [query, setQuery] = useState("");
  const [sourceId, setSourceId] = useState("all");
  const [span, setSpan] = useState("all");
  const [body, setBody] = useState("any");
  const [level, setLevel] = useState("all");
  const [category, setCategory] = useState("all");
  const [withTombstoned, setWithTombstoned] = useState(false);
  const [page, setPage] = useState(0);
  // keyset 锚点：next 向更早翻（seq < 锚点）；prev 向更新翻（seq > 锚点，升序拉取后反转）。
  const [anchor, setAnchor] = useState<{ seq: number; dir: "next" | "prev" } | null>(null);
  const [rows, setRows] = useState<Entry[] | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Entry | null>(null);
  const [busy, setBusy] = useState(false);
  const [version, setVersion] = useState(0);
  const selection = useSelection<string>();

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
  }, [version]);

  // 来源分布由服务端聚合：只统计真实存在的内容，不受分页影响。
  useEffect(() => {
    let alive = true;
    strataSummary()
      .then((next) => {
        if (alive) {
          setSummary(next);
        }
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [version]);

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
    // 标签值里可能含 =，只按第一个分隔符拆。
    const split = category === "all" ? null : (() => {
      const at = category.indexOf("=");
      return at < 0 ? null : { key: category.slice(0, at), value: category.slice(at + 1) };
    })();
    library({
      variant: kind === "all" ? null : kind,
      variants: known,
      search: query,
      snapshot: cursor,
      limit: PAGE_SIZE,
      sourceId: sourceId === "all" ? null : sourceId,
      since: spanMs > 0 ? new Date(Date.now() - spanMs).toISOString() : null,
      withBody: body === "with",
      includeTombstoned: withTombstoned,
      level: level === "all" ? null : level,
      processedOnly: level === "processed",
      label: split,
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
  }, [kind, query, cursor, kinds, sourceId, span, body, level, category, withTombstoned, anchor, param, version]);

  // hasMore 描述的是本次拉取的方向：向上翻时它说明「更新的方向」还有没有。
  const hasPrev = param ? false : anchor?.dir === "prev" ? hasMore : page > 0;
  const hasNext = param ? false : anchor?.dir === "prev" ? true : hasMore;
  const total = useMemo(() => rows?.length ?? 0, [rows]);

  // 来源选项来自服务端聚合：只列真实有内容的来源，名字取自信源清单，条数直接标注。
  const sourceOptions = useMemo(() => {
    const names = new Map((snapshot?.sources ?? []).map((item) => [item.id, item.name]));
    const totals = new Map((summary?.sources ?? []).map((item) => [item.sourceId, item.total]));
    const ids = new Set<string>([...totals.keys(), ...names.keys()]);
    return [
      { value: "all", label: "全部来源" },
      ...[...ids]
        .filter(Boolean)
        .sort()
        .map((id) => {
          const name = names.get(id) ?? shortId(id);
          const total = totals.get(id);
          return { value: id, label: total === undefined ? name : `${name} · ${count(total)}` };
        }),
    ];
  }, [summary, snapshot]);

  // 分类选项来自服务端标签分布：只列真实用过的分类，工作流内部标签不展示给用户。
  const categoryOptions = useMemo(() => {
    const groups = new Map<string, { value: string; total: number }[]>();
    for (const item of summary?.labels ?? []) {
      if (item.key === "workflow") {
        continue;
      }
      const bucket = groups.get(item.key) ?? [];
      bucket.push({ value: item.value, total: item.total });
      groups.set(item.key, bucket);
    }
    const options = [...groups.entries()]
      .sort(([left], [right]) => left.localeCompare(right))
      .flatMap(([key, items]) =>
        items
          .sort((left, right) => right.total - left.total || left.value.localeCompare(right.value))
          .map((item) => ({ value: `${key}=${item.value}`, label: `${key} · ${item.value} · ${count(item.total)}` })),
      );
    return [{ value: "all", label: "全部分类" }, ...options];
  }, [summary]);

  const picked = rows?.filter((entry) => selection.has(entry.row.rid)).length ?? 0;
  const sourceIdFilter = sourceId === "all" ? null : sourceId;

  /** 删除写入墓碑、恢复撤销墓碑：两者都作用在一组 RID 上。 */
  async function act(kind: "tombstone" | "restore", rids: string[]) {
    if (rids.length === 0) {
      return;
    }
    if (
      kind === "tombstone" &&
      !window.confirm(`将选中的 ${rids.length} 条内容写入墓碑？不会物理删除，勾选「包含已删除」后可以在这里恢复。`)
    ) {
      return;
    }
    setBusy(true);
    try {
      if (kind === "tombstone") {
        const report = await contentPurge({ rids, reason: "控制台删除", dryRun: false });
        notify("ok", `已删除 ${report.tombstoned} 条内容`, "默认列表不再显示；勾选「包含已删除」可以找回。");
      } else {
        const restored = await contentRestore(rids, "控制台恢复");
        notify("ok", `已恢复 ${restored} 条内容`);
      }
      selection.clear();
      setSelected(null);
      setRows(null);
      setVersion((current) => current + 1);
    } catch (problem) {
      notify("bad", kind === "tombstone" ? "删除失败" : "恢复失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  /** 按当前筛选一次清理：时间范围在这里是清理边界（早于窗口的内容才被写墓碑）。 */
  async function purgeFiltered() {
    const spanMs = SPANS.find((item) => item.value === span)?.ms ?? 0;
    if (span === "all" && !sourceIdFilter) {
      notify("info", "先把来源或时间范围收窄到一个明确的清理边界", "清理是批量写墓碑，避免「全部时间 + 全部来源」一刀切。");
      return;
    }
    if (query.trim()) {
      notify("info", "检索模式下不能按筛选清理", "清空检索词后，来源与时间筛选会重新生效。");
      return;
    }
    setBusy(true);
    try {
      const scope = {
        sourceId: sourceIdFilter,
        ...(spanMs > 0 ? { before: new Date(Date.now() - spanMs).toISOString() } : {}),
      };
      const stats = await contentCount({ ...scope, includeTombstoned: true });
      const pending = stats.matched - stats.tombstoned;
      if (pending <= 0) {
        notify("info", "当前筛选下没有待删除的内容", stats.matched > 0 ? "匹配到的条目都已写入墓碑。" : undefined);
        return;
      }
      const boundary = [
        sourceIdFilter ? `来源 ${shortId(sourceIdFilter)}` : null,
        spanMs > 0 ? `入库早于 ${SPANS.find((item) => item.value === span)?.label.replace("最近 ", "")}以前` : null,
      ]
        .filter(Boolean)
        .join("、");
      if (
        !window.confirm(
          `将给${boundary}下未删除的 ${pending} 条内容写入墓碑？不会物理删除，勾选「包含已删除」后可以找回。继续？`,
        )
      ) {
        return;
      }
      let tombstoned = 0;
      for (;;) {
        const report = await contentPurge({ ...scope, reason: "控制台按筛选清理", dryRun: false });
        tombstoned += report.tombstoned;
        if (!report.truncated || report.tombstoned === 0) {
          break;
        }
      }
      notify("ok", `已清理 ${tombstoned} 条内容`, "默认列表不再显示；勾选「包含已删除」可以找回。");
      selection.clear();
      setSelected(null);
      setRows(null);
      setVersion((current) => current + 1);
    } catch (problem) {
      notify("bad", "清理失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  function resetPage() {
    setPage(0);
    setAnchor(null);
  }

  /** 打标/摘标后统一刷新列表与抽屉：标签同时是筛选维度。 */
  async function writeTags(
    targets: Entry[],
    set: Record<string, string>,
    unset: string[],
    done: string,
  ) {
    const rids = targets.map((entry) => entry.row.rid);
    if (rids.length === 0) {
      return;
    }
    setBusy(true);
    try {
      await contentTag(rids, set, unset);
      notify("ok", done);
      selection.clear();
      setSelected(null);
      setRows(null);
      setVersion((current) => current + 1);
    } catch (problem) {
      notify("bad", "标签没有写入", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
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
              ...kinds.map((item) => ({
                value: item.variant,
                label: `${item.label} · ${count(item.total)}`,
              })),
            ]}
          />
        ) : null}
        <span className="spacer" />
        {cursor !== null ? (
          <Pill tone="warn">历史快照 #{cursor}</Pill>
        ) : (
          <span className="faint" style={{ fontSize: "var(--fs-xs)" }}>
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
            options={sourceOptions}
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
          <FilterSelect
            label="分层"
            value={level}
            onChange={(value) => {
              setLevel(value);
              resetPage();
            }}
            options={LEVEL_OPTIONS}
            disabled={query.trim() !== ""}
            title={query.trim() ? "检索模式下不能按分层筛选，清空检索词后恢复" : undefined}
          />
          <FilterSelect
            label="分类"
            value={category}
            onChange={(value) => {
              setCategory(value);
              resetPage();
            }}
            options={categoryOptions}
          />
          <label className="pick-inline" title="已删除的内容只写了墓碑，勾选后可以找回">
            <input
              type="checkbox"
              checked={withTombstoned}
              onChange={(event) => {
                setWithTombstoned(event.target.checked);
                resetPage();
              }}
            />
            包含已删除
          </label>
        </div>
      ) : null}
      {query.trim() ? (
        <p className="faint" style={{ fontSize: "var(--fs-xs)", margin: 0 }}>
          检索模式按相关度返回前 {PAGE_SIZE} 条，不支持翻页与正文筛选；清空检索词可恢复分页。
        </p>
      ) : null}

      <BatchBar
        count={picked}
        busy={busy}
        onClear={selection.clear}
        actions={[
          {
            key: "remove",
            label: "删除",
            icon: <IconTrash size={12} />,
            danger: true,
            onClick: () => void act("tombstone", selection.list()),
          },
          {
            key: "restore",
            label: "恢复",
            icon: <IconRefresh size={12} />,
            onClick: () => void act("restore", selection.list()),
          },
          ...(param || query.trim()
            ? []
            : [
                {
                  key: "purge-filtered",
                  label: "按筛选清理",
                  icon: <IconAlert size={12} />,
                  danger: true,
                  onClick: () => void purgeFiltered(),
                },
              ]),
        ]}
      />

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
              <LibraryLine
                key={`${entry.variant}-${entry.row.rid}`}
                entry={entry}
                picked={selection.has(entry.row.rid)}
                onPick={(rid) => selection.toggle(rid)}
                onOpen={setSelected}
              />
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

      <ItemDrawer entry={selected} busy={busy} onClose={() => setSelected(null)} onAct={act} onTags={writeTags} />
    </div>
  );
}

function LibraryLine({
  entry,
  picked,
  onPick,
  onOpen,
}: {
  entry: Entry;
  picked: boolean;
  onPick: (rid: string) => void;
  onOpen: (entry: Entry) => void;
}) {
  const label = kindTitle(entry.variant);
  const source = entry.row.sourceId ?? "";
  const ns = nsBadge(entry.row.ns);
  return (
    <div
      role="button"
      tabIndex={0}
      className="library-row"
      onClick={() => onOpen(entry)}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          onOpen(entry);
        }
      }}
    >
      <span className="pick-inline" onClick={(event) => event.stopPropagation()}>
        <input
          type="checkbox"
          aria-label={`选择 ${entry.row.rid}`}
          checked={picked}
          onChange={() => onPick(entry.row.rid)}
        />
      </span>
      <span className="library-text">
        <span className="library-title">{titleOf(entry)}</span>
        <span className="library-meta">
          <Pill tone="idle">{label}</Pill>
          {ns ? <Pill tone="idle">{ns}</Pill> : null}
          {entry.row.tombstone ? <Pill tone="warn">已删除</Pill> : null}
          {entry.row.labels?.workflow ? (
            <Pill tone="info">加工程序 · {entry.row.labels.workflow}</Pill>
          ) : null}
          {Object.entries(entry.row.labels ?? {})
            .filter(([key]) => key !== "workflow")
            .slice(0, 3)
            .map(([key, value]) => (
              <span key={key} className="tag">
                {key} · {value}
              </span>
            ))}
          {source ? (
            <button
              type="button"
              className="tag"
              title={`来自 ${source}`}
              onClick={(event) => {
                event.stopPropagation();
                navigate(`sources/${encodeURIComponent(source)}`);
              }}
            >
              来源 · {shortId(source)}
            </button>
          ) : null}
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
    </div>
  );
}

function ItemDrawer({
  entry,
  busy,
  onClose,
  onAct,
  onTags,
}: {
  entry: Entry | null;
  busy: boolean;
  onClose: () => void;
  onAct: (kind: "tombstone" | "restore", rids: string[]) => void;
  onTags: (targets: Entry[], set: Record<string, string>, unset: string[], done: string) => Promise<void>;
}) {
  const { notify } = useConsole();
  const [readable, setReadable] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [tagText, setTagText] = useState("");

  const writeTags = (set: Record<string, string>, unset: string[], done: string) =>
    onTags([entry as Entry], set, unset, done);

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
          {entry.row.tombstone ? (
            <button type="button" className="btn" disabled={busy} onClick={() => onAct("restore", [entry.row.rid])}>
              <IconRefresh size={12} /> 恢复
            </button>
          ) : (
            <button type="button" className="btn btn-danger" disabled={busy} onClick={() => onAct("tombstone", [entry.row.rid])}>
              <IconTrash size={12} /> 删除
            </button>
          )}
          <span className="spacer" />
          <span className="faint mono" style={{ fontSize: "var(--fs-2xs)" }}>
            #{entry.row.seq}
          </span>
        </>
      }
    >
      <div className="row">
        <Pill tone="ok">{kindTitle(entry.variant)}</Pill>
        {published ? (
          <span className="faint" style={{ fontSize: "var(--fs-xs)" }}>
            {stamp(published)} · {relative(published)}
          </span>
        ) : null}
        {typeof hints?.author === "string" && hints.author ? (
          <span className="faint" style={{ fontSize: "var(--fs-xs)" }}>
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

      <div>
        <SectionTitle title="分类标签" note="用户分类；工作流产出带 workflow 标签" />
        <div className="row" style={{ gap: 6, flexWrap: "wrap" }}>
          {Object.entries(entry.row.labels ?? {}).map(([key, value]) => (
            <Pill key={key} tone={key === "workflow" ? "info" : "idle"}>
              {key === "workflow" ? `加工 · ${value}` : `${key} · ${value}`}
              {key !== "workflow" ? (
                <button
                  type="button"
                  className="btn btn-sm btn-quiet"
                  style={{ marginLeft: 4 }}
                  disabled={busy}
                  onClick={() => void writeTags({}, [key], `已撤销 ${key}`)}
                  title="撤销这个标签"
                >
                  ×
                </button>
              ) : null}
            </Pill>
          ))}
          {Object.keys(entry.row.labels ?? {}).filter((key) => key !== "workflow").length === 0 ? (
            <span className="faint" style={{ fontSize: "var(--fs-xs)" }}>
              还没有分类标签。
            </span>
          ) : null}
        </div>
        <div className="row" style={{ gap: 6, marginTop: 8 }}>
          <input
            type="text"
            className="form-input mono"
            style={{ maxWidth: 260 }}
            value={tagText}
            onChange={(event) => setTagText(event.target.value)}
            placeholder="分类=值，如 topic=AI"
            aria-label="新标签"
          />
          <button
            type="button"
            className="btn btn-sm"
            disabled={busy || !tagText.includes("=")}
            onClick={() => {
              const at = tagText.indexOf("=");
              const key = tagText.slice(0, at).trim();
              const value = tagText.slice(at + 1).trim();
              if (key && value) {
                void writeTags({ [key]: value }, [], `已打上 ${key}=${value}`);
                setTagText("");
              }
            }}
          >
            打标
          </button>
        </div>
      </div>

      <p className="faint" style={{ fontSize: "var(--fs-2xs)", lineHeight: 1.6 }}>
        <IconLibrary size={11} /> 内容按 RID 归并，每次提交产生新版本；点「查看分层」可以在 Strata
        页看到这条内容的原文、正文与结构化各层，以及它们的版本与血缘。
      </p>
    </Drawer>
  );
}

/** 命名空间只读徽标：采集默认层不标注，其余空间（如工作流产出）显式区分。 */
function nsBadge(ns: string | undefined): string | null {
  if (!ns || ns === "data" || ns === "default") {
    return null;
  }
  return `空间 · ${ns}`;
}

/** 去掉 `source/` 之类前缀，界面上只显示短名字。 */
function shortId(id: string): string {
  const index = id.indexOf("/");
  return index >= 0 ? id.slice(index + 1) : id;
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
