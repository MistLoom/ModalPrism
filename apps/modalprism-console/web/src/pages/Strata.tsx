// Strata 页：从资源出发看内容分层——facet、版本与血缘。
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { IconBranch, IconCopy, IconTrash } from "../components/icons";
import { Card, Empty, Pagination, SearchInput, Skeleton } from "../components/ui";
import {
  committerStatus,
  contentKinds,
  facet,
  facetHistory,
  library,
  strataKinds,
  strataLineage,
  strataRelated,
  structCell,
  sweep,
  type LibraryQuery,
} from "../lib/api";
import { asReadable, bytes, count, relative } from "../lib/format";
import { useConsole } from "../lib/console";
import { navigate, useRoute } from "../lib/router";
import type {
  CommitterStatus,
  ContentKind,
  FacetBody,
  FacetSummary,
  LibraryRow,
  Lineage,
  RelatedEdge,
  StrataVersion,
  SweepReport,
} from "../lib/types";

type FacetKey = { kind: string; variant: string | null };

const PAGE_SIZE = 40;

function facetRef(rid: string, key: FacetKey, version: number | "latest"): string {
  const variant = key.variant ? `:${key.variant}` : "";
  const selector = version === "latest" ? "latest" : `v${version}`;
  return `${rid}@${key.kind}${variant}#${selector}`;
}

function facetLabel(key: FacetKey): string {
  const names: Record<string, string> = {
    struct: "结构化",
    html: "网页",
    text: "正文",
    md: "可读稿",
    raw: "原始",
    json: "JSON",
    yaml: "YAML",
    links: "链接",
    image: "图片",
    xml: "XML",
    audio: "音频",
    video: "视频",
  };
  const base = names[key.kind] ?? key.kind;
  return key.variant ? `${base} · ${key.variant}` : base;
}

function rowTitle(row: LibraryRow, variants: string[]): { title: string; kind: string | null } {
  for (const variant of variants) {
    const struct = structCell(row, variant);
    if (!struct) {
      continue;
    }
    const hints = struct.hint as Record<string, unknown> | undefined;
    const title =
      (typeof hints?.title === "string" && hints.title.trim()) ||
      (typeof struct.title === "string" && struct.title.trim()) ||
      "";
    if (title) {
      return { title, kind: variant };
    }
  }
  return { title: row.rid, kind: variants[0] ?? null };
}

export function StrataPage() {
  const { state } = useConsole();
  const { param } = useRoute();
  const [kinds, setKinds] = useState<ContentKind[] | null>(null);
  const [variant, setVariant] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [rows, setRows] = useState<LibraryRow[] | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [page, setPage] = useState(0);
  // keyset 锚点：next 向更早翻（seq < 锚点）；prev 向更新翻（seq > 锚点，升序拉取后反转）。
  const [anchor, setAnchor] = useState<{ seq: number; dir: "next" | "prev" } | null>(null);
  const [selected, setSelected] = useState<LibraryRow | null>(null);
  const [facets, setFacets] = useState<FacetSummary[] | null>(null);
  const [facetsError, setFacetsError] = useState<string | null>(null);
  const [activeFacet, setActiveFacet] = useState<FacetKey | null>(null);
  const [versions, setVersions] = useState<StrataVersion[] | null>(null);
  const [version, setVersion] = useState<number | "latest">("latest");
  const [body, setBody] = useState<FacetBody | null>(null);
  const [view, setView] = useState<"text" | "json">("text");
  const [lineage, setLineage] = useState<Lineage | null>(null);
  const [related, setRelated] = useState<RelatedEdge[] | null>(null);
  const searchTimer = useRef(0);

  useEffect(() => {
    contentKinds()
      .then(setKinds)
      .catch(() => setKinds([]));
  }, []);

  // 筛选或路径参数变化时回到第一页；keyset 锚点只对同一份筛选有意义。
  useEffect(() => {
    setAnchor(null);
    setPage(0);
  }, [param, variant, search]);

  const loadRows = useCallback(async () => {
    const known = kinds?.map((item) => item.variant) ?? [];
    const query: LibraryQuery = {
      variant,
      variants: variant ? [variant] : known,
      search,
      limit: PAGE_SIZE,
      before: anchor?.dir === "next" ? anchor.seq : null,
      after: anchor?.dir === "prev" ? anchor.seq : null,
      rid: param,
    };
    try {
      const result = await library(query);
      setRows(result.rows);
      setHasMore(result.hasMore);
      // 从内容库带着 RID 跳来时，自动选中这条资源。
      if (param) {
        const hit = result.rows.find((row) => row.rid === param);
        if (hit) {
          setSelected(hit);
        }
      }
    } catch {
      setRows([]);
    }
  }, [kinds, variant, search, anchor, param]);

  useEffect(() => {
    window.clearTimeout(searchTimer.current);
    searchTimer.current = window.setTimeout(() => void loadRows(), search ? 260 : 0);
    return () => window.clearTimeout(searchTimer.current);
  }, [loadRows, search]);

  useEffect(() => {
    if (!selected) {
      setFacets(null);
      setActiveFacet(null);
      return;
    }
    let alive = true;
    setFacets(null);
    setFacetsError(null);
    setActiveFacet(null);
    setBody(null);
    setVersions(null);
    setLineage(null);
    setRelated(null);
    strataKinds(selected.rid)
      .then((list) => {
        if (!alive) {
          return;
        }
        setFacets(list);
        const first = list.find((item) => item.kind === "struct") ?? list[0];
        if (first) {
          setActiveFacet({ kind: first.kind, variant: first.variant });
        }
      })
      .catch((error: unknown) => {
        if (alive) {
          setFacets([]);
          setFacetsError(error instanceof Error ? error.message : "读取数据分层失败");
        }
      });
    return () => {
      alive = false;
    };
  }, [selected]);

  useEffect(() => {
    if (!selected || !activeFacet) {
      return;
    }
    let alive = true;
    setVersions(null);
    setVersion("latest");
    facetHistory(facetRef(selected.rid, activeFacet, "latest"))
      .then((list) => {
        if (alive) {
          setVersions(list);
        }
      })
      .catch(() => {
        if (alive) {
          setVersions([]);
        }
      });
    return () => {
      alive = false;
    };
  }, [selected, activeFacet]);

  useEffect(() => {
    if (!selected || !activeFacet) {
      return;
    }
    let alive = true;
    const ref = facetRef(selected.rid, activeFacet, version);
    setBody(null);
    facet(ref)
      .then((data) => {
        if (alive) {
          setBody(data);
        }
      })
      .catch(() => {
        if (alive) {
          setBody(null);
        }
      });
    strataLineage(ref)
      .then((graph) => {
        if (alive) {
          setLineage(graph);
        }
      })
      .catch(() => {
        if (alive) {
          setLineage(null);
        }
      });
    strataRelated(selected.rid)
      .then((list) => {
        if (alive) {
          setRelated(list);
        }
      })
      .catch(() => {
        if (alive) {
          setRelated([]);
        }
      });
    return () => {
      alive = false;
    };
  }, [selected, activeFacet, version]);

  // hasMore 描述的是本次拉取的方向：向上翻时它说明「更新的方向」还有没有。
  const hasPrev = param ? false : anchor?.dir === "prev" ? hasMore : page > 0;
  const hasNext = param ? false : anchor?.dir === "prev" ? true : hasMore;

  function goNext() {
    const last = rows?.[rows.length - 1]?.seq;
    if (last === undefined) {
      return;
    }
    setAnchor({ seq: last, dir: "next" });
    setPage(page + 1);
  }

  function goPrev() {
    const first = rows?.[0]?.seq;
    if (first === undefined) {
      return;
    }
    setAnchor({ seq: first, dir: "prev" });
    setPage(page - 1);
  }

  const variantOptions = useMemo(() => kinds ?? [], [kinds]);
  const variants = useMemo(() => variantOptions.map((item) => item.variant), [variantOptions]);
  const totalFacets = useMemo(
    () => variantOptions.reduce((sum, item) => sum + item.total, 0),
    [variantOptions],
  );

  if (state === "loading" && rows === null && kinds === null) {
    return (
      <div className="page">
        <Skeleton lines={7} />
      </div>
    );
  }

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <span className="page-kicker">strata · 内容分层</span>
          <h1 className="page-title">
            内容<em>分层</em>
          </h1>
          <p className="page-lede">
            Strata 是存储的工程视图：每条资源按 facet 分层存放——原文、可读正文、结构化事实各存一份，互不覆盖，
            这里能看到每层的版本历史与产出血缘。只想阅读内容的话，去内容库页，两边用 RID 互链。
          </p>
        </div>
        <div className="page-actions">
          <span className="pill pill-idle">
            {count(totalFacets)} 个结构化 facet · {variantOptions.length} 类
          </span>
        </div>
      </header>

      <RetentionCard />

      {param ? (
        <div className="note">
          <div style={{ display: "grid", gap: 2 }}>
            <strong>正在定位单条资源</strong>
            <span>
              从内容库跳转而来，左侧列表已按 RID 过滤。
              <button
                type="button"
                className="btn btn-sm"
                style={{ marginLeft: 8 }}
                onClick={() => navigate("strata")}
              >
                返回完整列表
              </button>
            </span>
          </div>
        </div>
      ) : null}

      <div className="chips-row">
        <button
          type="button"
          className={`chip${variant === null ? " is-active" : ""}`}
          aria-pressed={variant === null}
          onClick={() => setVariant(null)}
        >
          全部 <span className="chip-n">{count(totalFacets)}</span>
        </button>
        {variantOptions.map((item) => (
          <button
            key={item.variant}
            type="button"
            className={`chip${variant === item.variant ? " is-active" : ""}`}
            aria-pressed={variant === item.variant}
            onClick={() => setVariant(item.variant === variant ? null : item.variant)}
          >
            {item.label} <span className="chip-n">{count(item.total)}</span>
          </button>
        ))}
      </div>

      <div className="strata-layout">
        <Card
          title="资源"
          sub={`${rows?.length ?? 0} 条${search ? "匹配结果" : "最近入库"}`}
          tight
        >
          <SearchInput
            value={search}
            onChange={setSearch}
            label="搜索内容"
            placeholder="全文搜索标题、正文…"
          />
          <div className="strata-list">
            {rows === null ? (
              <Skeleton lines={5} />
            ) : rows.length === 0 ? (
              <Empty
                title="还没有内容"
                text={search ? "换个关键词试试，或清空筛选。" : "等采集进程跑完一轮，这里就会出现内容。"}
              />
            ) : (
              rows.map((row) => {
                const { title, kind } = rowTitle(row, variants);
                const isActive = selected?.rid === row.rid;
                return (
                  <button
                    key={row.rid}
                    type="button"
                    className={`resource-item${isActive ? " is-active" : ""}`}
                    aria-pressed={isActive}
                    onClick={() => setSelected(row)}
                  >
                    <span className="resource-title">{title}</span>
                    <span className="resource-meta">
                      {kind ? <span className="tag tag-muted">{kind}</span> : null}
                      <span>#{row.seq}</span>
                    </span>
                  </button>
                );
              })
            )}
          </div>
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
        </Card>

        <div className="strata-detail">
          {selected ? (
            <FacetDetail
              row={selected}
              variants={variants}
              facets={facets}
              facetsError={facetsError}
              activeFacet={activeFacet}
              onSelectFacet={(key) => {
                setActiveFacet(key);
                setView("text");
              }}
              versions={versions}
              version={version}
              onSelectVersion={setVersion}
              body={body}
              view={view}
              onView={setView}
              lineage={lineage}
              related={related}
            />
          ) : (
            <Empty
              title="选择一条资源"
              text="左侧列表里点一条，右侧会展开它的分层：有哪些 facet、每个 facet 的版本历史、以及这次产出的血缘与关系。"
            />
          )}
        </div>
      </div>
    </div>
  );
}

function FacetDetail({
  row,
  variants,
  facets,
  facetsError,
  activeFacet,
  onSelectFacet,
  versions,
  version,
  onSelectVersion,
  body,
  view,
  onView,
  lineage,
  related,
}: {
  row: LibraryRow;
  variants: string[];
  facets: FacetSummary[] | null;
  facetsError: string | null;
  activeFacet: FacetKey | null;
  onSelectFacet: (key: FacetKey) => void;
  versions: StrataVersion[] | null;
  version: number | "latest";
  onSelectVersion: (value: number | "latest") => void;
  body: FacetBody | null;
  view: "text" | "json";
  onView: (value: "text" | "json") => void;
  lineage: Lineage | null;
  related: RelatedEdge[] | null;
}) {
  const { notify } = useConsole();
  const { title } = rowTitle(row, variants);
  // 长内容展开查看，不受预览框高度限制。
  const [expanded, setExpanded] = useState(false);

  // 网页层的 text 是原始 HTML，转成可读文本再看；其他层原样展示。
  const rawText = body?.text ?? null;
  const text = rawText !== null && activeFacet?.kind === "html" ? asReadable(rawText) : rawText;
  const json = body?.struct ?? null;
  const preview =
    view === "text"
      ? (text ?? (json ? JSON.stringify(json, null, 2) : null))
      : json
        ? JSON.stringify(json, null, 2)
        : (text ?? null);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(preview ?? row.rid);
      notify("ok", "已复制到剪贴板");
    } catch {
      notify("bad", "复制失败", "浏览器拒绝了剪贴板访问");
    }
  };

  return (
    <div className="detail-stack">
      <Card
        title={title}
        sub={row.rid}
        action={
          <span className="btn-row">
            <button
              type="button"
              className="btn btn-quiet btn-sm"
              onClick={() => navigate(`library/${row.rid}`)}
              title="在内容库查看这条内容的可读正文与结构化字段"
            >
              在内容库查看
            </button>
            <button type="button" className="btn btn-quiet btn-sm" onClick={copy}>
              <IconCopy size={13} /> 复制 rid
            </button>
          </span>
        }
      >
        <div className="facet-rail">
          {facets === null ? (
            <Skeleton lines={2} />
          ) : facetsError ? (
            <span className="faint">读取分层失败：{facetsError}</span>
          ) : facets.length === 0 ? (
            <span className="faint">这条资源还没有 facet。</span>
          ) : (
            facets.map((item) => {
              const key = { kind: item.kind, variant: item.variant };
              const active = activeFacet?.kind === item.kind && activeFacet?.variant === item.variant;
              return (
                <button
                  key={`${item.kind}:${item.variant ?? ""}`}
                  type="button"
                  className={`facet-btn${active ? " is-active" : ""}`}
                  aria-pressed={active}
                  onClick={() => onSelectFacet(key)}
                  title={`${item.versions} 个版本 · 最新 v${item.latest}`}
                >
                  {facetLabel(key)} <b>v{item.latest}</b>
                </button>
              );
            })
          )}
        </div>

        {activeFacet ? (
          <div className="version-strip">
            <span className="vs-label">版本</span>
            <button
              type="button"
              className={`version-btn${version === "latest" ? " is-active" : ""}`}
              aria-pressed={version === "latest"}
              onClick={() => onSelectVersion("latest")}
            >
              最新
            </button>
            {(versions ?? []).map((item) => (
              <button
                key={item.version}
                type="button"
                className={`version-btn${version === item.version ? " is-active" : ""}`}
                aria-pressed={version === item.version}
                onClick={() => onSelectVersion(item.version)}
                title={`${item.contentHash} · ${relative(item.createdAt)}`}
              >
                v{item.version}
              </button>
            ))}
            {versions !== null && versions.length === 0 ? (
              <span className="faint">这个 facet 只有一条记录，没有历史可回放。</span>
            ) : null}
            {body ? (
              <span className="version-meta mono">
                {body.mediaType ?? "—"} · {bytes(body.size)} · hash {body.contentHash.slice(0, 10)}
              </span>
            ) : null}
          </div>
        ) : null}
      </Card>

      <Card
        title="预览"
        action={
          <span className="btn-row">
            <span className="seg">
              <button
                type="button"
                className={view === "text" ? "is-active" : ""}
                aria-pressed={view === "text"}
                onClick={() => onView("text")}
              >
                正文
              </button>
              <button
                type="button"
                className={view === "json" ? "is-active" : ""}
                aria-pressed={view === "json"}
                onClick={() => onView("json")}
              >
                结构
              </button>
            </span>
            <button
              type="button"
              className="btn btn-quiet btn-sm"
              onClick={() => setExpanded(!expanded)}
            >
              {expanded ? "收起" : "展开全部"}
            </button>
            <button type="button" className="btn btn-quiet btn-sm" onClick={copy}>
              <IconCopy size={13} /> 复制
            </button>
          </span>
        }
        tight
      >
        <pre className={`code-view${expanded ? " is-expanded" : ""}`}>
          {preview ?? "这个 facet 没有可预览的内容，可能只有二进制正文。"}
        </pre>
      </Card>

      <Card title="血缘" sub="这次产出依赖了哪些上游 facet" tight>
        {lineage === null ? (
          <Skeleton lines={3} />
        ) : lineage.nodes.length === 0 ? (
          <Empty title="没有血缘" text="这个 facet 由采集直接产出，没有上游依赖。" />
        ) : (
          <div className="lineage-scroll">
            <ul className="lineage-list">
              {lineage.nodes.map((node) => {
                const active = node.facet === facetRef(row.rid, activeFacet!, version);
                return (
                  <li key={node.facet} className={`lineage-item${active ? " is-active" : ""}`}>
                    <IconBranch size={13} />
                    <span className="lineage-facet mono">{node.facet}</span>
                    {node.module ? <span className="tag tag-muted">{node.module}</span> : null}
                    {node.version !== null ? <span className="faint mono">v{node.version}</span> : null}
                  </li>
                );
              })}
            </ul>
            {lineage.edges.length ? (
              <p className="faint lineage-note">
                {lineage.edges.map((edge) => `${edge.from} → ${edge.to}`).join("；")}
              </p>
            ) : null}
          </div>
        )}
      </Card>

      <Card title="关系" sub="这条资源与其它资源之间的边" tight>
        {related === null ? (
          <Skeleton lines={2} />
        ) : related.length === 0 ? (
          <Empty title="暂无关系" text="还没有其它资源指向它，或它指向别人。" />
        ) : (
          <ul className="related-list">
            {related.map((edge) => (
              <li key={`${edge.src}-${edge.rel}-${edge.dst}`} className="related-row">
                <span className="mono related-side">{edge.src}</span>
                <span className="related-rel">{edge.rel}</span>
                <span className="mono related-side">{edge.dst}</span>
              </li>
            ))}
          </ul>
        )}
      </Card>
    </div>
  );
}

/** 保留与清理：先看各表现状，再试跑一次确认要删多少，确认后才真正执行。 */
function RetentionCard() {
  const { notify, refresh } = useConsole();
  const [status, setStatus] = useState<CommitterStatus | null>(null);
  const [preview, setPreview] = useState<SweepReport | null>(null);
  const [last, setLast] = useState<SweepReport | null>(null);
  const [lastAt, setLastAt] = useState<string | null>(null);
  const [busy, setBusy] = useState<"preview" | "run" | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      setStatus(await committerStatus());
      setError(null);
    } catch (problem) {
      setError(problem instanceof Error ? problem.message : "读取存储现状失败");
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  async function dryRun() {
    setBusy("preview");
    try {
      setPreview(await sweep(true));
      setError(null);
    } catch (problem) {
      notify("bad", "试跑失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(null);
    }
  }

  async function execute() {
    let report = preview;
    if (!report) {
      setBusy("preview");
      try {
        report = await sweep(true);
        setPreview(report);
      } catch (problem) {
        notify("bad", "试跑失败", problem instanceof Error ? problem.message : undefined);
        setBusy(null);
        return;
      }
      setBusy(null);
    }
    const stale = report.resources + report.resourceMarks + report.facets + report.relations + report.state + report.blobs + report.vectors + report.text;
    if (!window.confirm(`将删除 ${report.tasks} 条终态任务、${report.snapshots} 个快照与 ${stale} 条过期存储行，且不可撤销。继续？`)) {
      return;
    }
    setBusy("run");
    try {
      const done = await sweep(false);
      setLast(done);
      setLastAt(new Date().toISOString());
      setPreview(null);
      notify("ok", `清理完成：${done.tasks} 条任务 / ${done.snapshots} 个快照`, "行数已刷新。");
      await reload();
      refresh();
    } catch (problem) {
      notify("bad", "清理失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(null);
    }
  }

  const counts = status?.counts ?? null;

  return (
    <Card
      title="保留与清理"
      sub="按 policy.retention / versions / raw 的保留期清理；未配置的项不动"
      action={
        <div className="row" style={{ gap: 8 }}>
          <button type="button" className="btn btn-sm" onClick={() => void dryRun()} disabled={busy !== null}>
            {busy === "preview" ? "试跑中…" : "试跑一次"}
          </button>
          <button type="button" className="btn btn-sm btn-danger" onClick={() => void execute()} disabled={busy !== null}>
            <IconTrash size={12} /> {busy === "run" ? "清理中…" : "执行清理"}
          </button>
        </div>
      }
    >
      {error ? (
        <div className="note is-warn">
          <span>{error}</span>
        </div>
      ) : null}

      {counts === null ? (
        <Skeleton lines={3} />
      ) : (
        <div className="table-scroll">
          <table className="table">
            <thead>
              <tr>
                <th>表</th>
                <th className="align-right">行数</th>
                <th>清理规则</th>
              </tr>
            </thead>
            <tbody>
              {[
                ["resources", counts.resources, "低于剩余快照的最小版本"],
                ["resource marks", counts.marks, "同上（墓碑标记随资源清理）"],
                ["facets", counts.facets, "低于版本下限；每个变体另留 facet_keep 版"],
                ["blobs", counts.blobs, "不再被任何 facet 引用的字节"],
                ["relations", counts.relations, "低于版本下限"],
                ["state", counts.state, "低于版本下限"],
                ["snapshots", counts.snapshots, "早于 snapshot_retain；至少保留最新一条"],
                ["tasks", counts.tasks, "早于 retention.tasks 的终态任务"],
              ].map(([name, value, rule]) => (
                <tr key={String(name)}>
                  <td className="mono">{name}</td>
                  <td className="num align-right">{count(Number(value))}</td>
                  <td className="muted" style={{ fontSize: "var(--fs-xs)" }}>
                    {rule}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {preview ? (
        <div style={{ marginTop: 12 }}>
          <p className="faint" style={{ fontSize: "var(--fs-xs)" }}>
            试跑（未写入）：快照 {preview.snapshots} · 资源 {preview.resources} · 标记 {preview.resourceMarks} · facet{" "}
            {preview.facets} · 关系 {preview.relations} · 状态 {preview.state} · 字节 {preview.blobs} · 向量{" "}
            {preview.vectors} · 全文 {preview.text} · 终态任务 {preview.tasks}
          </p>
        </div>
      ) : null}

      {last ? (
        <p className="faint" style={{ fontSize: "var(--fs-xs)", marginTop: 8 }}>
          上次清理：快照 {last.snapshots} · 资源 {last.resources} · facet {last.facets} · 终态任务 {last.tasks} ·{" "}
          {relative(lastAt)}
        </p>
      ) : null}

      <p className="faint" style={{ fontSize: "var(--fs-2xs)", lineHeight: 1.7, marginTop: 8 }}>
        清理不会删掉墓碑资源：墓碑是删除语义的一部分，能恢复；这里只回收快照、旧版本行与不再被引用的字节。
      </p>
    </Card>
  );
}
