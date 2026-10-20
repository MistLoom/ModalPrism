(function(){const e=document.createElement("link").relList;if(e&&e.supports&&e.supports("modulepreload"))return;for(const r of document.querySelectorAll('link[rel="modulepreload"]'))b(r);new MutationObserver(r=>{for(const i of r)if(i.type==="childList")for(const $ of i.addedNodes)$.tagName==="LINK"&&$.rel==="modulepreload"&&b($)}).observe(document,{childList:!0,subtree:!0});function g(r){const i={};return r.integrity&&(i.integrity=r.integrity),r.referrerPolicy&&(i.referrerPolicy=r.referrerPolicy),r.crossOrigin==="use-credentials"?i.credentials="include":r.crossOrigin==="anonymous"?i.credentials="omit":i.credentials="same-origin",i}function b(r){if(r.ep)return;r.ep=!0;const i=g(r);fetch(r.href,i)}})();const re="x-modalprism-console";function ie(){return{Accept:"application/json"}}function oe(){return{Accept:"application/json","Content-Type":"application/json",[re]:"1"}}function B(t){if(!t)return"—";const e=new Date(t);return Number.isNaN(e.getTime())?t:e.toLocaleString(void 0,{year:"numeric",month:"2-digit",day:"2-digit",hour:"2-digit",minute:"2-digit"})}function ee(t){return t!=null?`?asOfSnapshot=${t}`:""}async function S(t,e){var r;const g=await fetch(`/console/api/v1${t}`,{...e,headers:{...e!=null&&e.method&&e.method!=="GET"?oe():ie(),...(e==null?void 0:e.headers)??{}}}),b=await g.json().catch(()=>({}));if(!g.ok){const i=b;throw new Error(((r=i.error)==null?void 0:r.message)??g.statusText)}return b}const w={meta:t=>S(`/meta${ee(t)}`),timeline:()=>S("/timeline"),overview:()=>S("/overview"),sources:()=>S("/sources"),trigger:t=>S(`/sources/${encodeURIComponent(t)}/trigger`,{method:"POST",body:JSON.stringify({confirm:!0})}),patchSource:(t,e)=>S(`/sources/${encodeURIComponent(t)}`,{method:"PATCH",body:JSON.stringify({enabled:e,confirm:!0})}),records:t=>{const e=new URLSearchParams;t.limit&&e.set("limit",String(t.limit)),t.cursor&&e.set("cursor",t.cursor),t.source&&e.set("source",t.source),t.q&&e.set("q",t.q),t.asOf!=null&&e.set("asOfSnapshot",String(t.asOf));const g=e.toString();return S(`/records${g?`?${g}`:""}`)},record:(t,e)=>S(`/records/${encodeURIComponent(t)}${ee(e)}`),governance:()=>S("/governance/egress"),resetCircuit:t=>S(`/governance/egress/${encodeURIComponent(t)}/reset-circuit`,{method:"POST",body:JSON.stringify({confirm:!0})}),configSummary:()=>S("/config/summary"),configFiles:()=>S("/config/files"),configFile:t=>S(`/config/files/${encodeURIComponent(t)}`),validateFileText:(t,e)=>S("/config/file-validate",{method:"POST",body:JSON.stringify({path:t,text:e})}),putConfigFile:(t,e)=>S(`/config/files/${encodeURIComponent(t)}`,{method:"PUT",body:JSON.stringify({text:e,confirm:!0})}),rollbackConfigFile:t=>S("/config/file-rollback",{method:"POST",body:JSON.stringify({path:t,confirm:!0})}),validateConfig:()=>S("/config/validate",{method:"POST",body:"{}"}),reloadConfig:()=>S("/config/reload",{method:"POST",body:"{}"})},ae="modalprism-console-theme";function se(){const t=localStorage.getItem(ae);return t==="light"||t==="dark"||t==="system"?t:"light"}function le(t){localStorage.setItem(ae,t),K(t)}function K(t){const e=document.documentElement;e.dataset.theme=t;const g=t==="dark"||t==="system"&&window.matchMedia("(prefers-color-scheme: dark)").matches;e.classList.toggle("theme-dark",g),e.classList.toggle("theme-light",!g)}function ce(t,e,g,b){t.innerHTML=`
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
  `;const r=t.querySelector(".cfg-textarea"),i=t.querySelector("#cfgGutter"),$=t.querySelector("#cfgDiffView"),d=t.querySelector("#cfgDiffHint"),j=t.querySelector("#cfgToggleDiff");let D=g,M=null,u=!1;r.value=e;const E=()=>{const v=r.value.split(`
`);i.innerHTML=v.map((O,A)=>{const x=A+1;return`<span class="cfg-ln${M===x?" cfg-line-error":""}">${x}</span>`}).join("")},y=()=>{const v=D.split(`
`),O=r.value.split(`
`),A=Math.max(v.length,O.length),x=[];for(let P=0;P<A;P++){const I=v[P]??"",c=O[P]??"";I===c?x.push(`<span class="diff-eq"> ${U(c)}</span>`):I&&!c?x.push(`<span class="diff-del">-${U(I)}</span>`):!I&&c?x.push(`<span class="diff-add">+${U(c)}</span>`):(x.push(`<span class="diff-del">-${U(I)}</span>`),x.push(`<span class="diff-add">+${U(c)}</span>`))}$.innerHTML=x.join(`
`)},k=()=>{E();const v=r.value!==D;d.textContent=v?"与磁盘版本有差异（未保存）":"与磁盘版本一致",u&&y(),b(v)};return r.addEventListener("input",k),r.addEventListener("scroll",()=>{i.scrollTop=r.scrollTop}),j.addEventListener("click",()=>{u=!u,r.hidden=u,$.hidden=!u,j.textContent=u?"返回编辑":"查看与磁盘差异",u&&y()}),k(),{getValue:()=>r.value,setValue(v){r.value=v,k()},setDiskVersion(v){D=v,k()},setErrorLine(v){if(M=v,E(),v!=null){const O=v-1,A=r.value.split(`
`);O>=0&&O<A.length&&(r.focus(),r.setSelectionRange(A.slice(0,O).join(`
`).length+(O>0?1:0),A.slice(0,O+1).join(`
`).length))}},focus:()=>r.focus()}}function U(t){return t.replace(/&/g,"&amp;").replace(/</g,"&lt;")}function te(t,e,g,b,r="请选择"){const i=e.find(u=>u.value===g);t.className="custom-select",t.innerHTML=`
    <button type="button" class="custom-select-trigger" aria-haspopup="listbox" aria-expanded="false">
      <span class="custom-select-label">${(i==null?void 0:i.label)??r}</span>
      <span class="custom-select-chevron" aria-hidden="true"></span>
    </button>
    <ul class="custom-select-menu" role="listbox" hidden></ul>
  `;const $=t.querySelector(".custom-select-menu"),d=t.querySelector(".custom-select-trigger"),j=t.querySelector(".custom-select-label"),D=()=>{$.innerHTML=e.map(u=>`<li role="option" data-value="${de(u.value)}" aria-selected="${u.value===g}">${ne(u.label)}</li>`).join("")};D();const M=()=>{$.hidden=!0,d.setAttribute("aria-expanded","false")};return d.addEventListener("click",u=>{u.stopPropagation();const E=$.hidden;document.querySelectorAll(".custom-select-menu").forEach(y=>{y.hidden=!0}),$.hidden=!E,d.setAttribute("aria-expanded",String(E))}),$.addEventListener("click",u=>{var k;const E=u.target.closest("[data-value]");if(!E)return;const y=E.dataset.value??"";g=y,j.textContent=((k=e.find(v=>v.value===y))==null?void 0:k.label)??r,D(),M(),b(y)}),document.addEventListener("click",M),{setValue(u){var E;g=u,j.textContent=((E=e.find(y=>y.value===u))==null?void 0:E.label)??r,D()},setOptions(u){e=u,D()}}}function ne(t){return t.replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/"/g,"&quot;")}function de(t){return ne(t)}const J=[{id:"overview",label:"总览"},{id:"sources",label:"信源"},{id:"records",label:"数据记录"},{id:"governance",label:"治理"},{id:"config",label:"配置"}];function ue(t){var z,X;const e={route:"overview",loading:!0,error:null,meta:null,asOfSnapshot:null,configDirty:!1,configPath:null};let g="",b=null;const r=()=>e.asOfSnapshot!=null,i=document.createElement("div");i.className="shell",i.innerHTML=`
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
  `,t.appendChild(i);const $=i.querySelector("#nav"),d=i.querySelector("#viewRoot"),j=i.querySelector("#crumb"),D=i.querySelector("#foot"),M=i.querySelector("#palette"),u=i.querySelector("#paletteInput"),E=i.querySelector("#confirm");function y(a,o){const l=document.createElement("div");l.className="toast",l.innerHTML=`<strong>${a}</strong><p>${o}</p>`,i.querySelector("#toasts").appendChild(l),setTimeout(()=>l.remove(),4e3)}function k(a,o){return new Promise(l=>{i.querySelector("#confirmTitle").textContent=a,i.querySelector("#confirmBody").textContent=o,E.hidden=!1;const H=i.querySelector("#confirmOk"),f=i.querySelector("#confirmCancel"),R=V=>{E.hidden=!0,H.onclick=null,f.onclick=null,l(V)};H.onclick=()=>R(!0),f.onclick=()=>R(!1)})}async function v(a){if(e.configDirty&&e.route==="config"&&a!=="config"){if(!await k("未保存的修改","离开配置页将丢失未保存的 YAML 修改。"))return;e.configDirty=!1}e.route=a,location.hash.replace(/^#/,"")!==a&&(location.hash=a),A(),N()}function O(a){var l;e.asOfSnapshot=a;const o=i.querySelector("#historyBanner");a==null?o.hidden=!0:(o.hidden=!1,o.innerHTML=`<div class="hb-copy"><strong>历史状态 #${a}</strong><span class="hb-note">数据记录按所选时间点显示；其他视图为实时数据且不可写入。</span></div><button type="button" class="btn btn-sm btn-quiet" id="liveBtn">回到实时</button>`,(l=o.querySelector("#liveBtn"))==null||l.addEventListener("click",()=>O(null))),Y().then(()=>N())}window.addEventListener("beforeunload",a=>{e.configDirty&&a.preventDefault()});function A(){var H;const a=J.filter(f=>f.id==="overview"||f.id==="sources"||f.id==="records"),o=J.filter(f=>f.id==="governance"||f.id==="config"),l=f=>`<button type="button" class="nav-item" data-route="${f.id}" aria-current="${e.route===f.id?"page":"false"}"><span class="nav-icon" aria-hidden="true"></span><span>${f.label}</span></button>`;$.innerHTML=`
      <div class="nav-group"><span class="nav-label">观测</span>${a.map(l).join("")}</div>
      <div class="nav-group"><span class="nav-label">系统</span>${o.map(l).join("")}</div>`,$.querySelectorAll("[data-route]").forEach(f=>{f.addEventListener("click",()=>v(f.dataset.route))}),j.textContent=((H=J.find(f=>f.id===e.route))==null?void 0:H.label)??""}function x(a){return!a||e.route==="records"?"":`<div class="view-note" role="status">${a}</div>`}function P(a){const o=a.match(/第\s*(\d+)\s*行/);return o?Number(o[1]):null}async function I(a){const o=d.querySelector("#recordDrawer");if(!o)return;o.hidden=!1;const l=async()=>{var H,f,R,V;o.innerHTML=`
        <header class="drawer-head">
          <h2 class="drawer-title" id="drawerTitle">加载中…</h2>
          <button type="button" class="icon-btn drawer-close" id="closeDrawer" aria-label="关闭">×</button>
        </header>
        <div class="drawer-body"><p class="loading">加载中…</p></div>`,(H=o.querySelector("#closeDrawer"))==null||H.addEventListener("click",()=>{o.hidden=!0});try{const s=await w.record(a,e.asOfSnapshot),L=s.title||"未命名";o.innerHTML=`
          <header class="drawer-head">
            <h2 class="drawer-title">${c(L)}</h2>
            <button type="button" class="icon-btn drawer-close" id="closeDrawer" aria-label="关闭">×</button>
          </header>
          <div class="drawer-body">
            <p class="muted">${c(String(s.sourceName??s.sourceId??""))}</p>
            <dl class="detail-dl">
              <dt>发布时间</dt><dd>${B(s.publishedAt)}</dd>
              <dt>采集时间</dt><dd>${B(s.collectedAt??s.updatedAt)}</dd>
              <dt>原文</dt><dd>${s.url?`<a href="${s.url}" target="_blank" rel="noopener">${c(String(s.url))}</a>`:"—"}</dd>
              <dt>摘要</dt><dd>${c(String(s.summary??"—"))}</dd>
            </dl>
            <pre class="body-preview">${c(String(s.bodyPreview??""))}</pre>
            <details class="tech-details"><summary>技术详情</summary><pre>${c(JSON.stringify(s.technical,null,2))}</pre></details>
          </div>`,(f=o.querySelector("#closeDrawer"))==null||f.addEventListener("click",()=>{o.hidden=!0})}catch(s){const L=s instanceof Error?s.message:String(s);o.innerHTML=`
          <header class="drawer-head">
            <h2 class="drawer-title">无法加载记录</h2>
            <button type="button" class="icon-btn drawer-close" id="closeDrawer" aria-label="关闭">×</button>
          </header>
          <div class="drawer-body error"><p>${c(L)}</p>
            <button type="button" class="btn btn-primary" id="retryDrawer">重试</button></div>`,(R=o.querySelector("#closeDrawer"))==null||R.addEventListener("click",()=>{o.hidden=!0}),(V=o.querySelector("#retryDrawer"))==null||V.addEventListener("click",()=>void l())}};await l()}function c(a){return a.replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/"/g,"&quot;")}async function N(){var a,o,l,H,f,R,V;e.loading=!0,e.error=null,d.innerHTML='<p class="loading">加载中…</p>';try{if(e.route==="overview"){const s=await w.overview(),L=s.sources??[],q=s.viewNote;d.innerHTML=`
          ${x(q)}
          <div class="page-head"><div><h1 class="page-title">总览</h1><p class="page-desc">采集进程与信源健康摘要。</p></div></div>
          <div class="stat-grid">
            <div class="stat od-stat"><span class="stat-label">信源</span><strong class="stat-value">${s.sourceCount}</strong><span class="stat-hint">已声明的采集信源</span></div>
            <div class="stat od-stat"><span class="stat-label">健康</span><strong class="stat-value">${s.healthyCount}</strong><span class="stat-hint">状态为正常的信源</span></div>
            <div class="stat od-stat"><span class="stat-label">配置版本</span><strong class="stat-value">${((a=e.meta)==null?void 0:a.configGeneration)??"—"}</strong><span class="stat-hint">当前生效的配置代际</span></div>
          </div>
          <h2 class="section-title">信源状态</h2>
          <table class="table"><thead><tr><th>名称</th><th>状态</th><th>下次采集</th><th>最近采集</th><th>今日条数</th><th>说明</th></tr></thead>
          <tbody>${L.slice(0,16).map(n=>{const m=String(n.statusChipClass??"status-muted"),T=n.skipReasonLabel??n.healthLabel??n.healthStatus,p=n.nextAt?B(n.nextAt):"—",F=n.lastYieldAt?B(n.lastYieldAt):"—",h=n.todayYieldCount??0,C=n.enabled===!1?"已停用":n.scheduleNote??n.lastError??"—";return`<tr><td>${c(String(n.name))}</td><td><span class="status-chip ${m}">${c(String(T))}</span></td><td>${p}</td><td>${F}</td><td>${h}</td><td class="muted">${c(String(C))}</td></tr>`}).join("")}</tbody></table>`}else if(e.route==="sources"){const s=await w.sources(),L=s.items,q=r()||!!s.readOnly;d.innerHTML=`
          ${x(s.viewNote)}
          <div class="page-head"><div><h1 class="page-title">信源</h1><p class="page-desc">已声明的采集信源。启用状态与调度来自 YAML 配置。</p></div></div>
          <table class="table"><thead><tr><th>信源</th><th>访问器</th><th>状态</th><th>操作</th></tr></thead>
          <tbody>${L.map(n=>{const m=String(n.statusChipClass??"status-muted"),T=n.skipReasonLabel??n.healthLabel??n.healthStatus;return`<tr data-id="${n.id}">
                <td>${c(String(n.name))}<div class="muted mono">${c(String(n.id))}</div></td>
                <td>${c(String(n.accessor))}</td>
                <td><span class="status-chip ${m}">${c(String(T))}</span><div class="muted">${c(String(n.scheduleNote??""))}</div></td>
                <td>
                  <button type="button" class="btn btn-sm" data-act="trigger" ${q?"disabled":""}>立即采集</button>
                  <button type="button" class="btn btn-sm btn-quiet" data-act="toggle" ${q?"disabled":""}>${n.enabled?"停用":"启用"}</button>
                </td></tr>`}).join("")}</tbody></table>`,d.querySelectorAll("[data-act]").forEach(n=>{const T=n.closest("tr").dataset.id;n.addEventListener("click",async()=>{if(q)return;const p=n.dataset.act;if(p==="trigger")await w.trigger(T.replace("source/","")),y("已触发",T);else if(p==="toggle"){const F=n.textContent==="启用";if(!await k("修改信源启用状态？",`${T} → enabled=${F}`))return;await w.patchSource(T.replace("source/",""),F),y("已更新","配置已校验并重新加载"),N()}})})}else if(e.route==="records"){const s=d.dataset.sourceFilter||"",L=d.dataset.q||"",{items:q,nextCursor:n,sourceOptions:m}=await w.records({limit:50,source:s||void 0,q:L||void 0,asOf:e.asOfSnapshot});d.innerHTML=`
          <div class="page-head"><div><h1 class="page-title">数据记录</h1><p class="page-desc">已沉积的采集条目。${e.asOfSnapshot!=null?"当前按所选历史状态显示。":""}</p></div></div>
          <div class="filter-row">
            <input id="recQ" class="input" placeholder="按标题搜索…" value="${c(L)}" />
            <div id="recSourceSelect"></div>
            <button type="button" class="btn" id="recFilter">筛选</button>
          </div>
          ${q.length===0?'<div class="empty">尚无记录。完成一轮采集并等待数据发布后刷新。</div>':`<table class="table"><thead><tr><th>标题</th><th>信源</th><th>发布时间</th><th>采集时间</th></tr></thead>
          <tbody>${q.map(p=>{const F=p.title||String(p.id),h=p.sourceName||p.sourceId||"—";return`<tr class="rec-row" data-id="${p.id}"><td class="od-clamp-2">${c(F)}</td><td>${c(h)}</td><td>${B(p.publishedAt)}</td><td>${B(p.collectedAt??p.updatedAt)}</td></tr>`}).join("")}</tbody></table>
          ${n?'<button type="button" class="btn" id="moreRec">加载更多</button>':""}`}
          <aside class="drawer" id="recordDrawer" hidden></aside>`;const T=d.querySelector("#recSourceSelect");te(T,[{value:"",label:"全部信源"},...(m??[]).map(p=>({value:p.id,label:p.name}))],s,p=>{d.dataset.sourceFilter=p}),(o=d.querySelector("#recFilter"))==null||o.addEventListener("click",()=>{d.dataset.q=d.querySelector("#recQ").value,N()}),d.querySelectorAll(".rec-row").forEach(p=>{p.addEventListener("click",()=>void I(p.dataset.id))})}else if(e.route==="governance"){const s=await w.governance(),L=s.domains??[],q=r()||!!s.readOnly;d.innerHTML=`
          ${x(s.viewNote)}
          <div class="page-head"><div><h1 class="page-title">出站治理</h1><p class="page-desc">域级熔断与限速状态。代理：${c(String(s.proxy))}</p></div></div>
          <table class="table"><thead><tr><th>主机</th><th>熔断</th><th>操作</th></tr></thead>
          <tbody>${L.map(n=>`<tr><td>${c(String(n.host))}</td><td>${c(String(n.circuitLabel??n.circuit))}</td>
                <td>${n.circuit!=="closed"?`<button type="button" class="btn btn-sm" data-host="${c(String(n.host))}" ${q?"disabled":""}>重置熔断</button>`:"—"}</td></tr>`).join("")}</tbody></table>`,d.querySelectorAll("[data-host]").forEach(n=>{n.addEventListener("click",async()=>{if(q)return;const m=n.dataset.host;await k("重置熔断？",m)&&(await w.resetCircuit(m),y("已重置",m),N())})})}else{const s=r(),L=await w.configSummary(),q=await w.configFiles(),n=((l=q.groups)==null?void 0:l.flatMap(h=>h.items))??q.items.map(h=>({path:h,label:h})),m=e.configPath??((H=n[0])==null?void 0:H.path)??"presets/standard.yaml",T=await w.configFile(m);g=T.text,e.configPath=m,d.innerHTML=`
          ${s?'<div class="view-note" role="status">该视图显示实时状态。历史模式下不可编辑配置。</div>':""}
          <div class="page-head"><div><h1 class="page-title">配置</h1><p class="page-desc">仅可编辑信源声明与策略预设。保存前会校验，失败则不写入。</p></div></div>
          <div class="stat-grid">
            <div class="stat od-stat"><span class="stat-label">配置版本</span><strong class="stat-value">${L.generation}</strong><span class="stat-hint">当前生效代际</span></div>
            <div class="stat od-stat"><span class="stat-label">磁盘未加载变更</span><strong class="stat-value">${L.pendingDiffCount}</strong><span class="stat-hint">相对运行中配置的 diff 条数</span></div>
          </div>
          <div class="config-layout">
            <div class="config-files" id="cfgFileList"></div>
            <div id="cfgEditorHost"></div>
            <div class="btn-row">
              <button type="button" class="btn" id="btnValidateText" ${s?"disabled":""}>校验草稿</button>
              <button type="button" class="btn btn-primary" id="btnSaveCfg" ${s?"disabled":""}>保存并加载</button>
              <button type="button" class="btn btn-quiet" id="btnRollback" ${!T.canRollback||s?"disabled":""}>回滚上一版</button>
            </div>
          </div>`;const p=d.querySelector("#cfgFileList");p.innerHTML=(q.groups??[]).map(h=>`<section class="cfg-group"><h3>${c(h.label)}</h3><ul>${h.items.map(C=>`<li><button type="button" class="cfg-file${C.path===m?" active":""}" data-path="${c(C.path)}">${c(C.label)}</button></li>`).join("")}</ul></section>`).join(""),p.querySelectorAll("[data-path]").forEach(h=>{h.addEventListener("click",async()=>{const C=h.dataset.path;C!==m&&(e.configDirty&&!await k("未保存的修改","切换文件将丢失当前编辑内容。")||(e.configPath=C,e.configDirty=!1,N()))})});const F=d.querySelector("#cfgEditorHost");b=ce(F,T.text,g,h=>{e.configDirty=h}),(f=d.querySelector("#btnValidateText"))==null||f.addEventListener("click",async()=>{if(b)try{b.setErrorLine(null),await w.validateFileText(m,b.getValue()),y("校验通过",m)}catch(h){const C=h instanceof Error?h.message:String(h);b.setErrorLine(P(C)),y("校验失败",C)}}),(R=d.querySelector("#btnSaveCfg"))==null||R.addEventListener("click",async()=>{if(!(!b||s)&&await k("保存并加载？",`${m} 将写入磁盘并替换当前生效配置。`))try{b.setErrorLine(null),await w.putConfigFile(m,b.getValue()),e.configDirty=!1,g=b.getValue(),b.setDiskVersion(g),y("已保存","配置已校验并加载"),await Y(),N()}catch(h){const C=h instanceof Error?h.message:String(h);b.setErrorLine(P(C)),y("保存失败",C)}}),(V=d.querySelector("#btnRollback"))==null||V.addEventListener("click",async()=>{s||await k("回滚？","恢复为上一次保存前的内容并重新加载。")&&(await w.rollbackConfigFile(m),e.configDirty=!1,y("已回滚",m),N())})}}catch(s){e.error=s instanceof Error?s.message:String(s),d.innerHTML=`<div class="error" role="alert"><h2>无法加载</h2><p>${e.error}</p></div>`}finally{e.loading=!1}}async function Y(){e.meta=await w.meta(e.asOfSnapshot);const a=await w.timeline();D.innerHTML=`
      <div class="foot-row"><span>配置版本</span><strong>${e.meta.configGeneration}</strong></div>
      <div class="foot-row"><span>查看历史状态</span><span id="asOfHost"></span></div>
      <p class="foot-note">写操作需确认后提交</p>`;const o=D.querySelector("#asOfHost");te(o,[{value:"",label:"实时"},...(a.items??[]).map(l=>({value:String(l.snapshot),label:String(l.label??l.snapshot)}))],e.asOfSnapshot!=null?String(e.asOfSnapshot):"",l=>O(l?Number(l):null))}function Q(){M.hidden=!1,u.value="",u.focus();const a=i.querySelector("#paletteResults"),o=[...J.map(l=>({label:`前往：${l.label}`,run:()=>v(l.id)})),{label:"校验配置",run:()=>w.validateConfig().then(()=>y("校验","通过"))}];a.innerHTML=o.map((l,H)=>`<button type="button" data-i="${H}">${l.label}</button>`).join(""),a.querySelectorAll("button").forEach(l=>{l.addEventListener("click",()=>{o[Number(l.dataset.i)].run(),M.hidden=!0})})}(z=i.querySelector("#paletteBtn"))==null||z.addEventListener("click",Q),window.addEventListener("keydown",a=>{(a.ctrlKey||a.metaKey)&&a.key.toLowerCase()==="k"&&(a.preventDefault(),Q()),a.key==="Escape"&&(M.hidden=!0)}),(X=M.querySelector(".palette-scrim"))==null||X.addEventListener("click",()=>{M.hidden=!0});const _=i.querySelector("#themeSeg"),W=()=>{const a=se();_.querySelectorAll("button").forEach(o=>{o.setAttribute("aria-pressed",String(o.dataset.theme===a))})};_.querySelectorAll("button").forEach(a=>{a.addEventListener("click",()=>{le(a.dataset.theme),W()})}),W();const Z=()=>{const a=location.hash.replace(/^#/,"");return J.some(o=>o.id===a)?a:"overview"};return window.addEventListener("hashchange",()=>v(Z())),v(Z()),A(),Y().then(()=>N()),{setRoute:v,refresh:()=>N()}}const G=new URLSearchParams(location.search).get("theme");K(G==="light"||G==="dark"?G:se());ue(document.querySelector("#app"));
