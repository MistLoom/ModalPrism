// Markdown 到「受限 HTML」的转换：只认标题、段落、列表、引用、代码、表格与行内链接。
// 产出仍然是 HTML 片段，必须再经 sanitizeArticleHtml 的白名单重建后渲染；
// 这里不处理原始 HTML（Markdown 里的 <script> 等一律当字面文本转义）。

const ESCAPES: Record<string, string> = {
  "&": "&amp;",
  "<": "&lt;",
  ">": "&gt;",
  '"': "&quot;",
  "'": "&#39;",
};

/** 单条正文上限：超出截断，避免异常内容把界面拖死。 */
const MAX_SOURCE = 400_000;

function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/g, (char) => ESCAPES[char] ?? char);
}

/** 行内语法：代码、粗体、斜体、删除线、链接。先转义再套标签，保证不会注入原始 HTML。 */
function inline(text: string): string {
  const codes: string[] = [];
  let output = text.replace(/`([^`]+)`/g, (_match, code: string) => {
    codes.push(`<code>${escapeHtml(code)}</code>`);
    return `\u0000${codes.length - 1}\u0000`;
  });
  output = escapeHtml(output);
  output = output.replace(/!\[([^\]]*)\]\(([^)\s]+)\)/g, (_match, alt: string, src: string) =>
    `<img src="${src}" alt="${alt}" />`,
  );
  output = output.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (_match, label: string, href: string) =>
    `<a href="${href}">${label}</a>`,
  );
  output = output.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  output = output.replace(/(^|[\s(])\*([^*\n]+)\*/g, "$1<em>$2</em>");
  output = output.replace(/~~([^~]+)~~/g, "<del>$1</del>");
  return output.replace(/\u0000(\d+)\u0000/g, (_match, index: string) => codes[Number(index)] ?? "");
}

const HEADING = /^(#{1,6})\s+(.*)$/;
const UL_ITEM = /^\s*[-*+]\s+(.*)$/;
const OL_ITEM = /^\s*\d+[.)]\s+(.*)$/;
const QUOTE = /^>\s?(.*)$/;
const FENCE = /^\s*(`{3,}|~{3,})\s*([\w-]*)\s*$/;
const HR = /^\s*([-*_])(\s*\1){2,}\s*$/;
const TABLE_DIVIDER = /^\s*\|?[\s:|-]+\|[\s:|-]*$/;

/** 表格行拆列：去掉首尾竖线后按竖线切分。 */
function cells(line: string): string[] {
  return line.replace(/^\s*\|/, "").replace(/\|\s*$/, "").split("|").map((cell) => cell.trim());
}

/**
 * 渲染 Markdown 为受限 HTML 片段。返回值必须再走 sanitizeArticleHtml，
 * 因此这里只需要保证「不产生原始 HTML 透传」，无需重复做协议白名单。
 */
export function renderMarkdown(source: string): string {
  const lines = source.slice(0, MAX_SOURCE).replace(/\r\n?/g, "\n").split("\n");
  const out: string[] = [];
  let index = 0;
  while (index < lines.length) {
    const line = lines[index];
    if (!line.trim()) {
      index += 1;
      continue;
    }
    const fence = FENCE.exec(line);
    if (fence) {
      const marker = fence[1];
      const language = fence[2] ?? "";
      const body: string[] = [];
      index += 1;
      while (index < lines.length && !new RegExp(`^\\s*${marker[0]}{${marker.length},}\\s*$`).test(lines[index])) {
        body.push(lines[index]);
        index += 1;
      }
      index += 1;
      const label = language ? `<code class="language-${escapeHtml(language)}">` : "<code>";
      out.push(`<pre>${label}${escapeHtml(body.join("\n"))}</code></pre>`);
      continue;
    }
    if (HR.test(line)) {
      out.push("<hr />");
      index += 1;
      continue;
    }
    const heading = HEADING.exec(line);
    if (heading) {
      const level = heading[1].length;
      out.push(`<h${level}>${inline(heading[2].trim())}</h${level}>`);
      index += 1;
      continue;
    }
    if (QUOTE.test(line)) {
      const body: string[] = [];
      while (index < lines.length && QUOTE.test(lines[index])) {
        body.push((QUOTE.exec(lines[index]) as RegExpExecArray)[1]);
        index += 1;
      }
      out.push(`<blockquote>${renderMarkdown(body.join("\n"))}</blockquote>`);
      continue;
    }
    if (UL_ITEM.test(line) || OL_ITEM.test(line)) {
      const ordered = OL_ITEM.test(line);
      const pattern = ordered ? OL_ITEM : UL_ITEM;
      const items: string[] = [];
      while (index < lines.length) {
        const item = pattern.exec(lines[index]);
        if (item) {
          items.push(`<li>${inline(item[1])}</li>`);
          index += 1;
          continue;
        }
        // 列表项的续行（缩进文本）并入上一项。
        if (items.length && /^\s{2,}\S/.test(lines[index])) {
          items[items.length - 1] = items[items.length - 1].replace(/<\/li>$/, ` ${inline(lines[index].trim())}</li>`);
          index += 1;
          continue;
        }
        break;
      }
      const tag = ordered ? "ol" : "ul";
      out.push(`<${tag}>${items.join("")}</${tag}>`);
      continue;
    }
    if (line.includes("|") && index + 1 < lines.length && TABLE_DIVIDER.test(lines[index + 1])) {
      const head = cells(line);
      index += 2;
      const rows: string[][] = [];
      while (index < lines.length && lines[index].includes("|") && lines[index].trim()) {
        rows.push(cells(lines[index]));
        index += 1;
      }
      const headHtml = head.map((cell) => `<th>${inline(cell)}</th>`).join("");
      const bodyHtml = rows
        .map((row) => `<tr>${row.map((cell) => `<td>${inline(cell)}</td>`).join("")}</tr>`)
        .join("");
      out.push(`<table><thead><tr>${headHtml}</tr></thead><tbody>${bodyHtml}</tbody></table>`);
      continue;
    }
    // 普通段落：连续非空、非其它块起始的行合并。
    const paragraph: string[] = [];
    while (index < lines.length && lines[index].trim()) {
      const current = lines[index];
      if (paragraph.length && (HEADING.test(current) || FENCE.test(current) || HR.test(current) || QUOTE.test(current))) break;
      if (paragraph.length && (UL_ITEM.test(current) || OL_ITEM.test(current))) break;
      paragraph.push(current.trim());
      index += 1;
    }
    out.push(`<p>${inline(paragraph.join(" "))}</p>`);
  }
  return out.join("\n");
}
