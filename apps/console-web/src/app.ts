import { api } from "./api";

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
};

export function createApp(root: HTMLElement) {
  const state: AppState = { route: "overview", loading: true, error: null, meta: null };

  const shell = document.createElement("div");
  shell.className = "shell";
  shell.innerHTML = `
    <aside class="sidebar" aria-label="主导航">
      <div class="brand"><span class="brand-name">ModalPrism</span><span class="brand-sub">沉积观测台</span></div>
      <nav class="nav" id="nav"></nav>
      <div class="sidebar-foot" id="foot"></div>
    </aside>
    <div class="main">
      <header class="topbar">
        <div class="crumb"><span>观测台</span> / <strong id="crumb"></strong></div>
        <button type="button" class="search-trigger" id="paletteBtn">命令面板 <kbd>Ctrl K</kbd></button>
        <select id="themeSelect" aria-label="主题">
          <option value="light">浅色</option>
          <option value="dark">深色</option>
          <option value="system">跟随系统</option>
        </select>
      </header>
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

  function setRoute(route: Route) {
    state.route = route;
    renderNav();
    void renderView();
  }

  function renderNav() {
    nav.innerHTML = routes
      .map(
        (r) =>
          `<button type="button" class="nav-item${state.route === r.id ? " active" : ""}" data-route="${r.id}">${r.label}</button>`,
      )
      .join("");
    nav.querySelectorAll("[data-route]").forEach((btn) => {
      btn.addEventListener("click", () => setRoute((btn as HTMLElement).dataset.route as Route));
    });
    crumb.textContent = routes.find((r) => r.id === state.route)?.label ?? "";
  }

  async function renderView() {
    state.loading = true;
    state.error = null;
    view.innerHTML = `<p class="loading">加载中…</p>`;
    try {
      if (state.route === "overview") {
        const data = await api.overview();
        const sources = (data.sources as Record<string, unknown>[]) ?? [];
        view.innerHTML = `
          <h1 class="page-title">总览</h1>
          <p class="page-desc">采集进程与信源健康摘要。数据来自当前 Strata 可见快照。</p>
          <div class="stat-grid">
            <div class="stat"><span class="stat-label">信源</span><strong>${data.sourceCount}</strong></div>
            <div class="stat"><span class="stat-label">健康</span><strong>${data.healthyCount}</strong></div>
            <div class="stat"><span class="stat-label">配置代际</span><strong>${state.meta?.configGeneration ?? "—"}</strong></div>
          </div>
          <h2>信源状态</h2>
          <table class="table"><thead><tr><th>名称</th><th>健康</th><th>下次</th><th>跳过原因</th></tr></thead>
          <tbody>${sources
            .slice(0, 12)
            .map(
              (s) =>
                `<tr><td>${s.name}</td><td>${s.healthStatus}</td><td>${s.nextAt ?? "—"}</td><td>${s.skipReason ?? "—"}</td></tr>`,
            )
            .join("")}</tbody></table>`;
      } else if (state.route === "sources") {
        const { items } = await api.sources();
        view.innerHTML = `
          <h1 class="page-title">信源</h1>
          <p class="page-desc">已声明的采集信源。启用状态与调度来自 YAML 配置。</p>
          <table class="table"><thead><tr><th>信源</th><th>访问器</th><th>健康</th><th>操作</th></tr></thead>
          <tbody>${items
            .map(
              (s) => `<tr data-id="${s.id}">
                <td>${s.name}<div class="muted mono">${s.id}</div></td>
                <td>${s.accessor}</td>
                <td>${s.healthStatus}${s.skipReason ? ` · ${s.skipReason}` : ""}</td>
                <td>
                  <button type="button" class="btn btn-sm" data-act="trigger">立即采集</button>
                  <button type="button" class="btn btn-sm btn-quiet" data-act="toggle">${s.enabled ? "停用" : "启用"}</button>
                </td></tr>`,
            )
            .join("")}</tbody></table>`;
        view.querySelectorAll("[data-act]").forEach((btn) => {
          const row = btn.closest("tr") as HTMLElement;
          const id = row.dataset.id!;
          btn.addEventListener("click", async () => {
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
        const { items } = await api.records(50);
        view.innerHTML = `
          <h1 class="page-title">数据记录</h1>
          <p class="page-desc">Strata 中的资源条目。进阶调试可在运维 API 查看内部 facet。</p>
          ${
            items.length === 0
              ? `<div class="empty">尚无记录。启动采集并等待 committer flush 后再查看。</div>`
              : `<table class="table"><thead><tr><th>记录</th><th>信源</th><th>更新</th></tr></thead>
          <tbody>${items
            .map(
              (r) =>
                `<tr><td class="mono">${r.id}</td><td>${r.sourceId ?? "—"}</td><td>${r.updatedAt ?? "—"}</td></tr>`,
            )
            .join("")}</tbody></table>`
          }`;
      } else if (state.route === "governance") {
        const data = await api.governance();
        const domains = (data.domains as Record<string, unknown>[]) ?? [];
        view.innerHTML = `
          <h1 class="page-title">出站治理</h1>
          <p class="page-desc">域级熔断与限速状态。代理：${data.proxy}</p>
          <table class="table"><thead><tr><th>主机</th><th>熔断</th><th>操作</th></tr></thead>
          <tbody>${domains
            .map(
              (d) =>
                `<tr><td>${d.host}</td><td>${d.circuit}</td>
                <td>${d.circuit !== "closed" ? `<button type="button" class="btn btn-sm" data-host="${d.host}">重置熔断</button>` : "—"}</td></tr>`,
            )
            .join("")}</tbody></table>`;
        view.querySelectorAll("[data-host]").forEach((btn) => {
          btn.addEventListener("click", async () => {
            const host = (btn as HTMLElement).dataset.host!;
            if (!(await confirm("重置熔断？", host))) return;
            await api.resetCircuit(host);
            toast("已重置", host);
            void renderView();
          });
        });
      } else {
        const summary = await api.configSummary();
        view.innerHTML = `
          <h1 class="page-title">配置</h1>
          <p class="page-desc">配置以仓库内 YAML 为准。修改文件后需校验并重新加载。</p>
          <div class="stat-grid">
            <div class="stat"><span class="stat-label">代际</span><strong>${summary.generation}</strong></div>
            <div class="stat"><span class="stat-label">与磁盘差异项</span><strong>${summary.pendingDiffCount}</strong></div>
          </div>
          <div class="btn-row">
            <button type="button" class="btn btn-primary" id="btnValidate">校验配置</button>
            <button type="button" class="btn" id="btnReload">重新加载</button>
          </div>`;
        view.querySelector("#btnValidate")?.addEventListener("click", async () => {
          await api.validateConfig();
          toast("校验通过", "配置目录有效");
        });
        view.querySelector("#btnReload")?.addEventListener("click", async () => {
          if (!(await confirm("重新加载配置？", "将替换当前生效快照"))) return;
          await api.reloadConfig();
          toast("已加载", "新代际已生效");
          await refreshMeta();
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
    state.meta = await api.meta();
    foot.innerHTML = `<div>配置代际 <strong>${state.meta.configGeneration}</strong></div>
      <div>快照 <strong>${state.meta.latestSnapshotSeq ?? "—"}</strong></div>
      <div class="mono">${state.meta.httpListen}</div>`;
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

  const themeSelect = shell.querySelector("#themeSelect") as HTMLSelectElement;
  themeSelect.value = localStorage.getItem("modalprism-console-theme") ?? "light";
  themeSelect.addEventListener("change", () => {
    const mode = themeSelect.value as "light" | "dark" | "system";
    localStorage.setItem("modalprism-console-theme", mode);
    document.documentElement.dataset.theme = mode;
    const dark =
      mode === "dark" ||
      (mode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
    document.documentElement.classList.toggle("theme-dark", dark);
    document.documentElement.classList.toggle("theme-light", !dark);
  });

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
