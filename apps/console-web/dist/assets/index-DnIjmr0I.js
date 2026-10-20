(function(){const e=document.createElement("link").relList;if(e&&e.supports&&e.supports("modulepreload"))return;for(const r of document.querySelectorAll('link[rel="modulepreload"]'))w(r);new MutationObserver(r=>{for(const n of r)if(n.type==="childList")for(const x of n.addedNodes)x.tagName==="LINK"&&x.rel==="modulepreload"&&w(x)}).observe(document,{childList:!0,subtree:!0});function v(r){const n={};return r.integrity&&(n.integrity=r.integrity),r.referrerPolicy&&(n.referrerPolicy=r.referrerPolicy),r.crossOrigin==="use-credentials"?n.credentials="include":r.crossOrigin==="anonymous"?n.credentials="omit":n.credentials="same-origin",n}function w(r){if(r.ep)return;r.ep=!0;const n=v(r);fetch(r.href,n)}})();const Se="x-modalprism-console";function we(){return{Accept:"application/json"}}function $e(){return{Accept:"application/json","Content-Type":"application/json",[Se]:"1"}}function U(t){if(!t)return"—";const e=new Date(t);return Number.isNaN(e.getTime())?t:e.toLocaleString(void 0,{year:"numeric",month:"2-digit",day:"2-digit",hour:"2-digit",minute:"2-digit"})}function ce(t){return t!=null?`?asOfSnapshot=${t}`:""}async function q(t,e){var r;const v=await fetch(`/console/api/v1${t}`,{...e,headers:{...e!=null&&e.method&&e.method!=="GET"?$e():we(),...(e==null?void 0:e.headers)??{}}}),w=await v.json().catch(()=>({}));if(!v.ok){const n=w;throw new Error(((r=n.error)==null?void 0:r.message)??v.statusText)}return w}const T={meta:t=>q(`/meta${ce(t)}`),timeline:()=>q("/timeline"),overview:()=>q("/overview"),sources:()=>q("/sources"),source:t=>q(`/sources/${encodeURIComponent(t.replace(/^source\//,""))}`),trigger:t=>q(`/sources/${encodeURIComponent(t)}/trigger`,{method:"POST",body:JSON.stringify({confirm:!0})}),patchSource:(t,e)=>q(`/sources/${encodeURIComponent(t)}`,{method:"PATCH",body:JSON.stringify({enabled:e,confirm:!0})}),records:t=>{const e=new URLSearchParams;t.limit&&e.set("limit",String(t.limit)),t.cursor&&e.set("cursor",t.cursor),t.source&&e.set("source",t.source),t.q&&e.set("q",t.q),t.asOf!=null&&e.set("asOfSnapshot",String(t.asOf));const v=e.toString();return q(`/records${v?`?${v}`:""}`)},record:(t,e)=>q(`/records/${encodeURIComponent(t)}${ce(e)}`),governance:()=>q("/governance/egress"),resetCircuit:t=>q(`/governance/egress/${encodeURIComponent(t)}/reset-circuit`,{method:"POST",body:JSON.stringify({confirm:!0})}),configSummary:()=>q("/config/summary"),configFiles:()=>q("/config/files"),configFile:t=>q(`/config/files/${encodeURIComponent(t)}`),validateFileText:(t,e)=>q("/config/file-validate",{method:"POST",body:JSON.stringify({path:t,text:e})}),putConfigFile:(t,e)=>q(`/config/files/${encodeURIComponent(t)}`,{method:"PUT",body:JSON.stringify({text:e,confirm:!0})}),rollbackConfigFile:t=>q("/config/file-rollback",{method:"POST",body:JSON.stringify({path:t,confirm:!0})}),validateConfig:()=>q("/config/validate",{method:"POST",body:"{}"}),reloadConfig:()=>q("/config/reload",{method:"POST",body:"{}"})},ue="modalprism-console-theme";function pe(){const t=localStorage.getItem(ue);return t==="light"||t==="dark"||t==="system"?t:"light"}function Le(t){localStorage.setItem(ue,t),X(t)}function X(t){const e=document.documentElement;e.dataset.theme=t;const v=t==="dark"||t==="system"&&window.matchMedia("(prefers-color-scheme: dark)").matches;e.classList.toggle("theme-dark",v),e.classList.toggle("theme-light",!v)}function ke(t,e,v,w){t.innerHTML=`
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
  `;const r=t.querySelector(".cfg-textarea"),n=t.querySelector("#cfgGutter"),x=t.querySelector("#cfgDiffView"),p=t.querySelector("#cfgDiffHint"),Y=t.querySelector("#cfgToggleDiff");let j=v,F=null,m=!1;r.value=e;const D=()=>{const L=r.value.split(`
`);n.innerHTML=L.map((P,H)=>{const R=H+1;return`<span class="cfg-ln${F===R?" cfg-line-error":""}">${R}</span>`}).join("")},N=()=>{const L=j.split(`
`),P=r.value.split(`
`),H=Math.max(L.length,P.length),R=[];for(let O=0;O<H;O++){const I=L[O]??"",M=P[O]??"";I===M?R.push(`<span class="diff-eq"> ${G(M)}</span>`):I&&!M?R.push(`<span class="diff-del">-${G(I)}</span>`):!I&&M?R.push(`<span class="diff-add">+${G(M)}</span>`):(R.push(`<span class="diff-del">-${G(I)}</span>`),R.push(`<span class="diff-add">+${G(M)}</span>`))}x.innerHTML=R.join(`
`)},b=()=>{D();const L=r.value!==j;p.textContent=L?"与磁盘版本有差异（未保存）":"与磁盘版本一致",m&&N(),w(L)};return r.addEventListener("input",b),r.addEventListener("scroll",()=>{n.scrollTop=r.scrollTop}),Y.addEventListener("click",()=>{m=!m,r.hidden=m,x.hidden=!m,Y.textContent=m?"返回编辑":"查看与磁盘差异",m&&N()}),b(),{getValue:()=>r.value,setValue(L){r.value=L,b()},setDiskVersion(L){j=L,b()},setErrorLine(L){if(F=L,D(),L!=null){const P=L-1,H=r.value.split(`
`);P>=0&&P<H.length&&(r.focus(),r.setSelectionRange(H.slice(0,P).join(`
`).length+(P>0?1:0),H.slice(0,P+1).join(`
`).length))}},focus:()=>r.focus()}}function G(t){return t.replace(/&/g,"&amp;").replace(/</g,"&lt;")}function de(t,e,v,w,r="请选择"){const n=e.find(m=>m.value===v);t.className="custom-select",t.innerHTML=`
    <button type="button" class="custom-select-trigger" aria-haspopup="listbox" aria-expanded="false">
      <span class="custom-select-label">${(n==null?void 0:n.label)??r}</span>
      <span class="custom-select-chevron" aria-hidden="true"></span>
    </button>
    <ul class="custom-select-menu" role="listbox" hidden></ul>
  `;const x=t.querySelector(".custom-select-menu"),p=t.querySelector(".custom-select-trigger"),Y=t.querySelector(".custom-select-label"),j=()=>{x.innerHTML=e.map(m=>`<li role="option" data-value="${qe(m.value)}" aria-selected="${m.value===v}">${fe(m.label)}</li>`).join("")};j();const F=()=>{x.hidden=!0,p.setAttribute("aria-expanded","false")};return p.addEventListener("click",m=>{m.stopPropagation();const D=x.hidden;document.querySelectorAll(".custom-select-menu").forEach(N=>{N.hidden=!0}),x.hidden=!D,p.setAttribute("aria-expanded",String(D))}),x.addEventListener("click",m=>{var b;const D=m.target.closest("[data-value]");if(!D)return;const N=D.dataset.value??"";v=N,Y.textContent=((b=e.find(L=>L.value===N))==null?void 0:b.label)??r,j(),F(),w(N)}),document.addEventListener("click",F),{setValue(m){var D;v=m,Y.textContent=((D=e.find(N=>N.value===m))==null?void 0:D.label)??r,j()},setOptions(m){e=m,j()}}}function fe(t){return t.replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/"/g,"&quot;")}function qe(t){return fe(t)}const K=[{id:"overview",label:"总览"},{id:"sources",label:"信源"},{id:"records",label:"数据记录"},{id:"governance",label:"治理"},{id:"config",label:"配置"}];function Ee(t){var ie,oe,le;const e={route:"overview",loading:!0,error:null,meta:null,asOfSnapshot:null,configDirty:!1,configPath:null};let v="",w=null;const r=()=>e.asOfSnapshot!=null,n=document.createElement("div");n.className="shell",n.innerHTML=`
    <aside class="sidebar" aria-label="主导航">
      <div class="brand">
        <svg class="brand-mark" viewBox="0 0 32 32" aria-hidden="true">
          <path d="M16 3.5 28 26H4L16 3.5Z" fill="none" stroke="currentColor" stroke-width="1.2"/>
          <path d="M16 3.5v22.5M10 14.8h12M7 21h18" fill="none" stroke="currentColor" stroke-width=".8" opacity=".55"/>
          <path d="M16 3.5 22 26" stroke="#E2552F" stroke-width="1.2" fill="none"/>
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
        <div class="crumb"><span class="crumb-root">观测台</span><span class="crumb-sep" aria-hidden="true">/</span><span id="crumb"></span></div>
        <button type="button" class="search-trigger" id="paletteBtn" aria-haspopup="dialog">
          <span class="search-text">搜索资源、信源、提交…</span>
          <kbd>Ctrl K</kbd>
        </button>
        <div class="topbar-meta">
          <span class="live-state" id="liveState"><span class="live-dot" aria-hidden="true"></span><span id="liveLabel">实时</span></span>
          <time class="clock" id="clock"></time>
          <button type="button" class="gen-chip" id="genChip" title="查看配置">—</button>
        </div>
        <div class="segmented theme-seg" id="themeSeg" role="group" aria-label="主题">
          <button type="button" data-theme="light">浅色</button>
          <button type="button" data-theme="dark">深色</button>
          <button type="button" data-theme="system">系统</button>
        </div>
      </header>
      <div id="historyBanner" class="history-banner" hidden role="status"></div>
      <main id="viewRoot" class="view" tabindex="-1"></main>
    </div>
    <footer class="snapshot-bar" id="snapshotBar" aria-label="快照游标">
      <div class="snap-left">
        <span class="snap-label">SNAPSHOT</span>
        <strong class="snap-id" id="snapId">—</strong>
        <span class="snap-time" id="snapTime"></span>
        <span class="snap-mode" id="snapMode">实时</span>
      </div>
      <div class="snap-right">
        <button type="button" class="btn btn-quiet btn-sm" id="snapLive" hidden>回到实时</button>
        <span class="snap-hint mono">读取按快照进行</span>
      </div>
    </footer>
    <div class="drawer-layer" id="drawerLayer" hidden>
      <div class="drawer-scrim" id="drawerScrim"></div>
      <section class="drawer" id="drawer" role="dialog" aria-modal="true" aria-labelledby="drawerTitle" tabindex="-1"></section>
    </div>
    <div class="palette-layer" id="palette" hidden>
      <div class="palette-scrim"></div>
      <section class="palette" role="dialog" aria-modal="true" aria-label="命令面板">
        <div class="palette-input-row">
          <input id="paletteInput" class="palette-input" type="search" placeholder="输入命令、信源或 RID…" autocomplete="off" spellcheck="false" />
          <kbd>ESC</kbd>
        </div>
        <div id="paletteResults" class="palette-results" role="listbox"></div>
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
  `,t.appendChild(n);const x=n.querySelector("#nav"),p=n.querySelector("#viewRoot"),Y=n.querySelector("#crumb"),j=n.querySelector("#foot"),F=n.querySelector("#palette"),m=n.querySelector("#paletteInput"),D=n.querySelector("#confirm"),N=n.querySelector("#drawerLayer"),b=n.querySelector("#drawer"),L=n.querySelector("#clock"),P=()=>{const a=new Date;L.textContent=a.toLocaleTimeString("zh-CN",{hour12:!1})};P(),setInterval(P,1e3);const H=()=>{N.hidden=!0,b.innerHTML=""};(ie=n.querySelector("#drawerScrim"))==null||ie.addEventListener("click",H);function R(a,l,d=""){return`<div class="table-scroll"><table class="data-table ${d}"><thead><tr>${a}</tr></thead><tbody>${l}</tbody></table></div>`}function O(a,l){const d=document.createElement("div");d.className="toast",d.innerHTML=`<strong>${a}</strong><p>${l}</p>`,n.querySelector("#toasts").appendChild(d),setTimeout(()=>d.remove(),4e3)}function I(a,l){return new Promise(d=>{n.querySelector("#confirmTitle").textContent=a,n.querySelector("#confirmBody").textContent=l,D.hidden=!1;const $=n.querySelector("#confirmOk"),o=n.querySelector("#confirmCancel"),f=h=>{D.hidden=!0,$.onclick=null,o.onclick=null,d(h)};$.onclick=()=>f(!0),o.onclick=()=>f(!1)})}async function M(a){if(e.configDirty&&e.route==="config"&&a!=="config"){if(!await I("未保存的修改","离开配置页将丢失未保存的 YAML 修改。"))return;e.configDirty=!1}e.route=a,location.hash.replace(/^#/,"")!==a&&(location.hash=a),ee(),B()}function Q(a){var d;e.asOfSnapshot=a;const l=n.querySelector("#historyBanner");a==null?l.hidden=!0:(l.hidden=!1,l.innerHTML=`<div class="hb-copy"><strong>历史状态 #${a}</strong><span class="hb-note">数据记录按所选时间点显示；其他视图为实时数据且不可写入。</span></div><button type="button" class="btn btn-sm btn-quiet" id="liveBtn">回到实时</button>`,(d=l.querySelector("#liveBtn"))==null||d.addEventListener("click",()=>Q(null))),z().then(()=>B())}window.addEventListener("beforeunload",a=>{e.configDirty&&a.preventDefault()});function ee(){var $;const a=K.filter(o=>o.id==="overview"||o.id==="sources"||o.id==="records"),l=K.filter(o=>o.id==="governance"||o.id==="config"),d=o=>`<button type="button" class="nav-item" data-route="${o.id}" aria-current="${e.route===o.id?"page":"false"}"><span class="nav-icon" data-icon="${o.id}" aria-hidden="true"></span><span>${o.label}</span></button>`;x.innerHTML=`
      <div class="nav-group"><span class="nav-label">观测 · OBSERVE</span>${a.map(d).join("")}</div>
      <div class="nav-group"><span class="nav-label">系统 · SYSTEM</span>${l.map(d).join("")}</div>`,x.querySelectorAll("[data-route]").forEach(o=>{o.addEventListener("click",()=>M(o.dataset.route))}),Y.textContent=(($=K.find(o=>o.id===e.route))==null?void 0:$.label)??""}function _(a){return!a||e.route==="records"?"":`<div class="view-note" role="status">${a}</div>`}function te(a){const l=a.match(/第\s*(\d+)\s*行/);return l?Number(l[1]):null}async function he(a){var d,$,o;N.hidden=!1,b.focus();const l=a.replace(/^source\//,"");b.innerHTML=`
      <header class="drawer-head">
        <h2 class="drawer-title" id="drawerTitle">加载中…</h2>
        <button type="button" class="icon-btn drawer-close" id="closeDrawer" aria-label="关闭">×</button>
      </header>
      <div class="drawer-body"><p class="loading">加载中…</p></div>`,(d=b.querySelector("#closeDrawer"))==null||d.addEventListener("click",H);try{const f=await T.source(l),h=f.requirements??[];b.innerHTML=`
        <header class="drawer-head">
          <h2 class="drawer-title" id="drawerTitle">${i(String(f.name??a))}</h2>
          <button type="button" class="icon-btn drawer-close" id="closeDrawer" aria-label="关闭">×</button>
        </header>
        <div class="drawer-body">
          <p class="muted">${i(String(f.accessorLabel??f.accessor??""))}</p>
          <dl class="detail-dl">
            <dt>状态</dt><dd><span class="status-chip ${i(String(f.statusChipClass??"status-muted"))}">${i(String(f.skipReasonLabel??f.healthLabel??f.healthStatus??""))}</span></dd>
            <dt>下次采集</dt><dd>${f.nextAt?U(f.nextAt):"—"}</dd>
            <dt>最近成功</dt><dd>${f.lastYieldAt?U(f.lastYieldAt):"—"}</dd>
            <dt>今日条数</dt><dd>${f.todayYieldCount??0}</dd>
          </dl>
          ${h.length?`<ul class="req-list">${h.map(C=>`<li><strong>${i(String(C.label))}</strong><span>${i(String(C.detail))}</span></li>`).join("")}</ul>`:""}
          <h3 class="section-title">最近记录</h3>
          <ul>${(f.recentRecords??[]).map(C=>`<li class="mono od-truncate">${i(String(C.title??C.id))}</li>`).join("")}</ul>
        </div>`,($=b.querySelector("#closeDrawer"))==null||$.addEventListener("click",H)}catch(f){const h=f instanceof Error?f.message:String(f);b.innerHTML=`
        <header class="drawer-head"><h2 class="drawer-title">无法加载信源</h2>
        <button type="button" class="icon-btn drawer-close" id="closeDrawer" aria-label="关闭">×</button></header>
        <div class="drawer-body error"><p>${i(h)}</p></div>`,(o=b.querySelector("#closeDrawer"))==null||o.addEventListener("click",H)}}async function ge(a){N.hidden=!1,b.focus();const l=async()=>{var d,$,o,f;b.innerHTML=`
        <header class="drawer-head">
          <h2 class="drawer-title" id="drawerTitle">加载中…</h2>
          <button type="button" class="icon-btn drawer-close" id="closeDrawer" aria-label="关闭">×</button>
        </header>
        <div class="drawer-body"><p class="loading">加载中…</p></div>`,(d=b.querySelector("#closeDrawer"))==null||d.addEventListener("click",H);try{const h=await T.record(a,e.asOfSnapshot),C=h.title||"未命名";b.innerHTML=`
          <header class="drawer-head">
            <h2 class="drawer-title">${i(C)}</h2>
            <button type="button" class="icon-btn drawer-close" id="closeDrawer" aria-label="关闭">×</button>
          </header>
          <div class="drawer-body">
            <p class="muted">${i(String(h.sourceName??h.sourceId??""))}</p>
            <dl class="detail-dl">
              <dt>发布时间</dt><dd>${U(h.publishedAt)}</dd>
              <dt>采集时间</dt><dd>${U(h.collectedAt??h.updatedAt)}</dd>
              <dt>原文</dt><dd>${h.url?`<a href="${h.url}" target="_blank" rel="noopener">${i(String(h.url))}</a>`:"—"}</dd>
              <dt>摘要</dt><dd>${i(String(h.summary??"—"))}</dd>
            </dl>
            <pre class="body-preview">${i(String(h.bodyPreview??""))}</pre>
            <details class="tech-details"><summary>技术详情</summary><pre>${i(JSON.stringify(h.technical,null,2))}</pre></details>
          </div>`,($=b.querySelector("#closeDrawer"))==null||$.addEventListener("click",H)}catch(h){const C=h instanceof Error?h.message:String(h);b.innerHTML=`
          <header class="drawer-head">
            <h2 class="drawer-title">无法加载记录</h2>
            <button type="button" class="icon-btn drawer-close" id="closeDrawer" aria-label="关闭">×</button>
          </header>
          <div class="drawer-body error"><p>${i(C)}</p>
            <button type="button" class="btn btn-primary" id="retryDrawer">重试</button></div>`,(o=b.querySelector("#closeDrawer"))==null||o.addEventListener("click",H),(f=b.querySelector("#retryDrawer"))==null||f.addEventListener("click",()=>void l())}};await l()}function i(a){return a.replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/"/g,"&quot;")}async function B(){var a,l,d,$,o,f,h,C,J;e.loading=!0,e.error=null,p.innerHTML='<p class="loading">加载中…</p>';try{if(e.route==="overview"){const u=await T.overview(),A=u.sources??[],S=u.viewNote,c=((a=e.meta)==null?void 0:a.configGeneration)??"—",y=((l=e.meta)==null?void 0:l.httpListen)??"",E=u.queueCapacities??{},g=u.recentUpdates??[],V=(s,k,W)=>`<article class="metric"><span class="metric-label">${s}</span><strong class="metric-value">${k}</strong><span class="metric-hint">${W}</span></article>`;p.innerHTML=`
          ${_(S)}
          <header class="page-head">
            <div>
              <div class="page-kicker"><span class="kicker-bar" aria-hidden="true"><i></i><i></i><i></i></span>L1 · OBSERVATORY</div>
              <h1 class="page-title">系统正在沉积</h1>
              <p class="page-desc">配置代际 <span class="mono">${c}</span>${y?` · 环回入口 <span class="mono">${i(y)}</span>`:""}。每一次读取都有快照，每一次写入都是提交。</p>
            </div>
          </header>
          <section class="metrics-band" aria-label="核心指标">
            ${V("信源",String(u.sourceCount??0),"已声明")}
            ${V("健康信源",String(u.healthyCount??0),`需关注 ${u.attentionCount??0} · 停用 ${u.disabledCount??0}`)}
            ${V("队列容量",`F${E.free??0}`,`付费 ${E.paid??0} · 渲染 ${E.render??0}`)}
            ${V("配置代际",String(c),"当前生效")}
          </section>
          <section class="block grid-full" aria-label="信源健康">
            <div class="block-head"><div class="block-title"><span class="block-kicker">HEALTH</span>信源健康</div>
              <button type="button" class="ghost-link" data-route="sources">全部 ${A.length} 个信源</button></div>
            ${R("<th>信源</th><th>健康</th><th>节奏</th><th>最近成功</th><th>今日条数</th><th>说明</th>",A.slice(0,12).map(s=>{const k=String(s.statusChipClass??"status-muted"),W=s.skipReasonLabel??s.healthLabel??s.healthStatus,me=s.nextAt?U(s.nextAt):"—",be=s.lastYieldAt?U(s.lastYieldAt):"—",ve=s.todayYieldCount??0,ye=s.enabled===!1?"已停用":s.scheduleNote??s.lastError??"—";return`<tr class="is-clickable" data-source="${i(String(s.id))}"><td class="cell-lead"><div class="cell-main"><strong>${i(String(s.name))}</strong><span class="muted mono od-truncate">${i(String(s.accessorLabel??s.accessor??""))}</span></div></td><td><span class="status-chip ${k}">${i(String(W))}</span></td><td class="mono">${me}</td><td class="mono">${be}</td><td class="num">${ve}</td><td class="muted">${i(String(ye))}</td></tr>`}).join(""),"source-table")}
          </section>
          <section class="block grid-full" aria-label="最近提交">
            <div class="block-head"><div class="block-title"><span class="block-kicker">COMMITS</span>最近提交</div></div>
            <ul class="commit-list">${g.slice(0,8).map(s=>`<li><button type="button" class="commit-row" data-snap="${s.snapshot}"><span class="commit-title">${i(String(s.summary??s.snapshot))}</span><span class="commit-meta mono">${U(s.publishedAt)} · #${s.snapshot}</span></button></li>`).join("")}</ul>
          </section>`,(d=p.querySelector('[data-route="sources"]'))==null||d.addEventListener("click",()=>M("sources")),p.querySelectorAll("[data-source]").forEach(s=>{s.addEventListener("click",()=>M("sources"))}),p.querySelectorAll("[data-snap]").forEach(s=>{s.addEventListener("click",()=>{const k=Number(s.dataset.snap);Number.isNaN(k)||Q(k)})})}else if(e.route==="sources"){const u=await T.sources(),A=u.items,S=r()||!!u.readOnly;p.innerHTML=`
          ${_(u.viewNote)}
          <div class="page-head"><div><h1 class="page-title">信源</h1><p class="page-desc">已声明的采集信源。启用状态与调度来自 YAML 配置。</p></div></div>
          ${R("<th>信源</th><th>访问器</th><th>状态</th><th>操作</th>",A.map(c=>{const y=String(c.statusChipClass??"status-muted"),E=c.skipReasonLabel??c.healthLabel??c.healthStatus,g=c.accessorLabel||String(c.accessor);return`<tr data-id="${c.id}">
                <td class="cell-lead"><button type="button" class="ghost-link cell-main" data-act="detail"><strong>${i(String(c.name))}</strong></button></td>
                <td>${i(g)}</td>
                <td><span class="status-chip ${y}">${i(String(E))}</span><div class="muted">${i(String(c.scheduleNote??""))}</div></td>
                <td>
                  <button type="button" class="btn btn-sm" data-act="trigger" ${S?"disabled":""}>立即采集</button>
                  <button type="button" class="btn btn-sm btn-quiet" data-act="toggle" ${S?"disabled":""}>${c.enabled?"停用":"启用"}</button>
                </td></tr>`}).join(""),"source-table")}`,p.querySelectorAll("[data-act='detail']").forEach(c=>{const E=c.closest("tr").dataset.id;c.addEventListener("click",g=>{g.stopPropagation(),he(E)})}),p.querySelectorAll("[data-act]").forEach(c=>{const E=c.closest("tr").dataset.id;c.addEventListener("click",async()=>{if(S)return;const g=c.dataset.act;if(g==="trigger")await T.trigger(E.replace("source/","")),O("已触发",E);else if(g==="toggle"){const V=c.textContent==="启用";if(!await I("修改信源启用状态？",`${E} → enabled=${V}`))return;await T.patchSource(E.replace("source/",""),V),O("已更新","配置已校验并重新加载"),B()}})})}else if(e.route==="records"){const u=p.dataset.sourceFilter||"",A=p.dataset.q||"",{items:S,nextCursor:c,sourceOptions:y}=await T.records({limit:50,source:u||void 0,q:A||void 0,asOf:e.asOfSnapshot});p.innerHTML=`
          <div class="page-head"><div><h1 class="page-title">数据记录</h1><p class="page-desc">已沉积的采集条目。${e.asOfSnapshot!=null?"当前按所选历史状态显示。":""}</p></div></div>
          <div class="filter-row">
            <input id="recQ" class="input" placeholder="按标题搜索…" value="${i(A)}" />
            <div id="recSourceSelect"></div>
            <button type="button" class="btn" id="recFilter">筛选</button>
          </div>
          ${S.length===0?'<div class="empty">尚无记录。完成一轮采集并等待数据发布后刷新。</div>':`${R("<th>标题</th><th>信源</th><th>发布时间</th><th>采集时间</th>",S.map(g=>{const V=g.title||String(g.id),s=g.sourceName||g.sourceId||"—";return`<tr class="rec-row is-clickable" data-id="${g.id}"><td class="cell-lead"><div class="cell-main od-clamp-2">${i(V)}</div></td><td>${i(s)}</td><td class="mono">${U(g.publishedAt)}</td><td class="mono">${U(g.collectedAt??g.updatedAt)}</td></tr>`}).join(""))}
          ${c?'<button type="button" class="btn" id="moreRec">加载更多</button>':""}`}`;const E=p.querySelector("#recSourceSelect");de(E,[{value:"",label:"全部信源"},...(y??[]).map(g=>({value:g.id,label:g.name}))],u,g=>{p.dataset.sourceFilter=g}),($=p.querySelector("#recFilter"))==null||$.addEventListener("click",()=>{p.dataset.q=p.querySelector("#recQ").value,B()}),p.querySelectorAll(".rec-row").forEach(g=>{g.addEventListener("click",()=>void ge(g.dataset.id))})}else if(e.route==="governance"){const u=await T.governance(),A=u.domains??[],S=r()||!!u.readOnly;p.innerHTML=`
          ${_(u.viewNote)}
          <div class="page-head"><div><h1 class="page-title">出站治理</h1><p class="page-desc">域级熔断与限速状态。代理：${i(String(u.proxy))}</p></div></div>
          ${R("<th>主机</th><th>熔断</th><th>操作</th>",A.map(c=>`<tr><td class="mono">${i(String(c.host))}</td><td>${i(String(c.circuitLabel??c.circuit))}</td>
                <td>${c.circuit!=="closed"?`<button type="button" class="btn btn-sm" data-host="${i(String(c.host))}" ${S?"disabled":""}>重置熔断</button>`:"—"}</td></tr>`).join(""))}`,p.querySelectorAll("[data-host]").forEach(c=>{c.addEventListener("click",async()=>{if(S)return;const y=c.dataset.host;await I("重置熔断？",y)&&(await T.resetCircuit(y),O("已重置",y),B())})})}else{const u=r(),A=await T.configSummary(),S=await T.configFiles(),c=((o=S.groups)==null?void 0:o.flatMap(s=>s.items))??S.items.map(s=>({path:s,label:s})),y=e.configPath??((f=c[0])==null?void 0:f.path)??"presets/standard.yaml",E=await T.configFile(y);v=E.text,e.configPath=y,p.innerHTML=`
          ${u?'<div class="view-note" role="status">该视图显示实时状态。历史模式下不可编辑配置。</div>':""}
          <div class="page-head"><div><h1 class="page-title">配置</h1><p class="page-desc">仅可编辑信源声明与策略预设。保存前会校验，失败则不写入。</p></div></div>
          <div class="stat-grid">
            <div class="stat od-stat"><span class="stat-label">配置版本</span><strong class="stat-value">${A.generation}</strong><span class="stat-hint">当前生效代际</span></div>
            <div class="stat od-stat"><span class="stat-label">磁盘未加载变更</span><strong class="stat-value">${A.pendingDiffCount}</strong><span class="stat-hint">相对运行中配置的 diff 条数</span></div>
          </div>
          <div class="config-layout">
            <div class="config-files" id="cfgFileList"></div>
            <div id="cfgEditorHost"></div>
            <div class="btn-row">
              <button type="button" class="btn" id="btnValidateText" ${u?"disabled":""}>校验草稿</button>
              <button type="button" class="btn btn-primary" id="btnSaveCfg" ${u?"disabled":""}>保存并加载</button>
              <button type="button" class="btn btn-quiet" id="btnRollback" ${!E.canRollback||u?"disabled":""}>回滚上一版</button>
            </div>
          </div>`;const g=p.querySelector("#cfgFileList");g.innerHTML=(S.groups??[]).map(s=>`<section class="cfg-group"><h3>${i(s.label)}</h3><ul>${s.items.map(k=>`<li><button type="button" class="cfg-file${k.path===y?" active":""}" data-path="${i(k.path)}">${i(k.label)}</button></li>`).join("")}</ul></section>`).join(""),g.querySelectorAll("[data-path]").forEach(s=>{s.addEventListener("click",async()=>{const k=s.dataset.path;k!==y&&(e.configDirty&&!await I("未保存的修改","切换文件将丢失当前编辑内容。")||(e.configPath=k,e.configDirty=!1,B()))})});const V=p.querySelector("#cfgEditorHost");w=ke(V,E.text,v,s=>{e.configDirty=s}),(h=p.querySelector("#btnValidateText"))==null||h.addEventListener("click",async()=>{if(w)try{w.setErrorLine(null),await T.validateFileText(y,w.getValue()),O("校验通过",y)}catch(s){const k=s instanceof Error?s.message:String(s);w.setErrorLine(te(k)),O("校验失败",k)}}),(C=p.querySelector("#btnSaveCfg"))==null||C.addEventListener("click",async()=>{if(!(!w||u)&&await I("保存并加载？",`${y} 将写入磁盘并替换当前生效配置。`))try{w.setErrorLine(null),await T.putConfigFile(y,w.getValue()),e.configDirty=!1,v=w.getValue(),w.setDiskVersion(v),O("已保存","配置已校验并加载"),await z(),B()}catch(s){const k=s instanceof Error?s.message:String(s);w.setErrorLine(te(k)),O("保存失败",k)}}),(J=p.querySelector("#btnRollback"))==null||J.addEventListener("click",async()=>{u||await I("回滚？","恢复为上一次保存前的内容并重新加载。")&&(await T.rollbackConfigFile(y),e.configDirty=!1,O("已回滚",y),B())})}}catch(u){e.error=u instanceof Error?u.message:String(u),p.innerHTML=`<div class="error" role="alert"><h2>无法加载</h2><p>${e.error}</p></div>`}finally{e.loading=!1}}async function z(){var A;e.meta=await T.meta(e.asOfSnapshot);const a=await T.timeline(),l=e.meta.configGeneration??"—",d=e.meta.httpListen??"—",$=a.items??[],o=$.length?$[$.length-1]:null,f=e.asOfSnapshot??(o==null?void 0:o.snapshot)??"—";n.querySelector("#genChip").textContent=String(l),(A=n.querySelector("#genChip"))==null||A.addEventListener("click",()=>M("config")),n.querySelector("#snapId").textContent=f==="—"?"—":`#${f}`,n.querySelector("#snapTime").textContent=o!=null&&o.publishedAt?U(o.publishedAt):"";const h=n.querySelector("#snapMode"),C=n.querySelector("#snapLive"),J=n.querySelector("#liveLabel");e.asOfSnapshot!=null?(h.textContent="历史",J.textContent="历史",C.hidden=!1,C.onclick=()=>Q(null)):(h.textContent="实时",J.textContent="实时",C.hidden=!0),j.innerHTML=`
      <div class="strata-glyph" aria-hidden="true"><span style="--i:0"></span><span style="--i:1"></span><span style="--i:2"></span><span style="--i:3"></span><span style="--i:4"></span></div>
      <div class="foot-row"><span>配置代际</span><strong>${l}</strong></div>
      <div class="foot-row"><span>查看历史</span><span id="asOfHost"></span></div>
      <div class="foot-row"><span>环回入口</span><span class="mono">${i(String(d))}</span></div>
      <p class="foot-note">只读观察者 · 写操作以变更集提交</p>`;const u=j.querySelector("#asOfHost");de(u,[{value:"",label:"实时"},...(a.items??[]).map(S=>({value:String(S.snapshot),label:String(S.label??S.snapshot)}))],e.asOfSnapshot!=null?String(e.asOfSnapshot):"",S=>Q(S?Number(S):null))}function ae(){F.hidden=!1,m.value="",m.focus();const a=n.querySelector("#paletteResults"),l=[...K.map(d=>({label:`前往：${d.label}`,run:()=>M(d.id)})),{label:"校验配置",run:()=>T.validateConfig().then(()=>O("校验","通过"))}];a.innerHTML=l.map((d,$)=>`<button type="button" data-i="${$}">${d.label}</button>`).join(""),a.querySelectorAll("button").forEach(d=>{d.addEventListener("click",()=>{l[Number(d.dataset.i)].run(),F.hidden=!0})})}(oe=n.querySelector("#paletteBtn"))==null||oe.addEventListener("click",ae),window.addEventListener("keydown",a=>{(a.ctrlKey||a.metaKey)&&a.key.toLowerCase()==="k"&&(a.preventDefault(),ae()),a.key==="Escape"&&(F.hidden=!0)}),(le=F.querySelector(".palette-scrim"))==null||le.addEventListener("click",()=>{F.hidden=!0});const se=n.querySelector("#themeSeg"),ne=()=>{const a=pe();se.querySelectorAll("button").forEach(l=>{l.setAttribute("aria-pressed",String(l.dataset.theme===a))})};se.querySelectorAll("button").forEach(a=>{a.addEventListener("click",()=>{Le(a.dataset.theme),ne()})}),ne();const re=()=>{const a=location.hash.replace(/^#/,"");return K.some(l=>l.id===a)?a:"overview"};return window.addEventListener("hashchange",()=>M(re())),M(re()),ee(),z().then(()=>B()),{setRoute:M,refresh:()=>B()}}const Z=new URLSearchParams(location.search).get("theme");X(Z==="light"||Z==="dark"?Z:pe());Ee(document.querySelector("#app"));
