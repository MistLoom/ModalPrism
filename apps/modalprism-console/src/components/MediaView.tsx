import { useEffect, useState } from "react";

import { contentFacetBlobUrl } from "../lib/api";
import { bytes as formatBytes } from "../lib/format";
import { Empty, Loading } from "./ui";

/**
 * 二进制数据面（图片/音视频/文件）的查看器。
 * 媒体标签无法携带项目头，所以先按项目上下文取回字节再生成对象地址；换面或卸载时释放。
 */
export function MediaView({ contentId, kind, variant, mediaType, size, href }: {
  contentId: string;
  kind: string;
  variant?: string | null;
  mediaType?: string | null;
  size?: number | null;
  /** 结构化面里声明的原始地址：取不到字节时作为回退入口。 */
  href?: string | null;
}) {
  const [url, setUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let alive = true;
    let created: string | null = null;
    setLoading(true);
    setError(null);
    setUrl(null);
    void (async () => {
      try {
        created = await contentFacetBlobUrl(contentId, kind, variant);
        if (!alive) {
          URL.revokeObjectURL(created);
          return;
        }
        setUrl(created);
        created = null;
      } catch (problem) {
        if (alive) setError(problem instanceof Error ? problem.message : "附件读取失败");
      } finally {
        if (alive) setLoading(false);
      }
    })();
    return () => {
      alive = false;
      if (created) URL.revokeObjectURL(created);
      setUrl((current) => {
        if (current) URL.revokeObjectURL(current);
        return null;
      });
    };
  }, [contentId, kind, variant]);

  if (loading) return <Loading lines={3} />;
  if (error || !url) {
    return (
      <Empty
        title="附件不可查看"
        text={error ?? "无可用附件字节：尚未下载或超出下载限额。"}
        action={href ? <a className="btn btn-sm" href={href} target="_blank" rel="noreferrer">打开原始地址</a> : undefined}
      />
    );
  }

  const type = (mediaType ?? "").toLowerCase();
  const meta = (
    <div className="row" style={{ gap: 8, alignItems: "center" }}>
      <span className="badge neutral">{kind === "image" ? "图片" : kind === "audio" ? "音频" : kind === "video" ? "视频" : "文件"}</span>
      {mediaType ? <span className="small muted">{mediaType}</span> : null}
      {size ? <span className="small muted">{formatBytes(size)}</span> : null}
      <span className="fill" />
      <a className="btn btn-sm" href={url} download={`${contentId}-${kind}${variant ? `-${variant}` : ""}`}>下载</a>
    </div>
  );

  if (kind === "image" || type.startsWith("image/")) {
    return (
      <div className="stack" style={{ gap: 10 }}>
        {meta}
        <img src={url} alt="内容图片" style={{ maxWidth: "100%", borderRadius: 8 }} />
      </div>
    );
  }
  if (kind === "audio" || type.startsWith("audio/")) {
    return (
      <div className="stack" style={{ gap: 10 }}>
        {meta}
        <audio src={url} controls style={{ width: "100%" }} />
      </div>
    );
  }
  if (kind === "video" || type.startsWith("video/")) {
    return (
      <div className="stack" style={{ gap: 10 }}>
        {meta}
        <video src={url} controls style={{ maxWidth: "100%", borderRadius: 8 }} />
      </div>
    );
  }
  return (
    <div className="stack" style={{ gap: 10 }}>
      {meta}
      <div className="alert info">该媒体类型不支持浏览器预览，可下载后查看。</div>
    </div>
  );
}
