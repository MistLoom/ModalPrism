import { useEffect, useMemo, useState } from "react";

import { getContentFacet } from "../lib/api";
import { isHtmlBody } from "../lib/article";
import { bytes as formatBytes } from "../lib/format";
import { defaultFacet, facetLabel, facetVariantLabel, groupFacets } from "../lib/facets";
import { renderMarkdown } from "../lib/markdown";
import type { ContentFacetBody, ContentFacetSummary } from "../lib/types";
import { ArticleBody } from "./ArticleBody";
import { MediaView } from "./MediaView";
import { Empty, Loading, StructuredValue } from "./ui";

type FacetRefKey = { kind: string; variant?: string | null };

function sameFacet(left: FacetRefKey, right: FacetRefKey): boolean {
  return left.kind === right.kind && (left.variant ?? null) === (right.variant ?? null);
}

/** 区域索引里的一行：RawPrism 扫描出的结构区域，指向可单独查看的数据面。 */
interface RegionRow {
  typeId?: string;
  start?: number;
  end?: number;
  confidence?: number;
  kind?: string;
  variant?: string;
  label?: string;
}

function regionsOf(value: unknown): RegionRow[] {
  if (!value || typeof value !== "object") return [];
  const raw = (value as { regions?: unknown }).regions;
  if (!Array.isArray(raw)) return [];
  return raw.filter((item): item is RegionRow => Boolean(item) && typeof item === "object");
}

/** 链接清单：兼容 `{links:[{url,text}]}`、`{items:[...]}` 与裸数组。 */
function linksOf(value: unknown): { url: string; text?: string }[] {
  const list = Array.isArray(value)
    ? value
    : value && typeof value === "object"
      ? ((value as { links?: unknown; items?: unknown }).links ?? (value as { items?: unknown }).items)
      : null;
  if (!Array.isArray(list)) return [];
  return list
    .map((item) => {
      if (typeof item === "string") return { url: item };
      if (item && typeof item === "object") {
        const record = item as Record<string, unknown>;
        const url = record.url ?? record.href ?? record.locator;
        const text = record.text ?? record.title ?? record.label;
        if (typeof url === "string") return { url, text: typeof text === "string" ? text : undefined };
      }
      return null;
    })
    .filter((item): item is { url: string; text?: string } => item !== null);
}

/** CSV 文本拆成行列：只处理逗号分隔与双引号包裹，表格化展示而不是抛原始文本。 */
function csvRows(text: string): string[][] {
  return text
    .split(/\r?\n/)
    .filter((line) => line.trim())
    .slice(0, 200)
    .map((line) => line.split(",").map((cell) => cell.replace(/^"|"$/g, "").trim()));
}

function CsvTable({ rows }: { rows: string[][] }) {
  const [head, ...body] = rows;
  if (!head) return <Empty title="没有表格数据" />;
  return (
    <div className="table-wrap" style={{ overflowX: "auto" }}>
      <table className="table">
        <thead>
          <tr>{head.map((cell, index) => <th key={index}>{cell}</th>)}</tr>
        </thead>
        <tbody>
          {body.map((row, rowIndex) => (
            <tr key={rowIndex}>
              {row.map((cell, index) => <td key={index}>{cell}</td>)}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/**
 * 内容的数据面浏览器：左侧列出这条内容包含的全部数据面（按类型分组、标出多份），
 * 右侧按类型分派渲染器。结构化值一律键值/表格化，不向用户抛原始 JSON。
 */
export function FacetExplorer({ contentId, facets, base, preferred }: {
  contentId: string;
  facets: ContentFacetSummary[];
  /** 原文地址：用于相对链接补齐与「打开原文」。 */
  base?: string;
  /** 默认选中的数据面（通常来自详情的默认正文）。 */
  preferred?: { kind?: string | null; variant?: string | null } | null;
}) {
  const groups = useMemo(() => groupFacets(facets ?? []), [facets]);
  const [selected, setSelected] = useState<FacetRefKey | null>(() => {
    const hit = defaultFacet(facets ?? [], preferred ?? null);
    return hit ? { kind: hit.kind, variant: hit.variant ?? null } : null;
  });
  const [body, setBody] = useState<ContentFacetBody | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(false);

  // 数据面变化（内容切换）时重新落位默认面。
  useEffect(() => {
    const hit = defaultFacet(facets ?? [], preferred ?? null);
    setSelected(hit ? { kind: hit.kind, variant: hit.variant ?? null } : null);
    // preferred 只作为初次落位依据，翻面时不重新落位。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [contentId, facets]);

  useEffect(() => {
    if (!selected) {
      setBody(null);
      return;
    }
    let alive = true;
    setLoading(true);
    setError(null);
    void (async () => {
      try {
        const next = await getContentFacet(contentId, selected.kind, selected.variant);
        if (alive) setBody(next);
      } catch (problem) {
        if (alive) {
          setBody(null);
          setError(problem);
        }
      } finally {
        if (alive) setLoading(false);
      }
    })();
    return () => {
      alive = false;
    };
  }, [contentId, selected]);

  if (!groups.length) {
    return <Empty title="没有可查看的数据" text="该内容仅含内部加工数据，无正文或附件。" />;
  }

  const select = (kind: string, variant?: string | null) => {
    if (selected && sameFacet(selected, { kind, variant })) return;
    setSelected({ kind, variant: variant ?? null });
  };

  const text = body?.text ?? "";
  const structured = body?.value;
  const isStructuredOnly = !text.trim() && structured !== undefined && structured !== null;

  return (
    <div className="facet-explorer">
      <nav className="facet-nav" aria-label="数据面清单">
        {groups.map((group) => (
          <div className="facet-group" key={group.kind}>
            <div className="facet-group-head">
              <span>{group.label}</span>
              {group.items.length > 1 ? <span className="tag">{group.items.length} 份</span> : null}
            </div>
            {group.items.map((facet) => {
              const active = selected ? sameFacet(selected, { kind: facet.kind, variant: facet.variant ?? null }) : false;
              return (
                <button
                  type="button"
                  key={`${facet.kind}:${facet.variant ?? ""}`}
                  className={`facet-item${active ? " is-active" : ""}`}
                  onClick={() => select(facet.kind, facet.variant)}
                  aria-current={active ? "true" : undefined}
                >
                  <span className="facet-item-label">
                    {facet.variant ? facetVariantLabel(facet.variant) || facet.variant : group.label}
                  </span>
                  <span className="facet-item-meta">
                    {facet.size ? formatBytes(facet.size) : facet.mediaType ?? ""}
                    {facet.versions > 1 ? ` · v${facet.version}` : ""}
                  </span>
                </button>
              );
            })}
          </div>
        ))}
      </nav>
      <section className="facet-body" aria-live="polite">
        {error ? (
          <Empty title="这一面读取失败" text={error instanceof Error ? error.message : "请稍后重试。"} />
        ) : loading ? (
          <Loading lines={6} />
        ) : !selected ? (
          <Empty title="选择一个数据面" />
        ) : (
          <FacetRenderer
            contentId={contentId}
            selected={selected}
            body={body}
            text={text}
            structured={structured}
            isStructuredOnly={isStructuredOnly}
            base={base}
            onOpenFacet={select}
          />
        )}
      </section>
    </div>
  );
}

function FacetRenderer({ contentId, selected, body, text, structured, isStructuredOnly, base, onOpenFacet }: {
  contentId: string;
  selected: FacetRefKey;
  body: ContentFacetBody | null;
  text: string;
  structured: unknown;
  isStructuredOnly: boolean;
  base?: string;
  onOpenFacet: (kind: string, variant?: string | null) => void;
}) {
  const { kind, variant } = selected;
  const hasText = Boolean(text.trim());

  if (["image", "audio", "video", "file"].includes(kind)) {
    return (
      <MediaView
        contentId={contentId}
        kind={kind}
        variant={variant}
        mediaType={body?.mediaType}
        size={body?.size}
        href={hrefOf(structured)}
      />
    );
  }

  if (kind === "regions") {
    const regions = regionsOf(structured);
    if (!regions.length) return <Empty title="没有识别到的结构区域" text="未识别到结构区域。" />;
    return (
      <div className="list">
        <div className="small muted" style={{ padding: "6px 2px" }}>
          共 {regions.length} 个结构区域；选择一行查看该区域数据。
        </div>
        {regions.map((region, index) => {
          const openable = Boolean(region.kind);
          return (
            <div className="list-row" key={`${region.typeId ?? "region"}-${index}`}>
              <span className="list-main">
                <span className="list-title">
                  {region.label ?? region.typeId ?? `区域 ${index + 1}`}
                  {region.kind ? <span className="tag" style={{ marginLeft: 8 }}>{facetLabel({ kind: region.kind, variant: region.variant ?? null })}</span> : null}
                </span>
                <span className="list-meta">
                  {typeof region.start === "number" && typeof region.end === "number" ? (
                    <span className="mono">{region.start}–{region.end}</span>
                  ) : null}
                  {typeof region.confidence === "number" ? <span>置信度 {Math.round(region.confidence * 100)}%</span> : null}
                </span>
              </span>
              {openable ? (
                <button type="button" className="btn btn-sm" onClick={() => onOpenFacet(region.kind as string, region.variant ?? null)}>
                  单独查看
                </button>
              ) : null}
            </div>
          );
        })}
      </div>
    );
  }

  if (kind === "links") {
    const links = linksOf(structured);
    if (!links.length && !hasText) return <Empty title="没有链接" />;
    return (
      <div className="list">
        {links.map((link, index) => (
          <div className="list-row" key={`${link.url}-${index}`}>
            <span className="list-main">
              <a className="list-title" href={link.url} target="_blank" rel="noreferrer">{link.text ?? link.url}</a>
              {link.text ? <span className="list-meta"><span className="mono">{link.url}</span></span> : null}
            </span>
          </div>
        ))}
      </div>
    );
  }

  if (isStructuredOnly) {
    return <StructuredValue value={structured} />;
  }

  if (kind === "md") {
    if (!hasText) {
      return structured !== undefined && structured !== null
        ? <StructuredValue value={structured} />
        : <Empty title="这一面没有内容" />;
    }
    return <ArticleBody html={renderMarkdown(text)} base={base} bodyKind="md" mediaType="text/markdown" />;
  }

  if (kind === "html" || isHtmlBody(kind, body?.mediaType, text)) {
    if (!hasText) return <Empty title="这一面没有内容" />;
    return <ArticleBody html={text} base={base} bodyKind={kind} mediaType={body?.mediaType} />;
  }

  if (kind === "csv") {
    const rows = csvRows(text);
    if (rows.length) return <CsvTable rows={rows} />;
  }

  if (kind === "json" || kind === "yaml" || kind === "struct" || kind === "xml") {
    // 结构化面：能解析成对象就键值化；否则按文本段落展示，绝不回吐原始 JSON 源码。
    if (hasText) {
      const parsed = parseMaybe(text);
      if (parsed !== undefined) return <StructuredValue value={parsed} />;
    }
    return hasText ? <TextBody text={text} /> : <Empty title="这一面没有内容" />;
  }

  return hasText ? <TextBody text={text} /> : <Empty title="这一面没有内容" />;
}

function TextBody({ text }: { text: string }) {
  return (
    <div className="body-text">
      {text.split(/\n{2,}/).map((paragraph, index) => <p key={index}>{paragraph}</p>)}
    </div>
  );
}

/** 结构化面的文本体：先尝试按 JSON 解析，失败时不猜测，交给文本渲染。 */
function parseMaybe(text: string): unknown {
  const trimmed = text.trim();
  if (!/^[[{]/.test(trimmed)) return undefined;
  try {
    return JSON.parse(trimmed.slice(0, 400_000)) as unknown;
  } catch {
    return undefined;
  }
}

/** 结构化面里声明的附件地址：取不到字节时作为回退入口。 */
function hrefOf(value: unknown): string | null {
  if (!value || typeof value !== "object") return null;
  const record = value as Record<string, unknown>;
  const candidate = record.url ?? record.src ?? (record.locator as Record<string, unknown> | undefined)?.url;
  return typeof candidate === "string" && /^https?:\/\//i.test(candidate) ? candidate : null;
}
