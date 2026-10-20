export type ConfigEditorHandle = {
  getValue: () => string;
  setValue: (text: string) => void;
  setDiskVersion: (text: string) => void;
  setErrorLine: (line: number | null) => void;
  focus: () => void;
};

export function mountConfigEditor(
  host: HTMLElement,
  initial: string,
  diskText: string,
  onDirty: (dirty: boolean) => void,
): ConfigEditorHandle {
  host.innerHTML = `
    <div class="cfg-shell">
      <div class="cfg-gutter" id="cfgGutter" aria-hidden="true"></div>
      <div class="cfg-main">
        <textarea class="cfg-textarea" spellcheck="false" aria-label="YAML 编辑器"></textarea>
        <pre class="cfg-diff-view" id="cfgDiffView" hidden></pre>
      </div>
    </div>
    <div class="cfg-toolbar">
      <button type="button" class="btn btn-sm btn-quiet" id="cfgToggleDiff">查看与磁盘差异</button>
      <span class="cfg-diff-hint" id="cfgDiffHint"></span>
    </div>
  `;
  const textarea = host.querySelector(".cfg-textarea") as HTMLTextAreaElement;
  const gutter = host.querySelector("#cfgGutter") as HTMLElement;
  const diffView = host.querySelector("#cfgDiffView") as HTMLElement;
  const diffHint = host.querySelector("#cfgDiffHint") as HTMLElement;
  const toggleDiff = host.querySelector("#cfgToggleDiff") as HTMLButtonElement;
  let disk = diskText;
  let errorLine: number | null = null;
  let showDiff = false;

  textarea.value = initial;

  const renderGutter = () => {
    const lines = textarea.value.split("\n");
    gutter.innerHTML = lines
      .map((_, i) => {
        const n = i + 1;
        const bad = errorLine === n ? " cfg-line-error" : "";
        return `<span class="cfg-ln${bad}">${n}</span>`;
      })
      .join("");
  };

  const renderDiff = () => {
    const a = disk.split("\n");
    const b = textarea.value.split("\n");
    const max = Math.max(a.length, b.length);
    const rows: string[] = [];
    for (let i = 0; i < max; i++) {
      const left = a[i] ?? "";
      const right = b[i] ?? "";
      if (left === right) {
        rows.push(`<span class="diff-eq"> ${escapeHtml(right)}</span>`);
      } else if (left && !right) {
        rows.push(`<span class="diff-del">-${escapeHtml(left)}</span>`);
      } else if (!left && right) {
        rows.push(`<span class="diff-add">+${escapeHtml(right)}</span>`);
      } else {
        rows.push(`<span class="diff-del">-${escapeHtml(left)}</span>`);
        rows.push(`<span class="diff-add">+${escapeHtml(right)}</span>`);
      }
    }
    diffView.innerHTML = rows.join("\n");
  };

  const sync = () => {
    renderGutter();
    const dirty = textarea.value !== disk;
    diffHint.textContent = dirty ? "与磁盘版本有差异（未保存）" : "与磁盘版本一致";
    if (showDiff) renderDiff();
    onDirty(dirty);
  };

  textarea.addEventListener("input", sync);
  textarea.addEventListener("scroll", () => {
    gutter.scrollTop = textarea.scrollTop;
  });

  toggleDiff.addEventListener("click", () => {
    showDiff = !showDiff;
    textarea.hidden = showDiff;
    diffView.hidden = !showDiff;
    toggleDiff.textContent = showDiff ? "返回编辑" : "查看与磁盘差异";
    if (showDiff) renderDiff();
  });

  sync();

  return {
    getValue: () => textarea.value,
    setValue(text: string) {
      textarea.value = text;
      sync();
    },
    setDiskVersion(text: string) {
      disk = text;
      sync();
    },
    setErrorLine(line: number | null) {
      errorLine = line;
      renderGutter();
      if (line != null) {
        const idx = line - 1;
        const lines = textarea.value.split("\n");
        if (idx >= 0 && idx < lines.length) {
          textarea.focus();
          textarea.setSelectionRange(
            lines.slice(0, idx).join("\n").length + (idx > 0 ? 1 : 0),
            lines.slice(0, idx + 1).join("\n").length,
          );
        }
      }
    },
    focus: () => textarea.focus(),
  };
}

function escapeHtml(s: string) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;");
}
