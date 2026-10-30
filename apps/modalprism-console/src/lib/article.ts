// 正文渲染的公共逻辑：采集来的正文多数是原始网页 HTML（kind=html / text/html），
// 直接按纯文本输出会把标签源码暴露给用户，按 innerHTML 塞进去又等于执行第三方脚本。
// 这里的做法是：识别正文形态 → 用 DOMParser 解析 → 按白名单重建节点与属性，
// 丢弃脚本、样式、表单、外框等一切可执行或可破坏布局的内容，再交给 ArticleBody 渲染。
// 只依赖浏览器原生 DOMParser，不引入额外依赖。

/** 直接整棵删除的标签：可执行、可外链资源或会劫持页面布局。 */
const BLOCKED = new Set([
  "script", "style", "link", "meta", "base", "title", "head",
  "iframe", "frame", "frameset", "object", "embed", "applet", "portal",
  "form", "input", "button", "select", "option", "optgroup", "textarea", "label", "fieldset",
  "svg", "math", "template", "noscript", "canvas", "dialog", "slot", "audio", "video", "source", "track",
]);

/** 允许保留的正文标签：覆盖常见文章、列表与表格结构。 */
const ALLOWED = new Set([
  "p", "div", "span", "section", "article", "header", "footer", "main", "aside", "nav",
  "br", "hr", "h1", "h2", "h3", "h4", "h5", "h6",
  "ul", "ol", "li", "dl", "dt", "dd",
  "blockquote", "pre", "code", "kbd", "samp",
  "strong", "b", "em", "i", "u", "s", "del", "ins", "small", "sub", "sup", "mark", "abbr",
  "figure", "figcaption", "img",
  "a", "table", "thead", "tbody", "tfoot", "tr", "th", "td", "caption", "time",
  "details", "summary",
]);

/** 每个标签允许保留的属性；其余属性（style/class/id/on* 等）一律删除。 */
const ATTRIBUTES: Record<string, string[]> = {
  a: ["href", "title"],
  img: ["src", "alt", "title"],
  td: ["colspan", "rowspan"],
  th: ["colspan", "rowspan", "scope"],
  time: ["datetime"],
};

/** 正文解析上限：超出部分截断，避免一条异常内容把界面拖死。 */
const MAX_HTML = 400_000;

/** 嵌套深度上限：正常文章远达不到，超过视为异常结构并丢弃该子树。 */
const MAX_DEPTH = 120;

/** 常见正文标签的出现，用来在服务端没给 mediaType 时兜底判断。 */
const HTML_HINT = /<\/?(article|div|p|br|span|a|img|ul|ol|li|h[1-6]|table|thead|tbody|tr|td|blockquote|pre|section|figure)\b[^>]*>/i;

/** 正文是否是 HTML：以 facet 形态为准，缺形态时退回嗅探。 */
export function isHtmlBody(kind?: string | null, mediaType?: string | null, text?: string): boolean {
  if (kind === "html") return true;
  if ((mediaType ?? "").toLowerCase().includes("html")) return true;
  return Boolean(text && HTML_HINT.test(text));
}

/** 地址白名单：只放行可安全跳转的协议；图片只认 http/https。 */
function safeUrl(raw: string, base: string, image: boolean): string | null {
  const value = raw.trim();
  if (!value) return null;
  if (/^(javascript|vbscript|data|file|blob|about):/i.test(value)) return null;
  try {
    const resolved = new URL(value, base || undefined);
    if (image) return resolved.protocol === "http:" || resolved.protocol === "https:" ? resolved.href : null;
    return ["http:", "https:", "mailto:"].includes(resolved.protocol) ? resolved.href : null;
  } catch {
    return null;
  }
}

function clean(parent: Element | DocumentFragment, base: string, depth: number): void {
  for (const node of Array.from(parent.childNodes)) {
    if (node.nodeType === Node.COMMENT_NODE) {
      node.remove();
      continue;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) continue;
    const element = node as Element;
    const tag = element.tagName.toLowerCase();
    if (BLOCKED.has(tag) || depth > MAX_DEPTH) {
      element.remove();
      continue;
    }
    clean(element, base, depth + 1);
    if (!ALLOWED.has(tag)) {
      // 未知但无害的标签（自定义组件、版式包裹层）保留文字，去掉节点本身。
      const parentNode = element.parentNode;
      if (!parentNode) continue;
      while (element.firstChild) parentNode.insertBefore(element.firstChild, element);
      element.remove();
      continue;
    }
    const keep = ATTRIBUTES[tag] ?? [];
    for (const attribute of Array.from(element.attributes)) {
      if (!keep.includes(attribute.name.toLowerCase())) element.removeAttribute(attribute.name);
    }
    if (tag === "a") {
      const href = element.getAttribute("href");
      const safe = href ? safeUrl(href, base, false) : null;
      if (safe) {
        element.setAttribute("href", safe);
        element.setAttribute("target", "_blank");
        element.setAttribute("rel", "noopener noreferrer");
      } else {
        element.removeAttribute("href");
      }
    } else if (tag === "img") {
      const src = element.getAttribute("src");
      const safe = src ? safeUrl(src, base, true) : null;
      if (safe) {
        element.setAttribute("src", safe);
        element.setAttribute("loading", "lazy");
        element.setAttribute("decoding", "async");
        element.setAttribute("referrerpolicy", "no-referrer");
      } else {
        element.remove();
      }
    }
  }
}

/** 按白名单把正文 HTML 重建为可安全渲染的片段；base 用于补齐相对地址。 */
export function sanitizeArticleHtml(html: string, base = ""): string {
  const source = html.trim();
  if (!source) return "";
  const doc = new DOMParser().parseFromString(source.slice(0, MAX_HTML), "text/html");
  const origin = base || (typeof window === "undefined" ? "http://localhost/" : window.location.href);
  clean(doc.body, origin, 0);
  return doc.body.innerHTML.trim();
}
