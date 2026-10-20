import { api } from "./api";
import { formatLocalTime } from "./format";
import { loadTheme, saveTheme, type ThemeMode } from "./theme";
import { mountConfigEditor, type ConfigEditorHandle } from "./ui/configEditor";
import { mountSelect } from "./ui/select";

export type Route = "overview" | "sources" | "records" | "governance" | "config";

const routes: { id: Route; label: string }[] = [
  { id: "overview", label: "总览" },
  { id: "sources", label: "信源" },
  { id: "records", label: "数据记录" },
  { id: "governance", label: "治理" },
  { id: "config", label: "配置" },
];

export type AppState = {
  route: Route;
  loading: boolean;
  error: string | null;
  meta: Record<string, unknown> | null;
  asOfSnapshot: number | null;
  configDirty: boolean;
  configPath: string | null;
};

export function createApp(root: HTMLElement) {
  const state: AppState = {
    route: "overview",
    loading: true,
    error: null,
    meta: null,
    asOfSnapshot: null,
    configDirty: false,
    configPath: null,
  };
  let diskConfigText = "";
  let cfgEditor: ConfigEditorHandle | null = null;
  const readOnly = () => state.asOfSnapshot != null;

  const shell = document.createElement("div");
  shell.className = "shell";
  shell.innerHTML = `
    <aside class="sidebar" aria-label="主导航">
      <div class="brand">
        <svg class="brand-mark" viewBox="0 0 32 32" aria-hidden="true">
          <path d="M16 3.5 28 26H4L16 3.5Z" fill="none" stroke="currentColor" stroke-width="1.2"/>
          <path d="M16 3.5 22 26" stroke="var(--cinnabar)" stroke-width="1.2" fill="none"/>
        </svg>
        <div class="brand-text">
          <span class="brand-name">ModalPrism</span>
          <span class="brand-sub">沉积观测台</span>
        </div>
      </div>
      <nav class="nav" id="nav"></nav>
      <div class="sidebar-foot" id="foot"></div>
    </aside>
    <div class="main">
      <header class="topbar">
        <div class="crumb"><span class="crumb-root">观测台</span><span class="crumb-sep">/</span><span id="crumb"></span></div>
        <button type="button" class="search-trigger" id="paletteBtn"><span class="search-text">命令面板</span><kbd>Ctrl K</kbd></button>
        <div class="segmented" id="themeSeg" role="group" aria-label="主题">
          <button type="button" data-theme="light">浅色</button>
          <button type="button" data-theme="dark">深色</button>
          <button type="button" data-theme="system">系统</button>
        </div>
      </header>
      <div id="historyBanner" class="history-banner" hidden role="status"></div>
      <main id="viewRoot" class="view" tabindex="-1"></main>
    </div>
    <div class="palette-layer" id="palette" hidden>
      <div class="palette-scrim"></div>
      <section class="palette" role="dialog" aria-label="命令面板">
        <input id="paletteInput" class="palette-input" placeholder="跳转视图或校验配置…" />
        <div id="paletteResults" class="palette-results"></div>
      </section>
    </div>
    <div class="confirm-layer" id="confirm" hidden>
      <div class="confirm-scrim"></div>
      <section class="confirm" role="alertdialog">
        <h2 id="confirmTitle"></h2>
        <p id="confirmBody"></p>
        <div class="confirm-actions">
          <button type="button" class="btn btn-quiet" id="confirmCancel">取消</button>
          <button type="button" class="btn btn-primary" id="confirmOk">确认</button>
        </div>
      </section>
    </div>
    <div class="toasts" id="toasts" role="status" aria-live="polite"></div>
  `;
  root.appendChild(shell);

  const nav = shell.querySelector("#nav") as HTMLElement;
  const view = shell.querySelector("#viewRoot") as HTMLElement;
  const crumb = shell.querySelector("#crumb") as HTMLElement;
  const foot = shell.querySelector("#foot") as HTMLElement;
  const palette = shell.querySelector("#palette") as HTMLElement;
  const paletteInput = shell.querySelector("#paletteInput") as HTMLInputElement;
  const confirmLayer = shell.querySelector("#confirm") as HTMLElement;

  function toast(title: string, body: string) {
    const el = document.createElement("div");
    el.className = "toast";
    el.innerHTML = `<strong>${title}</strong><p>${body}</p>`;
    shell.querySelector("#toasts")!.appendChild(el);
    setTimeout(() => el.remove(), 4000);
  }

  function confirm(title: string, body: string): Promise<boolean> {
    return new Promise((resolve) => {
      (shell.querySelector("#confirmTitle") as HTMLElement).textContent = title;
      (shell.querySelector("#confirmBody") as HTMLElement).textContent = body;
      confirmLayer.hidden = false;
      const ok = shell.querySelector("#confirmOk") as HTMLButtonElement;
      const cancel = shell.querySelector("#confirmCancel") as HTMLButtonElement;
      const done = (v: boolean) => {
        confirmLayer.hidden = true;
        ok.onclick = null;
        cancel.onclick = null;
        resolve(v);
      };
      ok.onclick = () => done(true);
      cancel.onclick = () => done(false);
    });
  }

  async function setRoute(route: Route) {
    if (state.configDirty && state.route === "config" && route !== "config") {
      if (!(await confirm("未保存的修改", "离开配置页将丢失未保存的 YAML 修改。"))) return;
      state.configDirty = false;
    }
    state.route = route;
    if (location.hash.replace(/^#/, "") !== route) {
      location.hash = route;
    }
    renderNav();
    void renderView();
  }

  function setAsOf(seq: number | null) {
    state.asOfSnapshot = seq;
    const banner = shell.querySelector("#historyBanner") as HTMLElement;
    if (seq == null) {
      banner.hidden = true;
    } else {
      banner.hidden = false;
      banner.innerHTML = `<div class="hb-copy"><strong>历史状态 #${seq}</strong><span class="hb-note">数据记录按所选时间点显示；其他视图为实时数据且不可写入。</span></div><button type="button" class="btn btn-sm btn-quiet" id="liveBtn">回到实时</button>`;
      banner.querySelector("#liveBtn")?.addEventListener("click", () => setAsOf(null));
    }
    void refreshMeta().then(() => renderView());
  }

  window.addEventListener("beforeunload", (e) => {
    if (state.configDirty) {
      e.preventDefault();
    }
  });

  function renderNav() {
    const observe = routes.filter((r) => r.id === "overview" || r.id === "sources" || r.id === "records");
    const system = routes.filter((r) => r.id === "governance" || r.id === "config");
    const item = (r: { id: Route; label: string }) =>
      `<button type="button" class="nav-item" data-route="${r.id}" aria-current="${state.route === r.id ? "page" : "false"}"><span class="nav-icon" aria-hidden="true"></span><span>${r.label}</span></button>`;
    nav.innerHTML = `
      <div class="nav-group"><span class="nav-label">观测</span>${observe.map(item).join("")}</div>
      <div class="nav-group"><span class="nav-label">系统</span>${system.map(item).join("")}</div>`;
    nav.querySelectorAll("[data-route]").forEach((btn) => {
      btn.addEventListener("click", () => setRoute((btn as HTMLElement).dataset.route as Route));
    });
    crumb.textContent = routes.find((r) => r.id === state.route)?.label ?? "";
  }

  function viewNoteHtml(note?: string) {
    if (!note || state.route === "records") return "";
    return `<div class="view-note" role="status">${note}</div>`;
  }

  function parseErrorLine(message: string): number | null {
    const m = message.match(/第\s*(\d+)\s*行/);
    return m ? Number(m[1]) : null;
  }

  async function openRecordDrawer(id: string) {
    const drawer = view.querySelector("#recordDrawer") as HTMLElement | null;
    if (!drawer) return;
    drawer.hidden = false;
    const load = async () => {
      drawer.innerHTML = `
        <header class="drawer-head">
          <h2 class="drawer-title" id="drawerTitle">加载中…</h2>
          <button type="button" class="icon-btn drawer-close" id="closeDrawer" aria-label="关闭">×</button>
        </header>
        <div class="drawer-body"><p class="loading">加载中…</p></div>`;
      drawer.querySelector("#closeDrawer")?.addEventListener("click", () => {
        drawer.hidden = true;
      });
      try {
        const d = await api.record(id, state.asOfSnapshot);
        const title = (d.title as string) || "未命名";
        drawer.innerHTML = `
          <header class="drawer-head">
            <h2 class="drawer-title">${escapeHtml(title)}</h2>
            <button type="button" class="icon-btn drawer-close" id="closeDrawer" aria-label="关闭">×</button>
          </header>
          <div class="drawer-body">
            <p class="muted">${escapeHtml(String(d.sourceName ?? d.sourceId ?? ""))}</p>
            <dl class="detail-dl">
              <dt>发布时间</dt><dd>${formatLocalTime(d.publishedAt as string)}</dd>
              <dt>采集时间</dt><dd>${formatLocalTime((d.collectedAt ?? d.updatedAt) as string)}</dd>
              <dt>原文</dt><dd>${d.url ? `<a href="${d.url}" target="_blank" rel="noopener">${escapeHtml(String(d.url))}</a>` : "—"}</dd>
              <dt>摘要</dt><dd>${escapeHtml(String(d.summary ?? "—"))}</dd>
            </dl>
            <pre class="body-preview">${escapeHtml(String(d.bodyPreview ?? ""))}</pre>
            <details class="tech-details"><summary>技术详情</summary><pre>${escapeHtml(JSON.stringify(d.technical, null, 2))}</pre></details>
          </div>`;
        drawer.querySelector("#closeDrawer")?.addEventListener("click", () => {
          drawer.hidden = true;
        });
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        drawer.innerHTML = `
          <header class="drawer-head">
            <h2 class="drawer-title">无法加载记录</h2>
            <button type="button" class="icon-btn drawer-close" id="closeDrawer" aria-label="关闭">×</button>
          </header>
          <div class="drawer-body error"><p>${escapeHtml(msg)}</p>
            <button type="button" class="btn btn-primary" id="retryDrawer">重试</button></div>`;
        drawer.querySelector("#closeDrawer")?.addEventListener("click", () => {
          drawer.hidden = true;
        });
        drawer.querySelector("#retryDrawer")?.addEventListener("click", () => void load());
      }
    };
    await load();
  }

  function escapeHtml(s: string) {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/"/g, "&quot;");
  }

  async function renderView() {
    state.loading = true;
    state.error = null;
    view.innerHTML = `<p class="loading">加载中…</p>`;
    try {
      if (state.route === "overview") {
        const data = await api.overview();
        const sources = (data.sources as Record<string, unknown>[]) ?? [];
        const note = data.viewNote as string | undefined;
        view.innerHTML = `
          ${viewNoteHtml(note)}
          <div class="page-head"><div><h1 class="page-title">总览</h1><p class="page-desc">采集进程与信源健康摘要。</p></div></div>
          <div class="stat-grid">
            <div class="stat od-stat"><span class="stat-label">信源</span><strong class="stat-value">${data.sourceCount}</strong><span class="stat-hint">已声明的采集信源</span></div>
            <div class="stat od-stat"><span class="stat-label">健康</span><strong class="stat-value">${data.healthyCount}</strong><span class="stat-hint">状态为正常的信源</span></div>
            <div class="stat od-stat"><span class="stat-label">配置版本</span><strong class="stat-value">${state.meta?.configGeneration ?? "—"}</strong><span class="stat-hint">当前生效的配置代际</span></div>
          </div>
          <h2 class="section-title">信源状态</h2>
          <table class="table"><thead><tr><th>名称</th><th>状态</th><th>下次采集</th><th>最近采集</th><th>今日条数</th><th>说明</th></tr></thead>
          <tbody>${sources
            .slice(0, 16)
            .map((s) => {
              const chip = String(s.statusChipClass ?? "status-muted");
              const label = s.skipReasonLabel ?? s.healthLabel ?? s.healthStatus;
              const next = s.nextAt ? formatLocalTime(s.nextAt as string) : "—";
              const last = s.lastYieldAt ? formatLocalTime(s.lastYieldAt as string) : "—";
              const count = s.todayYieldCount ?? 0;
              const noteText = s.enabled === false ? "已停用" : (s.scheduleNote ?? s.lastError ?? "—");
              return `<tr><td>${escapeHtml(String(s.name))}</td><td><span class="status-chip ${chip}">${escapeHtml(String(label))}</span></td><td>${next}</td><td>${last}</td><td>${count}</td><td class="muted">${escapeHtml(String(noteText))}</td></tr>`;
            })
            .join("")}</tbody></table>`;
      } else if (state.route === "sources") {
        const data = await api.sources();
        const items = data.items;
        const ro = readOnly() || Boolean(data.readOnly);
        view.innerHTML = `
          ${viewNoteHtml(data.viewNote as string)}
          <div class="page-head"><div><h1 class="page-title">信源</h1><p class="page-desc">已声明的采集信源。启用状态与调度来自 YAML 配置。</p></div></div>
          <table class="table"><thead><tr><th>信源</th><th>访问器</th><th>状态</th><th>操作</th></tr></thead>
          <tbody>${items
            .map((s) => {
              const chip = String(s.statusChipClass ?? "status-muted");
              const label = s.skipReasonLabel ?? s.healthLabel ?? s.healthStatus;
              return `<tr data-id="${s.id}">
                <td>${escapeHtml(String(s.name))}<div class="muted mono">${escapeHtml(String(s.id))}</div></td>
                <td>${escapeHtml(String(s.accessor))}</td>
                <td><span class="status-chip ${chip}">${escapeHtml(String(label))}</span><div class="muted">${escapeHtml(String(s.scheduleNote ?? ""))}</div></td>
                <td>
                  <button type="button" class="btn btn-sm" data-act="trigger" ${ro ? "disabled" : ""}>立即采集</button>
                  <button type="button" class="btn btn-sm btn-quiet" data-act="toggle" ${ro ? "disabled" : ""}>${s.enabled ? "停用" : "启用"}</button>
                </td></tr>`;
            })
            .join("")}</tbody></table>`;
        view.querySelectorAll("[data-act]").forEach((btn) => {
          const row = btn.closest("tr") as HTMLElement;
          const id = row.dataset.id!;
          btn.addEventListener("click", async () => {
            if (ro) return;
            const act = (btn as HTMLElement).dataset.act;
            if (act === "trigger") {
              await api.trigger(id.replace("source/", ""));
              toast("已触发", id);
            } else if (act === "toggle") {
              const enabled = btn.textContent === "启用";
              if (!(await confirm("修改信源启用状态？", `${id} → enabled=${enabled}`))) return;
              await api.patchSource(id.replace("source/", ""), enabled);
              toast("已更新", "配置已校验并重新加载");
              void renderView();
            }
          });
        });
      } else if (state.route === "records") {
        const sourceFilter = (view.dataset.sourceFilter as string) || "";
        const q = (view.dataset.q as string) || "";
        const { items, nextCursor, sourceOptions } = await api.records({
          limit: 50,
          source: sourceFilter || undefined,
          q: q || undefined,
          asOf: state.asOfSnapshot,
        });
        view.innerHTML = `
          <div class="page-head"><div><h1 class="page-title">数据记录</h1><p class="page-desc">已沉积的采集条目。${state.asOfSnapshot != null ? "当前按所选历史状态显示。" : ""}</p></div></div>
          <div class="filter-row">
            <input id="recQ" class="input" placeholder="按标题搜索…" value="${escapeHtml(q)}" />
            <div id="recSourceSelect"></div>
            <button type="button" class="btn" id="recFilter">筛选</button>
          </div>
          ${
            items.length === 0
              ? `<div class="empty">尚无记录。完成一轮采集并等待数据发布后刷新。</div>`
              : `<table class="table"><thead><tr><th>标题</th><th>信源</th><th>发布时间</th><th>采集时间</th></tr></thead>
          <tbody>${items
            .map((r) => {
              const title = (r.title as string) || String(r.id);
              const src = (r.sourceName as string) || (r.sourceId as string) || "—";
              return `<tr class="rec-row" data-id="${r.id}"><td class="od-clamp-2">${escapeHtml(title)}</td><td>${escapeHtml(src)}</td><td>${formatLocalTime(r.publishedAt as string)}</td><td>${formatLocalTime((r.collectedAt ?? r.updatedAt) as string)}</td></tr>`;
            })
            .join("")}</tbody></table>
          ${nextCursor ? `<button type="button" class="btn" id="moreRec">加载更多</button>` : ""}`
          }
          <aside class="drawer" id="recordDrawer" hidden></aside>`;
        const srcHost = view.querySelector("#recSourceSelect") as HTMLElement;
        mountSelect(
          srcHost,
          [{ value: "", label: "全部信源" }, ...(sourceOptions ?? []).map((o) => ({ value: o.id, label: o.name }))],
          sourceFilter,
          (v) => {
            view.dataset.sourceFilter = v;
          },
        );
        view.querySelector("#recFilter")?.addEventListener("click", () => {
          view.dataset.q = (view.querySelector("#recQ") as HTMLInputElement).value;
          void renderView();
        });
        view.querySelectorAll(".rec-row").forEach((row) => {
          row.addEventListener("click", () => void openRecordDrawer((row as HTMLElement).dataset.id!));
        });
      } else if (state.route === "governance") {
        const data = await api.governance();
        const domains = (data.domains as Record<string, unknown>[]) ?? [];
        const ro = readOnly() || Boolean(data.readOnly);
        view.innerHTML = `
          ${viewNoteHtml(data.viewNote as string)}
          <div class="page-head"><div><h1 class="page-title">出站治理</h1><p class="page-desc">域级熔断与限速状态。代理：${escapeHtml(String(data.proxy))}</p></div></div>
          <table class="table"><thead><tr><th>主机</th><th>熔断</th><th>操作</th></tr></thead>
          <tbody>${domains
            .map(
              (d) =>
                `<tr><td>${escapeHtml(String(d.host))}</td><td>${escapeHtml(String(d.circuitLabel ?? d.circuit))}</td>
                <td>${d.circuit !== "closed" ? `<button type="button" class="btn btn-sm" data-host="${escapeHtml(String(d.host))}" ${ro ? "disabled" : ""}>重置熔断</button>` : "—"}</td></tr>`,
            )
            .join("")}</tbody></table>`;
        view.querySelectorAll("[data-host]").forEach((btn) => {
          btn.addEventListener("click", async () => {
            if (ro) return;
            const host = (btn as HTMLElement).dataset.host!;
            if (!(await confirm("重置熔断？", host))) return;
            await api.resetCircuit(host);
            toast("已重置", host);
            void renderView();
          });
        });
      } else {
        const ro = readOnly();
        const summary = await api.configSummary();
        const files = await api.configFiles();
        const flat = files.groups?.flatMap((g) => g.items) ?? files.items.map((p) => ({ path: p, label: p }));
        const path = state.configPath ?? flat[0]?.path ?? "presets/standard.yaml";
        const file = await api.configFile(path);
        diskConfigText = file.text;
        state.configPath = path;
        view.innerHTML = `
          ${ro ? `<div class="view-note" role="status">该视图显示实时状态。历史模式下不可编辑配置。</div>` : ""}
          <div class="page-head"><div><h1 class="page-title">配置</h1><p class="page-desc">仅可编辑信源声明与策略预设。保存前会校验，失败则不写入。</p></div></div>
          <div class="stat-grid">
            <div class="stat od-stat"><span class="stat-label">配置版本</span><strong class="stat-value">${summary.generation}</strong><span class="stat-hint">当前生效代际</span></div>
            <div class="stat od-stat"><span class="stat-label">磁盘未加载变更</span><strong class="stat-value">${summary.pendingDiffCount}</strong><span class="stat-hint">相对运行中配置的 diff 条数</span></div>
          </div>
          <div class="config-layout">
            <div class="config-files" id="cfgFileList"></div>
            <div id="cfgEditorHost"></div>
            <div class="btn-row">
              <button type="button" class="btn" id="btnValidateText" ${ro ? "disabled" : ""}>校验草稿</button>
              <button type="button" class="btn btn-primary" id="btnSaveCfg" ${ro ? "disabled" : ""}>保存并加载</button>
              <button type="button" class="btn btn-quiet" id="btnRollback" ${!file.canRollback || ro ? "disabled" : ""}>回滚上一版</button>
            </div>
          </div>`;
        const listHost = view.querySelector("#cfgFileList") as HTMLElement;
        listHost.innerHTML = (files.groups ?? [])
          .map(
            (g) =>
              `<section class="cfg-group"><h3>${escapeHtml(g.label)}</h3><ul>${g.items
                .map(
                  (it) =>
                    `<li><button type="button" class="cfg-file${it.path === path ? " active" : ""}" data-path="${escapeHtml(it.path)}">${escapeHtml(it.label)}</button></li>`,
                )
                .join("")}</ul></section>`,
          )
          .join("");
        listHost.querySelectorAll("[data-path]").forEach((btn) => {
          btn.addEventListener("click", async () => {
            const next = (btn as HTMLElement).dataset.path!;
            if (next === path) return;
            if (state.configDirty && !(await confirm("未保存的修改", "切换文件将丢失当前编辑内容。"))) return;
            state.configPath = next;
            state.configDirty = false;
            void renderView();
          });
        });
        const editorHost = view.querySelector("#cfgEditorHost") as HTMLElement;
        cfgEditor = mountConfigEditor(editorHost, file.text, diskConfigText, (dirty) => {
          state.configDirty = dirty;
        });
        view.querySelector("#btnValidateText")?.addEventListener("click", async () => {
          if (!cfgEditor) return;
          try {
            cfgEditor.setErrorLine(null);
            await api.validateFileText(path, cfgEditor.getValue());
            toast("校验通过", path);
          } catch (err) {
            const msg = err instanceof Error ? err.message : String(err);
            cfgEditor.setErrorLine(parseErrorLine(msg));
            toast("校验失败", msg);
          }
        });
        view.querySelector("#btnSaveCfg")?.addEventListener("click", async () => {
          if (!cfgEditor || ro) return;
          if (!(await confirm("保存并加载？", `${path} 将写入磁盘并替换当前生效配置。`))) return;
          try {
            cfgEditor.setErrorLine(null);
            await api.putConfigFile(path, cfgEditor.getValue());
            state.configDirty = false;
            diskConfigText = cfgEditor.getValue();
            cfgEditor.setDiskVersion(diskConfigText);
            toast("已保存", "配置已校验并加载");
            await refreshMeta();
            void renderView();
          } catch (err) {
            const msg = err instanceof Error ? err.message : String(err);
            cfgEditor.setErrorLine(parseErrorLine(msg));
            toast("保存失败", msg);
          }
        });
        view.querySelector("#btnRollback")?.addEventListener("click", async () => {
          if (ro) return;
          if (!(await confirm("回滚？", "恢复为上一次保存前的内容并重新加载。"))) return;
          await api.rollbackConfigFile(path);
          state.configDirty = false;
          toast("已回滚", path);
          void renderView();
        });
      }
    } catch (e) {
      state.error = e instanceof Error ? e.message : String(e);
      view.innerHTML = `<div class="error" role="alert"><h2>无法加载</h2><p>${state.error}</p></div>`;
    } finally {
      state.loading = false;
    }
  }

  async function refreshMeta() {
    state.meta = await api.meta(state.asOfSnapshot);
    const timeline = await api.timeline();
    foot.innerHTML = `
      <div class="foot-row"><span>配置版本</span><strong>${state.meta.configGeneration}</strong></div>
      <div class="foot-row"><span>查看历史状态</span><span id="asOfHost"></span></div>
      <p class="foot-note">写操作需确认后提交</p>`;
    const host = foot.querySelector("#asOfHost") as HTMLElement;
    mountSelect(
      host,
      [{ value: "", label: "实时" }, ...(timeline.items ?? []).map((t) => ({ value: String(t.snapshot), label: String(t.label ?? t.snapshot) }))],
      state.asOfSnapshot != null ? String(state.asOfSnapshot) : "",
      (v) => setAsOf(v ? Number(v) : null),
    );
  }

  function openPalette() {
    palette.hidden = false;
    paletteInput.value = "";
    paletteInput.focus();
    const results = shell.querySelector("#paletteResults") as HTMLElement;
    const cmds = [
      ...routes.map((r) => ({ label: `前往：${r.label}`, run: () => setRoute(r.id) })),
      { label: "校验配置", run: () => api.validateConfig().then(() => toast("校验", "通过")) },
    ];
    results.innerHTML = cmds.map((c, i) => `<button type="button" data-i="${i}">${c.label}</button>`).join("");
    results.querySelectorAll("button").forEach((b) => {
      b.addEventListener("click", () => {
        cmds[Number((b as HTMLElement).dataset.i)]!.run();
        palette.hidden = true;
      });
    });
  }

  shell.querySelector("#paletteBtn")?.addEventListener("click", openPalette);
  window.addEventListener("keydown", (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
      e.preventDefault();
      openPalette();
    }
    if (e.key === "Escape") palette.hidden = true;
  });
  palette.querySelector(".palette-scrim")?.addEventListener("click", () => {
    palette.hidden = true;
  });

  const themeSeg = shell.querySelector("#themeSeg") as HTMLElement;
  const syncThemeUi = () => {
    const mode = loadTheme();
    themeSeg.querySelectorAll("button").forEach((btn) => {
      btn.setAttribute("aria-pressed", String((btn as HTMLElement).dataset.theme === mode));
    });
  };
  themeSeg.querySelectorAll("button").forEach((btn) => {
    btn.addEventListener("click", () => {
      saveTheme((btn as HTMLElement).dataset.theme as ThemeMode);
      syncThemeUi();
    });
  });
  syncThemeUi();

  const routeFromHash = (): Route => {
    const id = location.hash.replace(/^#/, "");
    return routes.some((r) => r.id === id) ? (id as Route) : "overview";
  };
  window.addEventListener("hashchange", () => setRoute(routeFromHash()));
  setRoute(routeFromHash());
  renderNav();
  void refreshMeta().then(() => renderView());

  return { setRoute, refresh: () => renderView() };
}
