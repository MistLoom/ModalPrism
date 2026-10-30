import { useMemo } from "react";

import { isHtmlBody, sanitizeArticleHtml } from "../lib/article";
import { Empty } from "./ui";

const SAFE_HREF = /^https?:\/\//i;

/** 正文阅读区：白名单化后的 HTML 走只读容器渲染，原文入口单独放在右上角。 */
export function ArticleBody({ html, base, bodyKind, mediaType }: {
  html: string;
  base?: string;
  bodyKind?: string | null;
  mediaType?: string | null;
}) {
  const safe = useMemo(() => sanitizeArticleHtml(html, base ?? ""), [html, base]);
  if (!safe) {
    return <Empty title="没有可读正文" text="该内容仅含结构化字段或附件，无可展示正文。" />;
  }
  const label = isHtmlBody(bodyKind, mediaType, html) ? "网页正文" : bodyKind === "md" ? "Markdown" : "纯文本";
  return (
    <div className="stack" style={{ gap: 10 }}>
      <div className="row" style={{ gap: 8, alignItems: "center" }}>
        <span className="badge neutral">{label}</span>
        <span className="small muted">已按白名单清除脚本、样式与外框</span>
        <span className="fill" />
        {base && SAFE_HREF.test(base) ? (
          <a className="btn btn-sm" href={base} target="_blank" rel="noopener noreferrer">打开原文</a>
        ) : null}
      </div>
      {/* safe 只含 sanitizeArticleHtml 重建后的节点：标签、属性、协议都已按白名单过滤。 */}
      <div className="article-body" dangerouslySetInnerHTML={{ __html: safe }} />
    </div>
  );
}
