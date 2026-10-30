// 数据面（facet）的展示语义：分组、排序与类型名称。
// 类型名称是字典 `facet.kind` 域的前端缺省值；字典加载后由 lib/dictionary 覆盖。

import type { ContentFacetSummary } from "./types";

/** 内部面：描述加工过程，不当作「内容」展示，也不进列表徽标。 */
const INTERNAL = new Set(["detect", "embedding"]);

/** 展示顺序：先正文，再结构化数据，再附件与索引。 */
const KIND_ORDER = [
  "md",
  "html",
  "text",
  "struct",
  "json",
  "yaml",
  "csv",
  "xml",
  "image",
  "audio",
  "video",
  "file",
  "links",
  "regions",
  "raw",
];

const KIND_LABELS: Record<string, string> = {
  md: "正文（Markdown）",
  html: "网页正文",
  text: "纯文本",
  struct: "结构化对象",
  json: "JSON 数据",
  yaml: "YAML 数据",
  csv: "表格数据",
  xml: "XML 数据",
  image: "图片",
  audio: "音频",
  video: "视频",
  file: "附件",
  links: "链接清单",
  regions: "结构区域索引",
  raw: "原始内容",
  detect: "识别结果",
  embedding: "向量",
};

/** 具名变体：承载标题与摘要的主结构面，排序时优先。 */
const NAMED_VARIANTS = ["article", "hotlist", "item", "discovery", "page", "jsonld"];

export function facetKindLabel(kind: string): string {
  return KIND_LABELS[kind] ?? kind;
}

const NAMED_LABELS: Record<string, string> = {
  article: "文章",
  hotlist: "热榜",
  item: "条目",
  discovery: "采集条目",
  page: "页面",
  jsonld: "结构化标注",
};

/** `r0`、`r12` 这类扫描序号变体显示为「第 N 段」。 */
export function facetVariantLabel(variant?: string | null): string {
  if (!variant) return "";
  const ranged = /^r(\d+)$/.exec(variant);
  if (ranged) return `第 ${Number(ranged[1]) + 1} 段`;
  if (variant === "prism") return "识别索引";
  if (variant === "readable") return "已净化";
  return NAMED_LABELS[variant] ?? variant;
}

/** 一个数据面的完整称呼，例如「JSON 数据 · 第 2 段」。 */
export function facetLabel(facet: Pick<ContentFacetSummary, "kind" | "variant">): string {
  const variant = facetVariantLabel(facet.variant);
  return variant ? `${facetKindLabel(facet.kind)} · ${variant}` : facetKindLabel(facet.kind);
}

export function isInternalFacet(facet: Pick<ContentFacetSummary, "kind" | "variant">): boolean {
  return INTERNAL.has(facet.kind) || INTERNAL.has(facet.variant ?? "");
}

function kindWeight(kind: string): number {
  const index = KIND_ORDER.indexOf(kind);
  return index < 0 ? KIND_ORDER.length : index;
}

function variantWeight(variant?: string | null): number {
  if (!variant) return -1;
  if (variant === "readable") return -0.5;
  const named = NAMED_VARIANTS.indexOf(variant);
  if (named >= 0) return named;
  if (variant === "prism") return 50;
  const ranged = /^r(\d+)$/.exec(variant);
  if (ranged) return 100 + Number(ranged[1]);
  return 1000;
}

export interface FacetGroup {
  kind: string;
  label: string;
  /** 同一类型的多份数据面（按展示顺序排好）。 */
  items: ContentFacetSummary[];
}

/** 按类型分组；组内按「正文→具名变体→序号变体」排序，组间按内容优先级排序。 */
export function groupFacets(facets: ContentFacetSummary[]): FacetGroup[] {
  const groups = new Map<string, ContentFacetSummary[]>();
  for (const facet of facets) {
    if (isInternalFacet(facet)) continue;
    const bucket = groups.get(facet.kind);
    if (bucket) bucket.push(facet);
    else groups.set(facet.kind, [facet]);
  }
  return [...groups.entries()]
    .sort(([left], [right]) => kindWeight(left) - kindWeight(right))
    .map(([kind, items]) => ({
      kind,
      label: facetKindLabel(kind),
      items: [...items].sort((left, right) => variantWeight(left.variant) - variantWeight(right.variant)),
    }));
}

/** 默认选中的数据面：优先已净化正文，其次网页正文、纯文本，最后取第一个可展示面。 */
export function defaultFacet(
  facets: ContentFacetSummary[],
  preferred?: { kind?: string | null; variant?: string | null } | null,
): ContentFacetSummary | null {
  const usable = facets.filter((facet) => !isInternalFacet(facet));
  if (preferred?.kind) {
    const hit = usable.find(
      (facet) => facet.kind === preferred.kind && (preferred.variant == null || facet.variant === preferred.variant),
    );
    if (hit) return hit;
  }
  for (const kind of ["md", "html", "text", "struct"]) {
    const hit = usable.filter((facet) => facet.kind === kind).sort((left, right) => variantWeight(left.variant) - variantWeight(right.variant))[0];
    if (hit) return hit;
  }
  const groups = groupFacets(usable);
  return groups[0]?.items[0] ?? null;
}
