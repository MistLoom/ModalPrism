import { useCallback, useEffect, useMemo, useState } from "react";

import { ArticleBody } from "../components/ArticleBody";
import { FacetExplorer } from "../components/FacetExplorer";
import { IconRefresh, IconTrash } from "../components/icons";
import { ManualDeliveryModal } from "../components/ManualDelivery";
import { Pager } from "../components/Pager";
import { Card, Drawer, Empty, Field, Loading, Modal, Problem, StatusBadge, StructuredValue, Tabs, useUi } from "../components/ui";
import {
  batchContent,
  contentLineage,
  contentPurge,
  contentRelations,
  deleteContent,
  getContent,
  listContent,
  listContentVersions,
  listSnapshots,
  listSources,
  listSourceTypes,
  patchContent,
  replaySnapshot,
} from "../lib/api";
import { isHtmlBody } from "../lib/article";
import { facetKindLabel } from "../lib/facets";
import { contentKindLabel, relative, stamp } from "../lib/format";
import { ProjectScopeBadge } from "../lib/project";
import { navigate } from "../lib/router";
import type { ContentDetail, ContentItem, Snapshot, SourceTypeRecord } from "../lib/types";

// 类型→来源两级筛选：按 sourceId 派生信源类型（不落库）。
function sourceKey(id: string): string {
  return id.replace(/^source\//, "");
}

export function ContentPage() {
  const { notify, run } = useUi();
  const [tab, setTab] = useState<"items" | "snapshots">("items");
  const [items, setItems] = useState<ContentItem[] | null>(null);
  const [snapshots, setSnapshots] = useState<Snapshot[] | null>(null);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(50);
  const [total, setTotal] = useState(0);
  const [search, setSearch] = useState("");
  const [source, setSource] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);
  const [selection, setSelection] = useState<string[]>([]);
  const [detail, setDetail] = useState<string | null>(null);
  const [replay, setReplay] = useState(false);
  const [purgeOpen, setPurgeOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [typeFilter, setTypeFilter] = useState("all");
  const [typeRecords, setTypeRecords] = useState<SourceTypeRecord[]>([]);
  const [deliver, setDeliver] = useState<ContentItem[] | null>(null);
  const [sourceTypes, setSourceTypes] = useState<Map<string, string>>(new Map());
  const [sourceNames, setSourceNames] = useState<Map<string, string>>(new Map());

  const load = useCallback(
    async (target = page) => {
      setLoading(true);
      const offset = (Math.max(1, target) - 1) * pageSize;
      try {
        if (tab === "snapshots") {
          const result = await listSnapshots({ limit: pageSize, offset });
          setSnapshots(result.items);
          setTotal(result.total ?? result.items.length + offset);
        } else {
          const result = await listContent({
            limit: pageSize,
            offset,
            query: search.trim() || undefined,
            sourceId: source || undefined,
            sourceType: typeFilter === "all" ? undefined : typeFilter,
            status: "active",
          });
          setItems(result.items);
          setTotal(result.total ?? result.items.length + offset);
        }
        setError(null);
      } catch (problem) {
        setError(problem);
      } finally {
        setLoading(false);
      }
    },
    [tab, search, source, typeFilter, page, pageSize],
  );

  useEffect(() => {
    setSelection([]);
    void load();
  }, [load]);

  // 深链：从采集记录或工作台速览跳过来时，直接展开这条内容的详情。
  useEffect(() => {
    const params = new URLSearchParams(window.location.hash.split("?")[1] ?? "");
    const id = params.get("id");
    if (id) setDetail(id);
  }, []);

  // 类型→来源两级筛选的元数据：一次拉取信源与类型注册表，按 sourceId 派生类型。
  useEffect(() => {
    void (async () => {
      try {
        const [sourcePage, typePage] = await Promise.all([listSources({ limit: 200 }), listSourceTypes()]);
        setSourceTypes(new Map(sourcePage.items.map((item) => [item.id, item.sourceType])));
        setSourceNames(new Map(sourcePage.items.map((item) => [item.id, item.name])));
        setTypeRecords(typePage.items);
      } catch {
        // 类型筛选是增强能力：拉取失败时退化为仅按来源筛选，不阻断内容主流程。
      }
    })();
  }, []);

  const typeOfSource = useCallback((id: string) => sourceTypes.get(sourceKey(id)) ?? "", [sourceTypes]);
  const typeLabelOf = useCallback(
    (typeId: string) => typeRecords.find((item) => item.id === typeId)?.label ?? typeId,
    [typeRecords],
  );

  const sourceOptions = useMemo(() => {
    const ids = new Set((items ?? []).map((item) => item.sourceId));
    return [...ids].sort();
  }, [items]);

  // 来源下拉按所选类型收窄；类型为「全部」时展示全部来源。
  const filteredSources = useMemo(
    () => sourceOptions.filter((id) => typeFilter === "all" || typeOfSource(id) === typeFilter),
    [sourceOptions, typeFilter, typeOfSource],
  );

  async function act(label: string, ids: string[], action: string, tags?: string[]) {
    if (!ids.length) return;
    setBusy(true);
    let started = false;
    try {
      const operation = await batchContent({ ids, action, tags });
      started = true;
      await run(operation, label);
      setSelection([]);
      await load();
    } catch (problem) {
      if (!started) {
        notify("bad", `${label}失败`, problem instanceof Error ? problem.message : undefined);
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <div className="kicker">content · 内容</div>
          <h1 className="page-title">内容库</h1>
          <p className="page-lede">
            删除会移除派生数据与关联对象，不可恢复。版本与关联信息见详情；按快照重建只追加新版本，不修改历史。
          </p>
        </div>
        <div className="page-actions">
          <ProjectScopeBadge />
          <button type="button" className="btn" onClick={() => void load()} disabled={loading}>
            <IconRefresh size={14} /> 刷新
          </button>
          <button type="button" className="btn btn-danger" onClick={() => setPurgeOpen(true)}>
            <IconTrash size={14} /> 清空内容
          </button>
          <button type="button" className="btn" onClick={() => setReplay(true)}>按快照重建</button>
        </div>
      </header>

      {error ? <Problem error={error} onRetry={() => void load()} /> : null}

      <Tabs
        value={tab}
        onChange={(value) => {
          setTab(value as "items" | "snapshots");
          setPage(1);
        }}
        items={[
          { id: "items", label: "内容库", count: total },
          { id: "snapshots", label: "快照" },
        ]}
      />

      {tab !== "snapshots" ? (
        <div className="toolbar-band">
          <label className="search">
            <span className="muted" aria-hidden="true">⌕</span>
            <input
              value={search}
              onChange={(event) => {
                setSearch(event.target.value);
                setPage(1);
              }}
              placeholder="按标题或正文检索"
              aria-label="检索内容"
            />
          </label>
          <select
            className="select"
            value={typeFilter}
            onChange={(event) => {
              const next = event.target.value;
              setTypeFilter(next);
              setPage(1);
              if (source && next !== "all" && typeOfSource(source) !== next) setSource("");
            }}
            aria-label="信源类型筛选"
          >
            <option value="all">全部信源类型</option>
            {typeRecords.map((item) => (
              <option key={item.id} value={item.id}>{item.label}</option>
            ))}
          </select>
          <select
            className="select"
            value={source}
            onChange={(event) => {
              setSource(event.target.value);
              setPage(1);
            }}
            aria-label="来源筛选"
          >
            <option value="">全部来源</option>
            {filteredSources.map((id) => (
              <option key={id} value={id}>
                {typeOfSource(id) ? `[${typeLabelOf(typeOfSource(id))}] ` : ""}{sourceNames.get(sourceKey(id)) ?? id}
              </option>
            ))}
          </select>
          <span className="fill" />
          <span className="small muted">{total} 条</span>
        </div>
      ) : (
        <div className="alert info">快照为库级版本（跨项目共享的只读记录）；重建为后台任务，仅重放当前项目自有信源的内容，不修改历史快照。</div>
      )}

      {selection.length > 0 && tab !== "snapshots" ? (
        <div className="batchbar">
          <span>已选 {selection.length} 条</span>
          <button type="button" className="btn btn-sm" disabled={busy} onClick={() => setDeliver((items ?? []).filter((item) => selection.includes(item.id)))}>
            投递
          </button>
          <button type="button" className="btn btn-sm" disabled={busy} onClick={() => void act("批量打标签", selection, "addTags", ["待复核"])}>
            标记待复核
          </button>
          <button type="button" className="btn btn-sm" disabled={busy} onClick={() => void act("批量移除标签", selection, "removeTags", ["待复核"])}>
            移除标记
          </button>
          <button type="button" className="btn btn-sm btn-danger" disabled={busy} onClick={() => void act("删除选中内容", selection, "delete")}>
            <IconTrash size={12} /> 删除
          </button>
          <button type="button" className="btn btn-sm" onClick={() => setSelection([])}>清除选择</button>
        </div>
      ) : null}

      {tab === "snapshots" ? (
        <Card title="已发布快照" sub="按快照号倒序" flush>
          {loading && !snapshots ? (
            <div className="card-body"><Loading lines={5} /></div>
          ) : (snapshots ?? []).length === 0 ? (
            <Empty title="暂无快照" text="首次提交后出现。" />
          ) : (
            <div className="list">
              {(snapshots ?? []).map((snapshot) => (
                <div className="list-row" key={snapshot.id}>
                  <span className="list-main">
                    <span className="list-title">快照 #{snapshot.snapshot}</span>
                    <span className="list-meta">
                      <span>{stamp(snapshot.publishedAt)}</span>
                      <span>{snapshot.commitCount} 个提交</span>
                    </span>
                  </span>
                  <button type="button" className="btn btn-sm" onClick={() => setReplay(true)}>重建</button>
                </div>
              ))}
            </div>
          )}
          <Pager
            page={page}
            pageSize={pageSize}
            total={total}
            onPage={setPage}
            onPageSize={(size) => {
              setPageSize(size);
              setPage(1);
            }}
          />
        </Card>
      ) : (
        <Card title="内容库" sub="版本只追加；删除会物理移除" flush>
          {loading && !items ? (
            <div className="card-body"><Loading lines={6} /></div>
          ) : (items ?? []).length === 0 ? (
            <Empty
              title="无匹配内容"
              text="调整检索词或来源筛选；采集完成后内容自动入库。"
            />
          ) : (
            <div className="content-list">
              {(items ?? []).map((item) => (
                <div key={item.id} className={`content-row${selection.includes(item.id) ? " is-selected" : ""}`}>
                  <input
                    type="checkbox"
                    aria-label={`选择 ${item.title}`}
                    checked={selection.includes(item.id)}
                    onChange={(event) =>
                      setSelection((current) => (event.target.checked ? [...current, item.id] : current.filter((id) => id !== item.id)))
                    }
                  />
                  <button
                    type="button"
                    className="content-main"
                    style={{ border: 0, background: "transparent", textAlign: "left", cursor: "pointer" }}
                    onClick={() => setDetail(item.id)}
                    aria-label={`打开 ${item.title}`}
                  >
                    <span className="content-title">{item.title}</span>
                    <span className="content-meta">
                      <span className="tag">{contentKindLabel(item.kind)}</span>
                      {item.facetCount && item.facetCount > 1 ? (
                        <span
                          className="tag"
                          title={Array.from(new Set((item.facetKinds ?? []).map((facet) => facetKindLabel(facet.split(":")[0])))).join("、")}
                        >
                          包含 {item.facetKinds?.length ?? item.facetCount} 类数据
                        </span>
                      ) : null}
                      <span>{item.sourceName}</span>
                      <span title={stamp(item.publishedAt ?? "")}>{item.publishedAt ? `发布 ${relative(item.publishedAt)}` : "发布 未披露"}</span>
                      <span>收录 {relative(item.createdAt)}</span>
                      {item.tags.map((tag) => <span className="tag" key={tag}>{tag}</span>)}
                    </span>
                  </button>
                  <div className="content-side row">
                    <button type="button" className="btn btn-sm" onClick={() => setDeliver([item])}>
                      投递
                    </button>
                    <button
                      type="button"
                      className="btn btn-sm btn-danger"
                      onClick={() =>
                        void (async () => {
                          if (!window.confirm(`永久删除「${item.title}」？关联的派生数据与无引用对象将一并移除。`)) return;
                          let started = false;
                          try {
                            const meta = await getContent(item.id);
                            const operation = await deleteContent(item.id, meta.etag ?? item.etag);
                            started = true;
                            await run(operation, "删除内容");
                            await load();
                          } catch (problem) {
                            if (!started) {
                              notify("bad", "删除失败", problem instanceof Error ? problem.message : undefined);
                            }
                          }
                        })()
                      }
                    >
                      删除
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}
          <Pager
            page={page}
            pageSize={pageSize}
            total={total}
            onPage={setPage}
            onPageSize={(size) => {
              setPageSize(size);
              setPage(1);
            }}
          />
        </Card>
      )}

      {detail ? <ContentDrawer id={detail} onClose={() => setDetail(null)} onChanged={() => void load()} /> : null}
      {deliver ? (
        <ManualDeliveryModal
          preset={{ items: deliver }}
          onClose={() => setDeliver(null)}
          onDone={() => {
            setSelection([]);
            void load();
          }}
        />
      ) : null}
      {replay ? <ReplayModal snapshots={snapshots ?? []} selected={selection} onClose={() => setReplay(false)} /> : null}
      {purgeOpen ? (
        <PurgeModal
          source={source}
          onClose={() => setPurgeOpen(false)}
          onDone={() => {
            setPurgeOpen(false);
            void load();
          }}
        />
      ) : null}
    </div>
  );
}

function ContentDrawer({ id, onClose, onChanged }: { id: string; onClose: () => void; onChanged: () => void }) {
  const { notify } = useUi();
  const [detail, setDetail] = useState<ContentDetail | null>(null);
  const [etag, setEtag] = useState("");
  const [versions, setVersions] = useState<Record<string, unknown>[]>([]);
  const [lineage, setLineage] = useState<{ nodes: Record<string, unknown>[]; edges: Record<string, unknown>[] } | null>(null);
  const [relations, setRelations] = useState<Record<string, unknown>[]>([]);
  const [error, setError] = useState<unknown>(null);
  const [tab, setTab] = useState<"body" | "versions" | "lineage" | "relations">("body");
  const [tags, setTags] = useState("");
  const [title, setTitle] = useState("");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let alive = true;
    void (async () => {
      try {
        const meta = await getContent(id);
        if (!alive) return;
        setDetail(meta.data);
        setEtag(meta.etag ?? meta.data.etag);
        setTags(meta.data.tags.join(", "));
        setTitle(meta.data.title);
        const [versionPage, graph, relationPage] = await Promise.allSettled([
          listContentVersions(id, { limit: 100 }),
          contentLineage(id),
          contentRelations(id),
        ]);
        if (!alive) return;
        if (versionPage.status === "fulfilled") setVersions(versionPage.value.items);
        if (graph.status === "fulfilled") setLineage(graph.value);
        if (relationPage.status === "fulfilled") setRelations(relationPage.value.items);
      } catch (problem) {
        if (alive) setError(problem);
      }
    })();
    return () => {
      alive = false;
    };
  }, [id]);

  async function save() {
    if (!detail) return;
    setSaving(true);
    try {
      await patchContent(id, { title, tags: tags.split(",").map((tag) => tag.trim()).filter(Boolean) }, etag);
      notify("ok", "内容元数据已更新");
      onChanged();
      const next = await getContent(id);
      setDetail(next.data);
      setEtag(next.etag ?? next.data.etag);
    } catch (problem) {
      notify("bad", "更新失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setSaving(false);
    }
  }

  return (
    <Drawer
      title={detail?.title ?? id}
      sub={detail ? `${detail.sourceName} · ${contentKindLabel(detail.kind)}` : undefined}
      onClose={onClose}
      wide
      footer={
        <>
          <button type="button" className="btn" onClick={() => navigate(`logs?correlationId=${encodeURIComponent(id)}`)}>查看关联日志</button>
          <button type="button" className="btn" onClick={onClose}>关闭</button>
          <button type="button" className="btn btn-primary" onClick={() => void save()} disabled={saving || !detail}>{saving ? "保存中…" : "保存元数据"}</button>
        </>
      }
    >
      {error ? <Problem error={error} /> : null}
      {!detail ? <Loading lines={5} /> : (
        <div className="stack" style={{ gap: 14 }}>
          <div className="detail-hero">
            <span className="detail-title">{detail.title}</span>
            <div className="detail-meta">
              <StatusBadge status={detail.status} />
              <span className="mono">{detail.id}</span>
              <span>{detail.sourceName}</span>
              <span>{detail.publishedAt ? `发布 ${stamp(detail.publishedAt)}` : "原文未提供发布时间"}</span>
              <span>收录 {stamp(detail.createdAt)}</span>
            </div>
          </div>
          <div className="grid cols-2">
            <Field label="标题">
              <input className="input" value={title} onChange={(event) => setTitle(event.target.value)} />
            </Field>
            <Field label="标签" hint="逗号分隔；写入用户分类，不改变原始内容">
              <input className="input" value={tags} onChange={(event) => setTags(event.target.value)} />
            </Field>
          </div>
          <Tabs
            value={tab}
            onChange={setTab}
            items={[
              { id: "body", label: "正文" },
              { id: "versions", label: "版本", count: versions.length },
              { id: "lineage", label: "关联" },
              { id: "relations", label: "关系", count: relations.length },
            ]}
          />
          {tab === "body" ? (
            detail.facets?.length ? (
              // 数据面浏览器：同一条内容的多形态数据（正文/结构化/附件/区域索引）都能单独查看。
              <FacetExplorer
                contentId={detail.id}
                facets={detail.facets}
                base={contentUrl(detail)}
                preferred={{ kind: detail.body?.kind, variant: detail.body?.variant }}
              />
            ) : detail.body?.text ? (
              isHtmlBody(detail.body.kind, detail.body.mediaType, detail.body.text) ? (
                // 采集正文多为原始网页 HTML：按白名单重建后渲染，避免把标签源码当文本展示。
                <ArticleBody
                  html={detail.body.text}
                  base={contentUrl(detail)}
                  bodyKind={detail.body.kind}
                  mediaType={detail.body.mediaType}
                />
              ) : (
                <div className="body-text">
                  {detail.body.text.split(/\n{2,}/).map((paragraph, index) => <p key={index}>{paragraph}</p>)}
                </div>
              )
            ) : detail.body?.value ? (
              <StructuredValue value={detail.body.value} />
            ) : detail.struct ? (
              <StructuredValue value={detail.struct} />
            ) : (
              <Empty title="没有可读正文" text="该内容可能仅含结构化字段或附件。" />
            )
          ) : null}
          {tab === "versions" ? (
            <div className="list">
              {versions.length === 0 ? <Empty title="没有版本记录" /> : versions.map((version) => (
                <div className="list-row" key={String(version.id ?? version.contentHash)}>
                  <span className="list-main">
                    <span className="list-title">{String(version.kind)} {version.variant ? `· ${String(version.variant)}` : ""} · v{String(version.version)}</span>
                    <span className="list-meta">
                      <span className="mono">{String(version.contentHash ?? "").slice(0, 12)}</span>
                      <span>{stamp(String(version.createdAt ?? ""))}</span>
                    </span>
                  </span>
                </div>
              ))}
            </div>
          ) : null}
          {tab === "lineage" ? (
            <div className="stack" style={{ gap: 12 }}>
              <div className="small muted">节点 {lineage?.nodes.length ?? 0} · 边 {lineage?.edges.length ?? 0}</div>
              {lineage && lineage.nodes.length ? (
                <div className="stack" style={{ gap: 6 }}>
                  <span className="small muted">节点</span>
                  <div className="value-list">
                    {lineage.nodes.map((node, index) => (
                      <div className="value-item" key={`node-${index}`}>
                        <StructuredValue value={node} />
                      </div>
                    ))}
                  </div>
                </div>
              ) : null}
              {lineage && lineage.edges.length ? (
                <div className="stack" style={{ gap: 6 }}>
                  <span className="small muted">边</span>
                  <div className="value-list">
                    {lineage.edges.map((edge, index) => (
                      <div className="value-item" key={`edge-${index}`}>
                        <StructuredValue value={edge} />
                      </div>
                    ))}
                  </div>
                </div>
              ) : null}
              {lineage && !lineage.nodes.length && !lineage.edges.length ? <Empty title="没有关联记录" /> : null}
            </div>
          ) : null}
          {tab === "relations" ? (
            <div className="list">
              {relations.length === 0 ? <Empty title="没有关系" /> : relations.map((relation) => (
                <div className="list-row" key={String(relation.id)}>
                  <span className="list-main">
                    <span className="list-title">{String(relation.relation)}</span>
                    <span className="list-meta">
                      <span>{String(relation.direction)}</span>
                      <span className="mono">{String(relation.sourceId)} → {String(relation.targetId)}</span>
                    </span>
                  </span>
                </div>
              ))}
            </div>
          ) : null}
        </div>
      )}
    </Drawer>
  );
}

/** 原文地址：优先取结构面的 locator.url，其次顶层 url；用于相对链接补齐与原文入口。 */
function contentUrl(detail: ContentDetail): string {
  const struct = (detail.struct ?? {}) as Record<string, unknown>;
  const locator = struct.locator;
  const located = locator && typeof locator === "object" ? (locator as Record<string, unknown>).url : undefined;
  const value = struct.url ?? located;
  return typeof value === "string" ? value : "";
}

function ReplayModal({ snapshots, selected, onClose }: { snapshots: Snapshot[]; selected: string[]; onClose: () => void }) {
  const { notify, run } = useUi();
  const [snapshot, setSnapshot] = useState(snapshots[0]?.id ?? "");
  const [scope, setScope] = useState(selected.length ? "selected" : "all");
  const [busy, setBusy] = useState(false);

  async function start() {
    setBusy(true);
    try {
      const operation = await replaySnapshot({ snapshotId: snapshot, scope, contentIds: scope === "selected" ? selected : undefined });
      await run(operation, "按快照重建");
      onClose();
    } catch (problem) {
      notify("bad", "按快照重建失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title="按快照重建"
      sub="以选定快照的结构化字段生成当前版本；历史快照保持只读"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>取消</button>
          <button type="button" className="btn btn-primary" onClick={() => void start()} disabled={busy || !snapshot || (scope === "selected" && selected.length === 0)}>
            {busy ? "重建中…" : "开始重建"}
          </button>
        </>
      }
    >
      <div className="stack" style={{ gap: 12 }}>
        <Field label="快照">
          <select className="select" value={snapshot} onChange={(event) => setSnapshot(event.target.value)}>
            {snapshots.length === 0 ? <option value="">没有可用快照</option> : null}
            {snapshots.map((item) => (
              <option key={item.id} value={item.id}>#{item.snapshot} · {stamp(item.publishedAt)}</option>
            ))}
          </select>
        </Field>
        <Field label="范围">
          <select className="select" value={scope} onChange={(event) => setScope(event.target.value)}>
            <option value="all">全部命中内容</option>
            <option value="selected">仅已选内容（{selected.length}）</option>
          </select>
        </Field>
        <div className="alert warn">重建为后台任务；失败条目保持原状，成功条目追加新版本并记录关联。</div>
      </div>
    </Modal>
  );
}

function PurgeModal({ source, onClose, onDone }: { source: string; onClose: () => void; onDone: () => void }) {
  const { notify } = useUi();
  const [scope, setScope] = useState<"source" | "all">(source ? "source" : "all");
  const [preview, setPreview] = useState<number | null>(null);
  const [deleted, setDeleted] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);

  const filter = () => ({ source: scope === "source" ? source : undefined });

  useEffect(() => {
    let alive = true;
    setPreview(null);
    void (async () => {
      setBusy(true);
      try {
        const report = await contentPurge({ ...filter(), dryRun: true });
        if (alive) setPreview(report.matched);
      } catch (problem) {
        if (alive) notify("bad", "预演失败", problem instanceof Error ? problem.message : undefined);
      } finally {
        if (alive) setBusy(false);
      }
    })();
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scope, source]);

  async function purge() {
    if (!window.confirm(`永久删除${scope === "source" ? "当前来源" : "全部"}的 ${preview ?? 0} 条内容？该操作不可恢复。`)) return;
    setBusy(true);
    let total = 0;
    try {
      for (let round = 0; round < 500; round += 1) {
        const report = await contentPurge({ ...filter(), dryRun: false });
        total += report.deleted;
        setDeleted(total);
        if (!report.truncated) break;
      }
      notify("ok", "内容已清空", `共永久删除 ${total} 条`);
      onDone();
    } catch (problem) {
      notify("bad", "清空失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title="清空内容"
      sub="按当前来源或全部内容永久删除；派生数据与无引用对象一并移除"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>取消</button>
          <button type="button" className="btn btn-danger" onClick={() => void purge()} disabled={busy || !preview}>
            {busy ? "处理中…" : `永久删除 ${preview ?? 0} 条`}
          </button>
        </>
      }
    >
      <div className="stack" style={{ gap: 12 }}>
        <Field label="范围">
          <select className="select" value={scope} onChange={(event) => setScope(event.target.value as "source" | "all")}>
            <option value="source" disabled={!source}>当前来源（{source || "未筛选"}）</option>
            <option value="all">全部内容（含已删除记录）</option>
          </select>
        </Field>
        <div className="alert warn">
          预演命中 <strong className="mono">{preview ?? "…"}</strong> 条。删除会移除资源行、全部版本的派生 facet、关系、身份映射与无引用对象；不可恢复。
        </div>
        {deleted !== null ? <div className="small muted">本轮已删除 {deleted} 条，继续清理中…</div> : null}
      </div>
    </Modal>
  );
}
