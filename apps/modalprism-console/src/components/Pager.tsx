import { IconChevronLeft, IconChevronRight } from "./icons";

const PAGE_SIZES = [20, 50, 100, 200];

export interface PagerProps {
  /** 1 起始的当前页。 */
  page: number;
  pageSize: number;
  /** 总条数；服务端扫描封顶时可传 capped=true 显示「N+」。 */
  total: number;
  capped?: boolean;
  onPage: (page: number) => void;
  onPageSize?: (size: number) => void;
}

// 页码窗口：当前页两侧各展开两页，首尾恒显，缺口用省略号占位。
function pageWindow(page: number, pages: number): (number | "gap")[] {
  if (pages <= 7) return Array.from({ length: pages }, (_, i) => i + 1);
  const wanted = new Set([1, pages, page - 2, page - 1, page, page + 1, page + 2]);
  const kept = [...wanted].filter((n) => n >= 1 && n <= pages).sort((a, b) => a - b);
  const out: (number | "gap")[] = [];
  let previous = 0;
  for (const n of kept) {
    if (previous && n - previous > 1) out.push("gap");
    out.push(n);
    previous = n;
  }
  return out;
}

export function Pager({ page, pageSize, total, capped = false, onPage, onPageSize }: PagerProps) {
  const pages = Math.max(1, Math.ceil(total / pageSize));
  const safePage = Math.min(Math.max(1, page), pages);
  return (
    <div className="pager">
      <span className="small muted">
        共 {capped ? `${total}+` : total} 条 · 第 {safePage}/{pages} 页
      </span>
      <div className="row" style={{ gap: 6, flexWrap: "wrap" }}>
        <button type="button" className="btn btn-ghost btn-sm" disabled={safePage <= 1} onClick={() => onPage(safePage - 1)} aria-label="上一页">
          <IconChevronLeft size={14} />
        </button>
        {pageWindow(safePage, pages).map((item, index) =>
          item === "gap" ? (
            <span className="small muted" key={`gap-${index}`}>
              …
            </span>
          ) : (
            <button
              type="button"
              key={item}
              className={`btn btn-sm${item === safePage ? " is-active" : " btn-ghost"}`}
              onClick={() => onPage(item)}
            >
              {item}
            </button>
          ),
        )}
        <button type="button" className="btn btn-ghost btn-sm" disabled={safePage >= pages} onClick={() => onPage(safePage + 1)} aria-label="下一页">
          <IconChevronRight size={14} />
        </button>
        {onPageSize ? (
          <select
            className="input input-sm"
            style={{ width: "auto" }}
            value={pageSize}
            onChange={(event) => onPageSize(Number(event.target.value))}
            aria-label="每页条数"
          >
            {PAGE_SIZES.map((size) => (
              <option key={size} value={size}>
                每页 {size}
              </option>
            ))}
          </select>
        ) : null}
      </div>
    </div>
  );
}

/** 客户端分页：已在本地的筛选结果按页切片（信源/计划等配置级列表）。 */
export function slicePage<T>(rows: T[], page: number, pageSize: number): T[] {
  const safePage = Math.max(1, page);
  return rows.slice((safePage - 1) * pageSize, safePage * pageSize);
}
