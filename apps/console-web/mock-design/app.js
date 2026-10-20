(() => {
  'use strict';

  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => Array.from(r.querySelectorAll(s));
  const esc = (v) => String(v == null ? '' : v).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const fmt = (n) => new Intl.NumberFormat('en-US').format(n);
  const pad = (n) => String(n).padStart(2, '0');

  const ICONS = {
    overview: '<circle cx="12" cy="12" r="3.2"/><path d="M12 2.5v4M12 17.5v4M2.5 12h4M17.5 12h4"/>',
    sources: '<path d="M4 6h16M4 12h16M4 18h9"/><circle cx="18.5" cy="18" r="1.4"/>',
    strata: '<path d="m12 3 9 4.5-9 4.5-9-4.5L12 3Z"/><path d="m3 12 9 4.5 9-4.5"/><path d="m3 16.5 9 4.5 9-4.5"/>',
    governance: '<path d="M12 3.2 19.5 6v5.4c0 4.3-3.1 7.5-7.5 9.4-4.4-1.9-7.5-5.1-7.5-9.4V6L12 3.2Z"/><path d="m8.8 12 2.2 2.2 4.4-4.6"/>',
    config: '<path d="M7.5 4H6a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h1.5M16.5 4H18a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-1.5"/><path d="M12 8.5v7M9.8 10.7 12 8.5l2.2 2.2"/>',
    search: '<circle cx="10.5" cy="10.5" r="6.2"/><path d="m15.2 15.2 4.3 4.3"/>',
    command: '<rect x="3" y="6" width="18" height="12" rx="2"/><path d="M7 10h.01M11 10h.01M15 10h.01M7 14h10"/>',
    'chevron-left': '<path d="m14.5 6-6 6 6 6"/>',
    'chevron-right': '<path d="m9.5 6 6 6-6 6"/>',
    'chevron-down': '<path d="m6 9.5 6 6 6-6"/>',
    close: '<path d="M6 6l12 12M18 6 6 18"/>',
    refresh: '<path d="M20 11a8 8 0 1 0-2.3 6.3"/><path d="M20 5v6h-6"/>',
    play: '<path d="m8 5 11 7-11 7V5Z"/>',
    pause: '<path d="M9 5v14M15 5v14"/>',
    copy: '<rect x="9" y="9" width="11" height="11" rx="1.5"/><path d="M15 9V5.5A1.5 1.5 0 0 0 13.5 4H5.5A1.5 1.5 0 0 0 4 5.5v8A1.5 1.5 0 0 0 5.5 15H9"/>',
    check: '<path d="m5 12.5 4.2 4.2L19 7"/>',
    alert: '<path d="M12 4 2.8 20h18.4L12 4Z"/><path d="M12 10v4M12 17h.01"/>',
    info: '<circle cx="12" cy="12" r="8.5"/><path d="M12 11v5M12 8h.01"/>',
    lock: '<rect x="5" y="10.5" width="14" height="9" rx="1.5"/><path d="M8.5 10.5V8a3.5 3.5 0 0 1 7 0v2.5"/>',
    clock: '<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/>',
    rss: '<path d="M4 11a9 9 0 0 1 9 9M4 4a16 16 0 0 1 16 16"/><circle cx="5" cy="19" r="1"/>',
    api: '<path d="m8 8-4 4 4 4M16 8l4 4-4 4M13.5 5 10.5 19"/>',
    browser: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 9h18M6 6.8h.01M8.5 6.8h.01"/>',
    file: '<path d="M6 3h8l4 4v14H6V3Z"/><path d="M14 3v5h5"/>',
    'arrow-right': '<path d="M4 12h15M13 6l6 6-6 6"/>',
    activity: '<path d="M3 12h4l2.5-6 4 12 2.5-6H21"/>'
  };

  const icon = (name, size = 16, cls = '') => `<svg class="od-icon ${cls}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" style="--od-icon-size:${size}px" aria-hidden="true">${ICONS[name] || ICONS.info}</svg>`;

  const STATUS = {
    ok: { label: '正常', color: '#A6C09A' },
    degraded: { label: '受限', color: '#D6A24A' },
    failing: { label: '失败', color: '#F06A45' },
    suspect: { label: '可疑', color: '#98AFBA' },
    misconfigured: { label: '配置错误', color: '#D6A24A' },
    paused: { label: '已暂停', color: '#7D837A' }
  };
  const SEVERITY = { failing: 0, suspect: 1, degraded: 2, misconfigured: 3, paused: 4, ok: 5 };
  const ACCESSOR = { rss: 'RSS', api: 'API', browser: '浏览器', http: 'HTTP', push: '推送', file: '文件' };

  const clockStart = 14 * 3600 + 32 * 60 + 8;
  let clockSec = clockStart;
  let commitSeq = 0;
  let lastFocus = null;

  const nowParts = () => {
    const total = clockStart + (clockSec - clockStart);
    return { h: Math.floor(total / 3600) % 24, m: Math.floor(total / 60) % 60, s: total % 60 };
  };
  const clockString = () => {
    const p = nowParts();
    return `${pad(p.h)}:${pad(p.m)}:${pad(p.s)}`;
  };
  const nowHM = () => {
    const p = nowParts();
    return `${pad(p.h)}:${pad(p.m)}`;
  };
  const nowHMS = () => clockString();

  const SOURCES = [
    {
      id: 'source/reuters-world', name: 'Reuters World', desc: '国际新闻列表与详情页，按需渲染', accessor: 'browser', status: 'failing', tier: 'T1', lang: 'en',
      interval: '2h', floor: '30m', ceiling: '2h', next: '12m 后', nextIn: 12, lastOk: '2 小时前', failures: 6,
      failure: '403 · 永久', lastError: 'HTTP 403 · 拦截页特征 s6', trend: 0.12, render: '兼容', preset: 'standard',
      extends: 'profile/news-browser', group: 'browser-tier1', cursor: 'cur_8f21c4', cost: '¥4.80', selectorMiss: 0, blockedRate: 0.62
    },
    {
      id: 'source/gh-trending', name: 'GitHub Trending', desc: '按日抓取趋势仓库列表与语言筛选页', accessor: 'browser', status: 'suspect', tier: 'T2', lang: 'en',
      interval: '45m', floor: '20m', ceiling: '3h', next: '9m 后', nextIn: 9, lastOk: '38 分钟前', failures: 0,
      failure: '', lastError: '选择器 .repo-list 连续未命中', trend: 0.38, render: '交互', preset: 'standard',
      extends: 'profile/browser-shared', group: 'browser-tier2', cursor: 'cur_1c90ee', cost: '¥1.20', selectorMiss: 4, blockedRate: 0.04
    },
    {
      id: 'source/arxiv-cl', name: 'arXiv cs.CL', desc: '计算语言学论文列表与摘要 API', accessor: 'api', status: 'degraded', tier: 'T1', lang: 'en',
      interval: '18m', floor: '15m', ceiling: '2h', next: '18m 后', nextIn: 18, lastOk: '6 分钟前', failures: 2,
      failure: '429 · 瞬时', lastError: 'HTTP 429 · Retry-After 90s', trend: 0.61, render: '结构', preset: 'archive',
      extends: 'profile/api-shared', group: 'api-shared', cursor: 'cur_44a1d0', cost: '¥2.10', selectorMiss: 0, blockedRate: 0.08
    },
    {
      id: 'source/weibo-hot', name: '微博热搜', desc: '公开热搜榜页面，受限速率', accessor: 'browser', status: 'degraded', tier: 'T3', lang: 'zh', interval: '22m',
      floor: '10m', ceiling: '1h', next: '4m 后', nextIn: 4, lastOk: '18 分钟前', failures: 2, failure: '429 · 瞬时',
      lastError: 'HTTP 429 · 自适应节流 -32%', trend: 0.55, render: '交互', preset: 'standard', extends: 'profile/browser-shared',
      group: 'browser-tier2', cursor: 'cur_5be102', cost: '¥0.90', selectorMiss: 1, blockedRate: 0.11
    },
    {
      id: 'source/caixin', name: '财新网', desc: '付费墙外索引页，仅登记外链', accessor: 'browser', status: 'misconfigured', tier: 'T1', lang: 'zh',
      interval: '—', floor: '30m', ceiling: '2h', next: '不派发', nextIn: 0, lastOk: '3 小时前', failures: 0,
      failure: '配置', lastError: 'spec.admission.allowUrlPrefixes 缺少 scheme', trend: 0, render: '会话', preset: 'minimal',
      extends: 'profile/news-browser', group: 'browser-tier1', cursor: 'cur_99aa03', cost: '¥0.00', selectorMiss: 0, blockedRate: 0.02
    },
    {
      id: 'source/producthunt', name: 'Product Hunt', desc: '每日产品榜，付费 API + 结果未知保护', accessor: 'api', status: 'paused', tier: 'T2', lang: 'en',
      interval: '4h', floor: '1h', ceiling: '6h', next: '预算窗口 6h', nextIn: 0, lastOk: '昨日 22:10', failures: 0,
      failure: '', lastError: '预算耗尽 · 不计失败', trend: 0, render: '结构', preset: 'minimal', extends: 'profile/paid-api',
      group: 'paid-daily', cursor: 'cur_paid_204', cost: '¥12.00', selectorMiss: 0, blockedRate: 0.00
    },
    {
      id: 'source/rust-blog', name: 'Rust Blog', desc: '官方博客与发布说明', accessor: 'rss', status: 'ok', tier: 'T1', lang: 'en',
      interval: '26m', floor: '15m', ceiling: '2h', next: '7m 后', nextIn: 7, lastOk: '2 分钟前', failures: 0,
      failure: '', lastError: '', trend: 0.92, render: '直连', preset: 'archive', extends: 'profile/longform',
      group: 'feed-tier1', cursor: 'cur_7f3a12', cost: '¥0.00', selectorMiss: 0, blockedRate: 0.00
    },
    {
      id: 'source/hn-front', name: 'Hacker News', desc: '首页与 Ask HN，官方 Firebase API', accessor: 'api', status: 'ok', tier: 'T1', lang: 'en',
      interval: '12m', floor: '5m', ceiling: '1h', next: '2m 后', nextIn: 2, lastOk: '1 分钟前', failures: 0,
      failure: '', lastError: '', trend: 1.08, render: '直连', preset: 'standard', extends: 'profile/api-shared',
      group: 'api-shared', cursor: 'cur_a901cd', cost: '¥0.00', selectorMiss: 0, blockedRate: 0.00
    },
    {
      id: 'source/sspai', name: '少数派', desc: '首页 Feed 与作者专栏', accessor: 'rss', status: 'ok', tier: 'T2', lang: 'zh',
      interval: '35m', floor: '20m', ceiling: '3h', next: '14m 后', nextIn: 14, lastOk: '4 分钟前', failures: 0,
      failure: '', lastError: '', trend: 0.88, render: '会话', preset: 'archive', extends: 'profile/longform',
      group: 'feed-tier1', cursor: 'cur_02be71', cost: '¥0.00', selectorMiss: 0, blockedRate: 0.01
    },
    {
      id: 'source/ruanyf', name: '阮一峰的网络日志', desc: '每周技术周刊与博客正文', accessor: 'rss', status: 'ok', tier: 'T2', lang: 'zh',
      interval: '40m', floor: '20m', ceiling: '4h', next: '21m 后', nextIn: 21, lastOk: '12 分钟前', failures: 0,
      failure: '', lastError: '', trend: 0.96, render: '直连', preset: 'archive', extends: 'profile/longform',
      group: 'feed-tier1', cursor: 'cur_77fc20', cost: '¥0.00', selectorMiss: 0, blockedRate: 0.00
    },
    {
      id: 'source/hf-papers', name: 'Hugging Face Papers', desc: '每日论文卡片与讨论数', accessor: 'api', status: 'ok', tier: 'T1', lang: 'en',
      interval: '30m', floor: '15m', ceiling: '2h', next: '11m 后', nextIn: 11, lastOk: '8 分钟前', failures: 0,
      failure: '', lastError: '', trend: 0.95, render: '结构', preset: 'standard', extends: 'profile/api-shared',
      group: 'api-shared', cursor: 'cur_cc09aa', cost: '¥1.40', selectorMiss: 0, blockedRate: 0.02
    },
    {
      id: 'source/lobsters', name: 'Lobsters', desc: '技术社区热榜与评论树', accessor: 'rss', status: 'ok', tier: 'T3', lang: 'en',
      interval: '50m', floor: '30m', ceiling: '4h', next: '26m 后', nextIn: 26, lastOk: '23 分钟前', failures: 0,
      failure: '', lastError: '', trend: 0.81, render: '直连', preset: 'standard', extends: 'profile/gentle-crawl',
      group: 'feed-tier2', cursor: 'cur_31b7d9', cost: '¥0.00', selectorMiss: 0, blockedRate: 0.00
    }
  ];

  const makeSpark = (seed, status) => {
    let x = seed * 97 + 11;
    const arr = [];
    for (let i = 0; i < 12; i += 1) {
      x = (x * 9301 + 49297) % 233280;
      let v = 22 + (x / 233280) * 68;
      if (status === 'failing') v *= Math.pow(0.82, i);
      if (status === 'suspect') v *= Math.pow(0.93, i);
      if (status === 'degraded') v *= Math.pow(0.965, i);
      if (status === 'paused') v = 4 + (x / 233280) * 3;
      if (status === 'ok') v = v * 0.7 + 20;
      arr.push(Math.max(2, Math.round(v)));
    }
    return arr;
  };
  SOURCES.forEach((s, i) => { s.spark = makeSpark(i + 3, s.status); });

  const COMMITS = [
    { hash: 'a1f9c2e', snap: '1842', kind: 'ingest', title: 'reuters-world 回填 · 写入 42 个 facet，游标前移', time: '14:31:42', producer: 'ingest/runner v0.8.2' },
    { hash: '7b0d4aa', snap: '1842', kind: 'health', title: '健康评估 · reuters-world → failing（连续 6 次 403）', time: '14:29:10', producer: 'health/evaluator v0.4.1' },
    { hash: '3c2e817', snap: '1842', kind: 'ingest', title: 'arxiv-cl 触发退避 · Retry-After 90s 写入节奏信号', time: '14:24:55', producer: 'ingest/runner v0.8.2' },
    { hash: 'f04b1c9', snap: '1841', kind: 'config', title: '配置代际 G-37 生效 · 174 个文件整体校验通过', time: '13:58:03', producer: 'config/loader v0.6.0' },
    { hash: '8d7a330', snap: '1841', kind: 'ingest', title: 'hn-front 领取 12 个任务 · 合并为 1 次 API 调用', time: '13:47:20', producer: 'ingest/runner v0.8.2' },
    { hash: '2b6e9f1', snap: '1840', kind: 'pause', title: 'producthunt 手动暂停 · 等待预算窗口恢复', time: '13:52:11', producer: 'console/operator' },
    { hash: 'e9c1a75', snap: '1839', kind: 'ingest', title: 'sspai 抓取完成 · 6 个新条目，3 个墓碑身份跳过', time: '13:36:44', producer: 'ingest/runner v0.8.2' },
    { hash: '5a8f0d2', snap: '1838', kind: 'config', title: '配置代际 G-36 生效 · 付费预算窗口改为 12h', time: '11:20:31', producer: 'config/loader v0.6.0' }
  ];

  const SNAPSHOTS = [
    { id: '1838', label: '#1838', at: '13:12', time: '2026-10-21 13:12', resources: 1262004, added: 2840, tasks: 96, egress: 31, cost: 16.2, healthy: 6, scale: 0.7, daily: '+3.1%' },
    { id: '1839', label: '#1839', at: '13:32', time: '2026-10-21 13:32', resources: 1264411, added: 3511, tasks: 111, egress: 38, cost: 16.7, healthy: 6, scale: 0.78, daily: '+4.8%' },
    { id: '1840', label: '#1840', at: '13:52', time: '2026-10-21 13:52', resources: 1268204, added: 5486, tasks: 96, egress: 44, cost: 17.1, healthy: 6, scale: 0.84, daily: '+6.9%' },
    { id: '1841', label: '#1841', at: '14:12', time: '2026-10-21 14:12', resources: 1274018, added: 6892, tasks: 143, egress: 55, cost: 17.9, healthy: 6, scale: 0.92, daily: '+9.7%' },
    { id: '1842', label: '#1842', at: '14:32', time: '2026-10-21 14:32', resources: 1284930, added: 8412, tasks: 128, egress: 42, cost: 18.4, healthy: 6, scale: 1, daily: '+12.4%' }
  ];

  const LOCATORS = {
    'source/reuters-world': 'https://www.reuters.com/world/',
    'source/gh-trending': 'https://github.com/trending?since=daily',
    'source/arxiv-cl': 'https://export.arxiv.org/api/query?search_query=cat:cs.CL',
    'source/weibo-hot': 'https://s.weibo.com/top/summary',
    'source/caixin': 'https://www.caixin.com/',
    'source/producthunt': 'https://api.producthunt.com/v2/api/graphql',
    'source/rust-blog': 'https://blog.rust-lang.org/feed.xml',
    'source/hn-front': 'https://hacker-news.firebaseio.com/v0/topstories.json',
    'source/sspai': 'https://sspai.com/feed',
    'source/ruanyf': 'https://www.ruanyifeng.com/blog/atom.xml',
    'source/hf-papers': 'https://huggingface.co/api/daily_papers',
    'source/lobsters': 'https://lobste.rs/rss'
  };

  const RESOURCES = [
    {
      rid: 'rid:01JA7K3M9X4Q2E8T6B0N5V3C7D',
      title: 'Rust 2026 路线图：把编译时间再砍一半',
      source: 'source/rust-blog', sourceName: 'Rust Blog', updated: '14:28', url: 'https://blog.rust-lang.org/2026/10/21/roadmap/',
      identity: ['url:https://blog.rust-lang.org/2026/10/21/roadmap/', 'ext:rust-blog/2026-roadmap'],
      relations: [{ rel: 'discovered_by', dir: 'out', target: 'source/rust-blog' }, { rel: 'links_to', dir: 'out', target: 'rid:01JA7K61…crates-io' }],
      facets: [
        { kind: 'raw', variant: null, versions: [{ v: 3, hash: 'sha256:9f3c…a1', size: '184 KB', producer: 'accessor/rss v2.3.0', at: '14:28' }] },
        { kind: 'md', variant: 'readable', versions: [
          { v: 1, hash: 'sha256:8a11…3d', size: '11.8 KB', producer: 'adapter/readability-md v1.4.0', at: '14:28', text: [
            'Rust 团队在 2026 路线图中把编译时间列为首要目标。新的查询缓存将跨 crate 复用中间结果，并允许在增量构建之间保留宏展开产物。',
            '路线图同时提到 const generics 的进一步收敛，以及 rustdoc 对跨 crate 文档的索引方式调整。两项改动都会改变现有缓存键的构成。'
          ] },
          { v: 2, hash: 'sha256:4b8e…7c', size: '12.4 KB', producer: 'adapter/readability-md v1.4.0', at: '14:29', text: [
            'Rust 团队在 2026 路线图中把编译时间列为首要目标。新的查询缓存将跨 crate 复用中间结果，并允许在增量构建之间保留宏展开产物。官方估计，这一项会覆盖 Cargo 工作区内约七成的重复查询。',
            '路线图同时提到 const generics 的进一步收敛，以及 rustdoc 对跨 crate 文档的索引方式调整。两项改动都会改变现有缓存键的构成，因此旧的增量缓存会在升级后失效一次。',
            '团队给出的验收标准是：在 crates.io 前 1000 个 crate 的基准集上，冷启动编译时间下降 38%，增量构建的 p95 延迟不超过 1.2 秒。'
          ] }
        ] },
        { kind: 'struct', variant: 'article', versions: [
          { v: 1, hash: 'sha256:3d92…aa', size: '1.9 KB', producer: 'adapter/jsonld-struct v0.9.1', at: '14:28', yaml: 'title: "Rust 2026 路线图"\nauthor: "Rust Project"\npublished_at: "2026-10-21T09:00:00+08:00"\ncanonical_url: "https://blog.rust-lang.org/2026/10/21/roadmap/"\nlanguage: en' },
          { v: 2, hash: 'sha256:c71a…02', size: '2.1 KB', producer: 'adapter/jsonld-struct v0.9.1', at: '14:29', yaml: 'title: "Rust 2026 路线图：把编译时间再砍一半"\nauthor: "Rust Project"\npublished_at: "2026-10-21T09:00:00+08:00"\ncanonical_url: "https://blog.rust-lang.org/2026/10/21/roadmap/"\nlanguage: en\ntags: [rust, compiler, roadmap]\nword_count: 1420' }
        ] },
        { kind: 'links', variant: null, versions: [{ v: 1, hash: 'sha256:0b81…f2', size: '38 links', producer: 'adapter/link-discovery v1.1.0', at: '14:28', links: [
          { label: 'Rust 2026 Roadmap', url: 'https://blog.rust-lang.org/2026/10/21/roadmap/' },
          { label: 'crates.io benchmark set', url: 'https://github.com/rust-lang/crates.io' },
          { label: 'rustc query cache RFC', url: 'https://github.com/rust-lang/rfcs' }
        ] }] },
        { kind: 'embedding', variant: 'bge-m3', versions: [{ v: 1, hash: 'sha256:e410…9c', size: '1024 维 · 4 KB', producer: 'adapter/embed-bge v0.3.0', at: '14:29', dims: 1024 }] }
      ]
    },
    {
      rid: 'rid:01JA7JY2V6QK8M4R1T9Z0P5W3E',
      title: 'Show HN: A local-first sync engine in 900 lines',
      source: 'source/hn-front', sourceName: 'Hacker News', updated: '14:21', url: 'https://news.ycombinator.com/item?id=44719021',
      identity: ['ext:hn/44719021', 'url:https://news.ycombinator.com/item?id=44719021'],
      relations: [{ rel: 'discovered_by', dir: 'out', target: 'source/hn-front' }, { rel: 'part_of', dir: 'out', target: 'rid:01JA7JY2…hn-front' }],
      facets: [
        { kind: 'raw', variant: null, versions: [{ v: 2, hash: 'sha256:cc41…09', size: '96 KB', producer: 'accessor/api v1.8.0', at: '14:21' }] },
        { kind: 'md', variant: 'readable', versions: [{ v: 1, hash: 'sha256:19e2…b3', size: '8.2 KB', producer: 'adapter/readability-md v1.4.0', at: '14:21', text: [
          '作者用一个单文件同步引擎演示了 CRDT 之外的另一条路径：把冲突解决推迟到读取时，让写入路径只追加操作日志。代码约 900 行，包含 IndexedDB 与 SQLite 两个后端。',
          '讨论集中在离线冲突的可见性与压缩成本。作者回应称，日志按周分段，旧段会在所有设备确认后折叠为快照。'
        ] }] },
        { kind: 'struct', variant: 'article', versions: [{ v: 1, hash: 'sha256:77aa…1c', size: '1.4 KB', producer: 'adapter/jsonld-struct v0.9.1', at: '14:21', yaml: 'title: "Show HN: A local-first sync engine in 900 lines"\nauthor: "hn_user_ethan"\npoints: 412\ncomments: 128\npublished_at: "2026-10-21T12:40:00+08:00"\ncanonical_url: "https://news.ycombinator.com/item?id=44719021"' }] },
        { kind: 'links', variant: null, versions: [{ v: 1, hash: 'sha256:8581…4d', size: '6 links', producer: 'adapter/link-discovery v1.1.0', at: '14:21', links: [
          { label: 'GitHub · local-first-sync', url: 'https://github.com/' },
          { label: 'Hacker News 讨论', url: 'https://news.ycombinator.com/item?id=44719021' }
        ] }] },
        { kind: 'embedding', variant: 'bge-m3', versions: [{ v: 1, hash: 'sha256:1a7c…d0', size: '1024 维 · 4 KB', producer: 'adapter/embed-bge v0.3.0', at: '14:22', dims: 1024 }] }
      ]
    },
    {
      rid: 'rid:01JA7J6T8N2P4X6C0V8B3M5Q7L',
      title: 'Scaling laws for retrieval-augmented training',
      source: 'source/arxiv-cl', sourceName: 'arXiv cs.CL', updated: '14:18', url: 'https://arxiv.org/abs/2610.09121',
      identity: ['ext:arxiv/2610.09121', 'url:https://arxiv.org/abs/2610.09121'],
      relations: [{ rel: 'discovered_by', dir: 'out', target: 'source/arxiv-cl' }, { rel: 'same_as', dir: 'out', target: 'rid:01JA7J6T…openreview' }],
      facets: [
        { kind: 'raw', variant: null, versions: [{ v: 1, hash: 'sha256:6d20…7e', size: '212 KB', producer: 'accessor/api v1.8.0', at: '14:18' }] },
        { kind: 'md', variant: 'readable', versions: [{ v: 1, hash: 'sha256:b0c4…52', size: '18.6 KB', producer: 'adapter/readability-md v1.4.0', at: '14:18', text: [
          '论文系统比较了检索增强训练中语料规模、检索深度与模型参数量的关系。作者发现，在固定计算预算下，检索深度比语料规模更快达到收益饱和。',
          '在 8 个下游任务上，最优检索深度稳定在 12 到 16 篇之间。超过该区间后，上下文长度带来的训练开销开始抵消召回收益。'
        ] }] },
        { kind: 'struct', variant: 'paper', versions: [{ v: 1, hash: 'sha256:42f0…bb', size: '2.6 KB', producer: 'adapter/arxiv-struct v1.2.0', at: '14:18', yaml: 'title: "Scaling laws for retrieval-augmented training"\nauthors: ["L. Chen", "M. Okafor", "R. Ito"]\narxiv_id: "2610.09121"\nprimary_class: "cs.CL"\npublished_at: "2026-10-20T22:14:00+08:00"\nabstract_chars: 1284' }] },
        { kind: 'links', variant: null, versions: [{ v: 1, hash: 'sha256:99c2…18', size: '11 links', producer: 'adapter/link-discovery v1.1.0', at: '14:18', links: [
          { label: 'arXiv 摘要页', url: 'https://arxiv.org/abs/2610.09121' },
          { label: 'PDF', url: 'https://arxiv.org/pdf/2610.09121' }
        ] }] },
        { kind: 'embedding', variant: 'bge-m3', versions: [{ v: 1, hash: 'sha256:f21a…34', size: '1024 维 · 4 KB', producer: 'adapter/embed-bge v0.3.0', at: '14:19', dims: 1024 }] }
      ]
    },
    {
      rid: 'rid:01JA7GZ5H4D8S2K6M0R9T3B7N1',
      title: 'GitHub Trending · 2026-10-21',
      source: 'source/gh-trending', sourceName: 'GitHub Trending', updated: '14:02', url: 'https://github.com/trending',
      identity: ['url:https://github.com/trending?since=daily', 'src:gh-trending/2026-10-21'],
      relations: [{ rel: 'discovered_by', dir: 'out', target: 'source/gh-trending' }, { rel: 'has_part', dir: 'out', target: 'rid:01JA7GZ5…12-repos' }],
      facets: [
        { kind: 'raw', variant: null, versions: [{ v: 1, hash: 'sha256:8e11…65', size: '148 KB', producer: 'accessor/browser v0.9.0', at: '14:02' }] },
        { kind: 'md', variant: 'readable', versions: [{ v: 1, hash: 'sha256:d912…4a', size: '9.1 KB', producer: 'adapter/readability-md v1.4.0', at: '14:02', text: [
          '今日趋势以本地优先与小型推理运行时为主。前 12 个仓库中，有 5 个在最近 24 小时内新增超过 800 星。',
          '榜单页面在 13:40 后的两次抓取中命中了空壳结构，健康评估标记为可疑，选择器未命中计数为 4。'
        ] }] },
        { kind: 'struct', variant: 'hotlist.snapshot', versions: [{ v: 1, hash: 'sha256:5fa1…07', size: '3.2 KB', producer: 'adapter/trending-list v1.0.3', at: '14:02', yaml: 'snapshot_at: "2026-10-21T14:00:00+08:00"\nwindow: daily\nentries: 12\ntop_repo: "tinygrad/tinygrad"\ntop_gain: "+1,284 stars"\nselector_miss: 4' }] },
        { kind: 'links', variant: null, versions: [{ v: 2, hash: 'sha256:2b77…c9', size: '24 links', producer: 'adapter/link-discovery v1.1.0', at: '14:02', links: [
          { label: 'GitHub Trending', url: 'https://github.com/trending' },
          { label: 'tinygrad', url: 'https://github.com/tinygrad/tinygrad' }
        ] }] },
        { kind: 'embedding', variant: 'bge-m3', versions: [{ v: 1, hash: 'sha256:aa02…17', size: '1024 维 · 4 KB', producer: 'adapter/embed-bge v0.3.0', at: '14:03', dims: 1024 }] }
      ]
    },
    {
      rid: 'rid:01JA7G3C9V5M1Q7W3E8R2T6Y0U',
      title: '为什么我们放弃微服务，回到了单体架构',
      source: 'source/sspai', sourceName: '少数派', updated: '13:57', url: 'https://sspai.com/post/102931',
      identity: ['url:https://sspai.com/post/102931', 'ext:sspai/102931'],
      relations: [{ rel: 'discovered_by', dir: 'out', target: 'source/sspai' }, { rel: 'duplicate_of', dir: 'out', target: 'rid:01JA7G3C…mirror' }],
      facets: [
        { kind: 'raw', variant: null, versions: [{ v: 2, hash: 'sha256:0cc7…88', size: '168 KB', producer: 'accessor/rss v2.3.0', at: '13:57' }] },
        { kind: 'md', variant: 'readable', versions: [{ v: 2, hash: 'sha256:73b2…c6', size: '14.2 KB', producer: 'adapter/readability-md v1.4.0', at: '13:57', text: [
          '团队最初按业务域拆成 11 个服务，两年后把其中 9 个合并回单一部署单元。作者把原因归结为组织规模与调用链成本，而不是微服务本身的缺陷。',
          '迁移后，发布频率没有下降，但故障定位时间从平均 40 分钟缩短到 12 分钟。文章也列出了不适合回退的场景：独立扩缩容与合规隔离。'
        ] }] },
        { kind: 'struct', variant: 'article', versions: [{ v: 1, hash: 'sha256:e1f4…29', size: '2.0 KB', producer: 'adapter/jsonld-struct v0.9.1', at: '13:57', yaml: 'title: "为什么我们放弃微服务，回到了单体架构"\nauthor: "少数派编辑部"\npublished_at: "2026-10-21T11:20:00+08:00"\ncanonical_url: "https://sspai.com/post/102931"\nlanguage: zh-CN\ntags: [engineering, architecture]' }] },
        { kind: 'links', variant: null, versions: [{ v: 1, hash: 'sha256:a17e…5b', size: '14 links', producer: 'adapter/link-discovery v1.1.0', at: '13:57', links: [
          { label: '少数派原文', url: 'https://sspai.com/post/102931' },
          { label: '相关讨论', url: 'https://news.ycombinator.com/' }
        ] }] },
        { kind: 'embedding', variant: 'bge-m3', versions: [{ v: 1, hash: 'sha256:9d30…e2', size: '1024 维 · 4 KB', producer: 'adapter/embed-bge v0.3.0', at: '13:58', dims: 1024 }] }
      ]
    }
  ];

  const EGRESS = [
    { domain: 'reuters.com', rate: '6 / min', concurrency: '1', circuit: 'open', cooldown: '12m 40s', throttle: '—', error: '18.2%', robots: 'obey-with-exemptions', last: '403 · 14:18', sessions: '会话池 2 · 1 退役', identity: 'ModalPrismBot/1.2', note: '主机熔断中；连续 6 次 403。冷却时间按 2× 递增，下一次探测在 12 分钟后。' },
    { domain: 'arxiv.org', rate: '20 / min', concurrency: '2', circuit: 'half-open', cooldown: '4m 12s', throttle: '-32%', error: '3.1%', robots: 'api 豁免', last: '503 · 14:18', sessions: '无会话 · API', identity: 'ModalPrismBot/1.2', note: '半开状态，允许单个探测请求。自适应节流依据 429/503 比例下调 32%。' },
    { domain: 'weibo.com', rate: '4 / min', concurrency: '1', circuit: 'half-open', cooldown: '2m 05s', throttle: '-48%', error: '5.6%', robots: 'obey', last: '429 · 14:12', sessions: '会话池 1 · UA 轮换关闭', identity: 'ModalPrismBot/1.2', note: '仅访问公开页面；会话不轮换，限速按注册域计算。' },
    { domain: 'api.github.com', rate: '30 / min', concurrency: '4', circuit: 'closed', cooldown: '—', throttle: '-8%', error: '0.4%', robots: 'api 豁免', last: '200 · 14:31', sessions: '令牌桶共享', identity: 'ModalPrismBot/1.2', note: '使用官方 API 使用条款允许的请求身份；预算未启用。' },
    { domain: 'news.ycombinator.com', rate: '30 / min', concurrency: '4', circuit: 'closed', cooldown: '—', throttle: '0%', error: '0.0%', robots: 'api 豁免', last: '200 · 14:31', sessions: '无会话 · API', identity: 'ModalPrismBot/1.2', note: 'Firebase API；付费调用治理未启用，逻辑请求键按窗口合并。' },
    { domain: 'sspai.com', rate: '8 / min', concurrency: '2', circuit: 'closed', cooldown: '—', throttle: '+4%', error: '0.2%', robots: 'feed 豁免', last: '200 · 14:30', sessions: '会话池 2', identity: 'ModalPrismBot/1.2', note: 'Feed 地址由页面 <link rel=alternate> 公开声明，豁免理由写入审计。' },
    { domain: 'github.com', rate: '12 / min', concurrency: '3', circuit: 'closed', cooldown: '—', throttle: '0%', error: '0.6%', robots: 'obey', last: '200 · 14:02', sessions: '会话池 3 · 1 冷却', identity: 'ModalPrismBot/1.2', note: '浏览器子请求同样经过出站治理；robots.txt 缓存 6 小时。' },
    { domain: 'huggingface.co', rate: '12 / min', concurrency: '2', circuit: 'closed', cooldown: '—', throttle: '0%', error: '0.1%', robots: 'obey', last: '200 · 14:24', sessions: '无会话 · API', identity: 'ModalPrismBot/1.2', note: '模型与论文 API 共用域速率；预算按日窗口聚合。' }
  ];

  const ROBOTS = [
    { source: 'source/sspai', category: 'feed', reason: '站点首页 <link rel="alternate" type="application/rss+xml"> 公开声明的 Feed 地址。', last: '14:30', audit: 'AUD-2041' },
    { source: 'source/hn-front', category: 'api', reason: '官方 Firebase API，公开使用条款允许的程序化读取。', last: '14:31', audit: 'AUD-2038' },
    { source: 'source/arxiv-cl', category: 'api', reason: 'arXiv API 使用条款允许的批量检索；仅访问摘要与元数据。', last: '14:18', audit: 'AUD-2036' },
    { source: 'source/reuters-world', category: 'feed', reason: '公开 RSS 地址；正文抓取不豁免，仍受 robots 与出站治理约束。', last: '14:18', audit: 'AUD-2034' }
  ];

  const BUDGETS = [
    { name: '搜索 API · SearchAPI', spend: 62.4, limit: 120, window: '24h 窗口 · 6h 12m 后重置', tone: 'warn' },
    { name: '渲染集群 · moli', spend: 28.1, limit: 60, window: '24h 窗口 · 9h 40m 后重置', tone: '' },
    { name: '模型抽取 · embed-bge', spend: 6.4, limit: 40, window: '24h 窗口 · 14h 后重置', tone: '' }
  ];

  const RECEIPTS = [
    { task: 'task-01JA7K…/attempt-1', service: 'SearchAPI', state: '完成', amount: '¥1.20', at: '14:18' },
    { task: 'task-01JA7J…/attempt-2', service: 'SearchAPI', state: '未知', amount: '¥0.80', at: '13:44' },
    { task: 'task-01JA7J…/attempt-1', service: 'moli render', state: '完成', amount: '¥0.24', at: '13:41' }
  ];

  const LADDER = [
    { name: '直连', count: 5, width: 100, note: 'HTTP，无脚本' },
    { name: '会话', count: 3, width: 62, note: 'Cookie · 条件请求' },
    { name: '结构', count: 2, width: 44, note: '执行脚本，仅 DOM' },
    { name: '交互', count: 1, width: 28, note: '布局 · 网络捕获' },
    { name: '兼容', count: 1, width: 16, note: '回退内核' }
  ];

  const PENDING = {
    key: 'pending',
    id: 'G-38',
    at: '待校验',
    pending: true,
    files: 174,
    note: 'sources/arxiv.yaml · 节奏下限 20m → 15m',
    fileMap: {
      'sources/arxiv.yaml': {
        status: 'modified', add: 1, del: 1,
        lines: [
          { t: 'ctx', n: 17, text: '  policy:' },
          { t: 'ctx', n: 18, text: '    preset: archive' },
          { t: 'ctx', n: 19, text: '  schedule:' },
          { t: 'ctx', n: 20, text: '    adaptive: true' },
          { t: 'del', n: 21, text: '    floor: 20m' },
          { t: 'add', n: 21, text: '    floor: 15m' },
          { t: 'ctx', n: 22, text: '    ceiling: 2h' },
          { t: 'ctx', n: 23, text: '  admission:' },
          { t: 'ctx', n: 24, text: '    initialBackfill: { limit: 30 }' }
        ]
      },
      'system.yaml': {
        status: 'unchanged', add: 0, del: 0,
        lines: [
          { t: 'ctx', n: 34, text: 'kind: system' },
          { t: 'ctx', n: 35, text: 'policy:' },
          { t: 'ctx', n: 36, text: '  preset: standard' },
          { t: 'ctx', n: 37, text: '  locks: [robots.mode, egress.private_networks]' }
        ]
      },
      'presets/archive.yaml': {
        status: 'unchanged', add: 0, del: 0,
        lines: [
          { t: 'ctx', n: 4, text: 'id: preset/archive' },
          { t: 'ctx', n: 5, text: 'extends: [preset/standard]' },
          { t: 'ctx', n: 6, text: 'policy:' },
          { t: 'ctx', n: 7, text: '  raw: { store: all, retain: forever }' }
        ]
      }
    }
  };

  const GENS = [
    {
      key: 'G-37', id: 'G-37', at: '13:58', files: 174, note: 'reuters · 渲染上限',
      fileMap: {
        'sources/reuters.yaml': {
          status: 'modified', add: 1, del: 1,
          lines: [
            { t: 'ctx', n: 9, text: '  policy:' },
            { t: 'ctx', n: 10, text: '    preset: standard' },
            { t: 'ctx', n: 11, text: '    render:' },
            { t: 'del', n: 12, text: '      max: structure' },
            { t: 'add', n: 12, text: '      max: interact' },
            { t: 'ctx', n: 13, text: '  access:' },
            { t: 'ctx', n: 14, text: '    accessor: accessor/browser' }
          ]
        },
        'system.yaml': {
          status: 'unchanged', add: 0, del: 0,
          lines: [
            { t: 'ctx', n: 34, text: 'kind: system' },
            { t: 'ctx', n: 35, text: 'policy:' },
            { t: 'ctx', n: 36, text: '  preset: standard' },
            { t: 'ctx', n: 37, text: '  locks: [robots.mode, egress.private_networks]' }
          ]
        }
      }
    },
    {
      key: 'G-36', id: 'G-36', at: '11:20', files: 173, note: 'preset · 预算窗口',
      fileMap: {
        'presets/standard.yaml': {
          status: 'modified', add: 1, del: 1,
          lines: [
            { t: 'ctx', n: 22, text: 'policy:' },
            { t: 'ctx', n: 23, text: '  paid:' },
            { t: 'del', n: 24, text: '    budget_window: 24h' },
            { t: 'add', n: 24, text: '    budget_window: 12h' },
            { t: 'ctx', n: 25, text: '    unknown_result: cooldown_once' }
          ]
        },
        'system.yaml': {
          status: 'unchanged', add: 0, del: 0,
          lines: [
            { t: 'ctx', n: 34, text: 'kind: system' },
            { t: 'ctx', n: 35, text: 'policy:' },
            { t: 'ctx', n: 36, text: '  preset: standard' }
          ]
        }
      }
    },
    {
      key: 'G-35', id: 'G-35', at: '09:04', files: 171, note: '锁 · 私网策略',
      fileMap: {
        'system.yaml': {
          status: 'modified', add: 1, del: 1,
          lines: [
            { t: 'ctx', n: 34, text: 'kind: system' },
            { t: 'ctx', n: 35, text: 'policy:' },
            { t: 'ctx', n: 36, text: '  preset: standard' },
            { t: 'del', n: 37, text: '  locks: [robots.mode]' },
            { t: 'add', n: 37, text: '  locks: [robots.mode, egress.private_networks]' },
            { t: 'ctx', n: 38, text: '  egress:' },
            { t: 'ctx', n: 39, text: '    private_networks: deny' }
          ]
        },
        'presets/standard.yaml': {
          status: 'unchanged', add: 0, del: 0,
          lines: [
            { t: 'ctx', n: 22, text: 'policy:' },
            { t: 'ctx', n: 23, text: '  paid:' },
            { t: 'ctx', n: 24, text: '    budget_window: 12h' }
          ]
        }
      }
    }
  ];

  const PROV = [
    { key: 'policy.preset', value: 'standard', file: 'system.yaml:34', chain: ['通用配置 · defaults', 'preset/standard · presets/standard.yaml', 'system.yaml:34 · 最终生效'] },
    { key: 'policy.raw.retain', value: '180d', file: 'sources/arxiv.yaml:18', chain: ['preset/archive · presets/archive.yaml', 'profile/longform · profiles/longform.yaml', 'sources/arxiv.yaml:18 · 180d'] },
    { key: 'policy.robots.mode', value: 'obey-with-exemptions', file: 'system.yaml:41', chain: ['系统级锁定 · 不可被模块覆盖', 'system.yaml:41 · obey-with-exemptions'], locked: true },
    { key: 'schedule.floor', value: '15m', file: 'sources/arxiv.yaml:21', chain: ['类型基础配置 · 20m', 'profile/api-shared · 20m', 'sources/arxiv.yaml:21 · 15m'] },
    { key: 'render.max', value: '交互', file: 'policy/render', chain: ['系统锁定 · policy.render.max', 'reuters 请求兼容 · 被裁剪为交互'], locked: true },
    { key: 'paid.budget.daily', value: '¥120', file: 'system.yaml:66', chain: ['preset/standard · ¥80', 'system.yaml:66 · ¥120'] }
  ];

  const state = {
    route: 'overview',
    range: '24h',
    sourceFilter: 'all',
    sourceQuery: '',
    resourceId: RESOURCES[0].rid,
    resourceQuery: '',
    facetKey: 'md',
    versionIndex: null,
    previewTab: 'readable',
    expandedDomain: null,
    selectedGen: 'pending',
    selectedFile: 'sources/arxiv.yaml',
    selectedProv: 'policy.raw.retain',
    asOf: SNAPSHOTS.length - 1,
    pending: PENDING,
    currentGen: 'G-37',
    sourceTab: 'pace',
    drawer: null,
    confirmHandler: null,
    paletteIndex: 0,
    paletteFiltered: [],
    lastCheck: '13:58'
  };

  const isHistorical = () => state.asOf < SNAPSHOTS.length - 1;
  const currentSnap = () => SNAPSHOTS[state.asOf];
  const sortedSources = () => [...SOURCES].sort((a, b) => SEVERITY[a.status] - SEVERITY[b.status] || a.name.localeCompare(b.name));
  const statusPill = (status) => `<span class="status" data-status="${status}"><span class="status-mark" aria-hidden="true"></span>${STATUS[status] ? STATUS[status].label : status}</span>`;

  function sparkline(values, color, w = 76, h = 22) {
    const max = Math.max(...values, 1);
    const min = Math.min(...values, 0);
    const span = max - min || 1;
    const pts = values.map((v, i) => [Number((i * (w / (values.length - 1))).toFixed(1)), Number((h - 3 - ((v - min) / span) * (h - 6)).toFixed(1))]);
    const d = pts.map((p, i) => `${i ? 'L' : 'M'}${p[0]} ${p[1]}`).join(' ');
    const last = pts[pts.length - 1];
    return `<svg class="row-spark" viewBox="0 0 ${w} ${h}" aria-hidden="true"><path class="spark-fill" d="${d} L${w} ${h} L0 ${h} Z" fill="${color}"/><path class="spark-line" d="${d}" stroke="${color}"/><circle cx="${last[0]}" cy="${last[1]}" r="1.6" fill="${color}"/></svg>`;
  }

  function addCommit(title, kind = 'ingest', producer = 'ingest/runner v0.8.2') {
    commitSeq += 1;
    const hash = (0x9e3779 + commitSeq * 2654435).toString(16).slice(-6);
    const commit = { hash, snap: currentSnap().id, kind, title, time: nowHMS(), producer };
    COMMITS.unshift(commit);
    return commit;
  }

  function sourceRow(s) {
    const color = STATUS[s.status] ? STATUS[s.status].color : '#7D837A';
    return `<tr class="is-clickable" data-action="open-source" data-id="${esc(s.id)}">
      <td class="cell-lead" data-label="信源">
        <div class="cell-main">
          <span class="cell-name"><button class="row-link" data-action="open-source" data-id="${esc(s.id)}">${esc(s.name)}</button> <span class="tag tag-muted">${ACCESSOR[s.accessor] || s.accessor}</span></span>
          <span class="cell-id">${esc(s.id)}</span>
        </div>
      </td>
      <td data-label="健康">
        ${statusPill(s.status)}
        ${s.failures ? `<div class="cell-sub">连续 ${s.failures} 次 · ${esc(s.failure)}</div>` : (s.selectorMiss ? `<div class="cell-sub">选择器未命中 ${s.selectorMiss} 次</div>` : '')}
      </td>
      <td data-label="产出趋势">
        <div class="trend-cell">${sparkline(s.spark, color)}<span class="trend-val">${s.trend.toFixed(2)}×</span></div>
      </td>
      <td data-label="节奏"><div class="cell-main"><span class="cell-strong">${esc(s.interval)}</span><span class="cell-sub">${esc(s.next)}</span></div></td>
      <td data-label="渲染"><span class="tag">${esc(s.render)}</span></td>
      <td data-label="最近成功"><span class="cell-sub">${esc(s.lastOk)}</span></td>
    </tr>`;
  }

  function metricMarkup(label, value, unit, sub, tone = '') {
    return `<div class="metric">
      <div class="metric-label"><span>${label}</span></div>
      <div class="metric-value">${value}${unit ? `<span class="unit">${unit}</span>` : ''}</div>
      <div class="metric-sub ${tone}">${sub}</div>
    </div>`;
  }

  function chartModel() {
    const range = state.range;
    const bins = range === '6h' ? 12 : range === '7d' ? 28 : 24;
    const scale = currentSnap().scale;
    const base = range === '7d' ? 1480 : 640;
    const series = [];
    for (let i = 0; i < bins; i += 1) {
      const wave = 0.38 + 0.4 * Math.sin((i / bins) * Math.PI * 1.8 + 0.5) + 0.14 * Math.sin(i * 2.1 + state.asOf);
      const total = Math.max(32, Math.round(base * (0.52 + wave) * scale));
      const raw = Math.round(total * 0.46);
      const md = Math.round(total * 0.31);
      const struct = Math.round(total * 0.15);
      const emb = Math.max(1, total - raw - md - struct);
      series.push({ raw, md, struct, emb, total });
    }
    const totals = series.reduce((a, d) => {
      a.raw += d.raw; a.md += d.md; a.struct += d.struct; a.emb += d.emb; a.all += d.total;
      return a;
    }, { raw: 0, md: 0, struct: 0, emb: 0, all: 0 });
    const maxTotal = Math.max(...series.map((d) => d.total));
    const step = Math.ceil(maxTotal / 4 / 50) * 50 || 50;
    return { series, totals, bins, max: step * 4, range };
  }

  function binLabel(range, i, bins) {
    if (range === '6h') {
      const m = 9 * 60 + i * 30;
      return `${pad(Math.floor(m / 60))}:${pad(m % 60)}`;
    }
    if (range === '24h') {
      const h = (14 - (bins - 1 - i) + 24) % 24;
      return `${pad(h)}:00`;
    }
    const day = 15 + Math.floor(i / 4);
    return `10/${pad(day)}`;
  }

  function chartSvg() {
    const model = chartModel();
    const W = 720; const H = 242; const padL = 36; const padR = 10; const padT = 14; const padB = 24;
    const plotW = W - padL - padR; const plotH = H - padT - padB;
    const slot = plotW / model.bins; const barW = Math.max(3, slot - 3);
    const colors = { raw: '#5F6E64', md: '#A6C09A', struct: '#D6A24A', emb: '#E2552F' };
    let grid = '';
    for (let i = 0; i <= 4; i += 1) {
      const y = padT + (plotH / 4) * i;
      const v = Math.round(model.max - (model.max / 4) * i);
      grid += `<line class="chart-grid-line" x1="${padL}" y1="${y}" x2="${W - padR}" y2="${y}"/><text class="chart-axis-label" x="${padL - 7}" y="${y + 3}" text-anchor="end">${v >= 1000 ? `${(v / 1000).toFixed(1)}k` : v}</text>`;
    }
    let bars = '';
    let hits = '';
    const tickEvery = model.range === '24h' ? 4 : model.range === '6h' ? 2 : 7;
    model.series.forEach((d, i) => {
      const x = padL + i * slot + (slot - barW) / 2;
      let y = padT + plotH;
      const parts = [['raw', d.raw], ['md', d.md], ['struct', d.struct], ['emb', d.emb]];
      parts.forEach(([k, v]) => {
        const h = (v / model.max) * plotH;
        y -= h;
        if (h > 1) bars += `<rect x="${x.toFixed(1)}" y="${y.toFixed(1)}" width="${barW.toFixed(1)}" height="${Math.max(0.8, h - 0.8).toFixed(1)}" fill="${colors[k]}"/>`;
      });
      hits += `<rect class="chart-hit" data-idx="${i}" x="${(padL + i * slot).toFixed(1)}" y="${padT}" width="${slot.toFixed(1)}" height="${plotH}"><title>${binLabel(model.range, i, model.bins)} · ${fmt(d.total)} 条 facet</title></rect>`;
      if (i % tickEvery === 0 || i === model.bins - 1) {
        bars += `<text class="chart-axis-label" x="${(x + barW / 2).toFixed(1)}" y="${H - 7}" text-anchor="middle">${binLabel(model.range, i, model.bins)}</text>`;
      }
    });
    const nowX = padL + plotW;
    return `<svg class="chart-svg" viewBox="0 0 ${W} ${H}" role="img" aria-label="采集脉冲柱状图，${state.range === '6h' ? '近 6 小时' : state.range === '24h' ? '近 24 小时' : '近 7 天'}共 ${fmt(model.totals.all)} 条 facet 产出，悬停查看分时构成。">
      ${grid}
      <line class="chart-now" x1="${nowX}" y1="${padT}" x2="${nowX}" y2="${padT + plotH}"/>
      <text class="chart-now-label" x="${nowX - 4}" y="${padT + 12}" text-anchor="end">${isHistorical() ? currentSnap().at : 'NOW'}</text>
      ${bars}
      ${hits}
    </svg>`;
  }

  function chartMarkup() {
    const m = chartModel();
    const rangeText = state.range === '6h' ? '近 6 小时' : state.range === '24h' ? '近 24 小时' : '近 7 天';
    return `<div class="chart-panel">
      <div class="chart-top">
        <div>
          <div class="chart-total-label">FACET 产出 · ${rangeText}</div>
          <div class="chart-total">${fmt(m.totals.all)}<small>条</small></div>
        </div>
        <span class="tag tag-muted">${isHistorical() ? `截至 ${currentSnap().at}` : '实时写入'}</span>
      </div>
      <div class="chart-wrap">
        ${chartSvg()}
        <div class="chart-tip" id="chartTip"></div>
      </div>
      <div class="chart-legend">
        <span class="legend-item"><i class="swatch" style="background:#5F6E64"></i>原始信封 <b>${fmt(m.totals.raw)}</b></span>
        <span class="legend-item"><i class="swatch" style="background:#A6C09A"></i>Markdown <b>${fmt(m.totals.md)}</b></span>
        <span class="legend-item"><i class="swatch" style="background:#D6A24A"></i>结构化 <b>${fmt(m.totals.struct)}</b></span>
        <span class="legend-item"><i class="swatch" style="background:#E2552F"></i>向量 <b>${fmt(m.totals.emb)}</b></span>
      </div>
    </div>`;
  }

  function queuesMarkup(snap) {
    const queues = [
      { name: '免费队列', used: Math.round(snap.tasks * 0.25), cap: 48, note: 'feed · api · 文件回放' },
      { name: '付费队列', used: 8, cap: 12, note: 'SearchAPI · 结果未知保护' },
      { name: '渲染队列', used: 3, cap: 6, note: 'moli · 单任务超时 90s' },
      { name: '推送队列', used: 0, cap: 4, note: 'webhook · 幂等键去重' }
    ];
    return `<div class="queue-list">
      ${queues.map((q) => {
        const pct = Math.round((q.used / q.cap) * 100);
        const tone = pct >= 85 ? 'is-hot' : pct >= 65 ? 'is-warn' : '';
        return `<div class="queue-row">
          <div class="queue-line"><span class="queue-name"><span class="dot-mini"></span>${q.name}</span><span class="queue-nums"><strong>${q.used}</strong> <span class="cap">/ ${q.cap}</span></span></div>
          <div class="meter ${tone}" role="progressbar" aria-label="${q.name}" aria-valuemin="0" aria-valuemax="${q.cap}" aria-valuenow="${q.used}"><i style="width:${pct}%"></i></div>
          <div class="queue-foot"><span>${q.note}</span><span>${pct}%</span></div>
        </div>`;
      }).join('')}
    </div>
    <div class="queue-foot"><span>合并分组 feed-tier1 · 4 成员</span><span>付费预算窗口 6h 12m</span></div>`;
  }

  function strataMarkup(snap) {
    const total = snap.resources;
    const values = [
      { name: '原始信封', key: 'raw', value: Math.round(total * 0.32), bg: '#232A25' },
      { name: 'facet 层', key: 'facet', value: Math.round(total * 0.53), bg: '#2E3A31' },
      { name: '资源', key: 'resource', value: Math.round(total * 0.1), bg: '#3C3428' },
      { name: '类型投影', key: 'type', value: total - Math.round(total * 0.95), bg: '#3E2A23' }
    ];
    return `<div class="strata-stack" role="group" aria-label="数据层积构成">
      ${values.map((v) => `<button class="strata-seg" style="flex:${v.value} 1 0;--seg-bg:${v.bg}" data-route="strata" aria-label="${v.name} ${fmt(v.value)}，查看 Strata 资源">
        <span class="seg-name">${v.name}</span><span class="seg-val">${fmt(v.value)}</span>
      </button>`).join('')}
    </div>
    <div class="strata-legend">
      <span>只追加</span><span>版本不可变</span><span>按快照可见</span><span>内容哈希去重</span>
    </div>`;
  }

  function commitsMarkup(limit = 6) {
    const rows = COMMITS.filter((c) => Number(c.snap) <= Number(currentSnap().id)).slice(0, limit);
    if (!rows.length) return `<div class="empty-state"><strong>该快照没有提交</strong>更早的快照可能尚未产生可见变更。</div>`;
    return `<ul class="commit-list">${rows.map((c) => `<li>
      <button class="commit" data-action="open-commit" data-hash="${c.hash}" data-kind="${c.kind}">
        <span class="commit-mark" aria-hidden="true"></span>
        <span class="commit-hash">${c.hash}</span>
        <span class="commit-title">${esc(c.title)}</span>
        <span class="commit-meta">${c.time}<br>#${c.snap}</span>
      </button></li>`).join('')}</ul>`;
  }

  function renderOverview() {
    const snap = currentSnap();
    const healthy = isHistorical() ? snap.healthy : SOURCES.filter((s) => s.status === 'ok').length;
    const unhealthy = SOURCES.length - healthy;
    const rangeButtons = [['6h', '6h'], ['24h', '24h'], ['7d', '7d']].map(([v, l]) => `<button data-action="chart-range" data-range="${v}" aria-pressed="${state.range === v}">${l}</button>`).join('');
    return `<header class="page-head">
      <div>
        <div class="page-kicker"><span class="kicker-bar" aria-hidden="true"><i></i><i></i><i></i></span>L1 · OBSERVATORY</div>
        <h1 class="page-title">系统正在沉积</h1>
        <p class="page-desc">配置代际 <span class="mono">${state.currentGen}</span> · 快照 <span class="mono">${snap.label}</span> · 环回入口 <span class="mono">127.0.0.1:7420</span>。每一次读取都有快照，每一次写入都是提交。</p>
      </div>
      <div class="page-head-actions">
        <button class="btn btn-quiet" data-route="config">${icon('config', 14)}配置溯源</button>
        <button class="btn btn-primary" data-action="run-scan" ${isHistorical() ? 'disabled' : ''}>${icon('refresh', 14)}运行一轮</button>
      </div>
    </header>
    <section class="metrics-band" aria-label="核心指标">
      ${metricMarkup('资源总量', fmt(snap.resources), '', `<span class="delta-up">+${fmt(snap.added)}</span> 今日`)}
      ${metricMarkup('今日新增', fmt(snap.added), '', `较昨日 <span class="delta-up">${snap.daily}</span>`)}
      ${metricMarkup('待处理任务', fmt(snap.tasks), '', '4 个队列 · 租约 90s')}
      ${metricMarkup('出站请求', fmt(snap.egress), '/min', '上限 60 /min · 自适应')}
      ${metricMarkup('健康信源', String(healthy), `/${SOURCES.length}`, `<span class="${unhealthy ? 'delta-warn' : 'delta-up'}">${unhealthy}</span> 个非正常`)}
      ${metricMarkup('本窗口成本', `¥${snap.cost.toFixed(2)}`, '', '预算 ¥120 · 24h')}
    </section>
    <div class="grid-main">
      <section class="block" aria-label="采集脉冲">
        <div class="block-head">
          <div class="block-title"><span class="block-kicker">L1 · INGEST PULSE</span>采集脉冲</div>
          <div class="segmented" role="group" aria-label="时间范围">${rangeButtons}</div>
        </div>
        ${chartMarkup()}
      </section>
      <section class="block" aria-label="运行队列">
        <div class="block-head"><div class="block-title"><span class="block-kicker">QUEUES</span>运行队列</div><span class="block-note">租约 90s · 扫描 5s</span></div>
        ${queuesMarkup(snap)}
      </section>
    </div>
    <div class="grid-secondary">
      <section class="block" aria-label="层积构成">
        <div class="block-head"><div class="block-title"><span class="block-kicker">STRATA</span>层积构成</div><span class="block-note">按资源 ID 锚定</span></div>
        ${strataMarkup(snap)}
      </section>
      <section class="block" aria-label="最近提交">
        <div class="block-head"><div class="block-title"><span class="block-kicker">COMMITS</span>最近提交</div><span class="block-note">按快照 ${snap.label} 过滤</span></div>
        ${commitsMarkup(6)}
      </section>
    </div>
    <section class="block grid-full" aria-label="信源健康">
      <div class="block-head"><div class="block-title"><span class="block-kicker">HEALTH</span>信源健康</div><button class="ghost-link" data-route="sources">全部 ${SOURCES.length} 个信源 ${icon('arrow-right', 11)}</button></div>
      <div class="table-scroll">
        <table class="data-table source-table">
          <thead><tr><th>信源</th><th>健康</th><th>产出趋势</th><th>节奏</th><th>渲染</th><th>最近成功</th></tr></thead>
          <tbody>${sortedSources().slice(0, 6).map(sourceRow).join('')}</tbody>
        </table>
      </div>
    </section>`;
  }

  function filteredSources() {
    const q = state.sourceQuery.trim().toLowerCase();
    return sortedSources().filter((s) => {
      if (state.sourceFilter !== 'all' && s.status !== state.sourceFilter) return false;
      if (!q) return true;
      return `${s.name} ${s.id} ${s.desc} ${s.accessor}`.toLowerCase().includes(q);
    });
  }

  function sourceFiltersMarkup() {
    const counts = SOURCES.reduce((a, s) => { a[s.status] = (a[s.status] || 0) + 1; return a; }, {});
    const items = [['all', '全部', SOURCES.length]];
    ['ok', 'degraded', 'suspect', 'failing', 'misconfigured', 'paused'].forEach((k) => {
      if (counts[k]) items.push([k, STATUS[k].label, counts[k]]);
    });
    return items.map(([key, label, n]) => `<button class="chip" data-action="source-filter" data-value="${key}" aria-pressed="${state.sourceFilter === key}">${label}<span class="chip-n">${n}</span></button>`).join('');
  }

  function renderSources() {
    const rows = filteredSources();
    return `<header class="page-head">
      <div>
        <div class="page-kicker"><span class="kicker-bar" aria-hidden="true"><i></i><i></i><i></i></span>L1 · SOURCES</div>
        <h1 class="page-title">信源</h1>
        <p class="page-desc">健康不是布尔值。状态来自错误类别、选择器命中、拦截特征、产出自适应与渲染级别，反馈给调度器决定节奏与熔断。</p>
      </div>
      <div class="page-head-actions">
        <button class="btn btn-quiet" data-route="governance">${icon('governance', 14)}出站治理</button>
        <button class="btn btn-primary" data-action="run-scan" ${isHistorical() ? 'disabled' : ''}>${icon('refresh', 14)}运行到期任务</button>
      </div>
    </header>
    <div class="summary-strip">
      <span class="summary-item"><strong>${SOURCES.length}</strong> 个信源</span>
      <span class="summary-item"><strong>${SOURCES.filter((s) => s.status === 'ok').length}</strong> 正常</span>
      <span class="summary-item"><strong>${SOURCES.filter((s) => s.preset === 'archive').length}</strong> archive 预设</span>
      <span class="summary-item"><strong>4</strong> 队列 · 租约 90s</span>
      <span class="summary-split">上次全局扫描 14:30:12 · 配置代际 ${state.currentGen}</span>
    </div>
    <div class="toolbar">
      <label class="field" for="sourceSearch">${icon('search', 13)}<input id="sourceSearch" type="search" placeholder="搜索名称或 ID…" aria-label="搜索信源名称或 ID" autocomplete="off" value="${esc(state.sourceQuery)}"></label>
      <div class="chip-row">${sourceFiltersMarkup()}</div>
      <div class="toolbar-right"><span class="toolbar-note" id="sourceFilteredCount" aria-live="polite">${rows.length} 个信源</span></div>
    </div>
    <div class="table-scroll">
      <table class="data-table source-table">
        <thead><tr><th>信源</th><th>健康</th><th>产出趋势</th><th>节奏</th><th>渲染</th><th>最近成功</th></tr></thead>
        <tbody id="sourceRows">${rows.length ? rows.map(sourceRow).join('') : `<tr><td colspan="6"><div class="empty-state"><strong>没有匹配的信源</strong>调整关键词或状态筛选。</div></td></tr>`}</tbody>
      </table>
    </div>
    <p class="hint-inline">选择信源可查看节奏推导、健康信号、最近任务与只读配置。</p>`;
  }

  function activeResource() {
    return RESOURCES.find((r) => r.rid === state.resourceId) || RESOURCES[0];
  }

  function activeFacet() {
    const res = activeResource();
    return res.facets.find((f) => f.kind === state.facetKey) || res.facets[0];
  }

  function activeVersion(facet, idx = state.versionIndex) {
    const vi = idx == null ? facet.versions.length - 1 : Math.min(idx, facet.versions.length - 1);
    return facet.versions[vi];
  }

  function resourceItem(r) {
    const kinds = r.facets.map((f) => f.kind);
    const dots = ['raw', 'md', 'struct', 'embedding'].map((k) => `<i class="facet-dot ${k}" style="opacity:${kinds.includes(k) ? 1 : 0.18}"></i>`).join('');
    return `<button class="resource-item" data-action="select-resource" data-rid="${esc(r.rid)}" aria-pressed="${r.rid === state.resourceId}">
      <div class="resource-title">${esc(r.title)}</div>
      <div class="resource-meta"><span class="facet-count">${dots}</span><span>${esc(r.sourceName)}</span><span class="sep">/</span><span>${r.facets.length} facet</span><span class="sep">/</span><span>${esc(r.updated)}</span></div>
    </button>`;
  }

  function filteredResources() {
    const q = state.resourceQuery.trim().toLowerCase();
    if (!q) return RESOURCES;
    return RESOURCES.filter((r) => `${r.title} ${r.rid} ${r.sourceName} ${r.url}`.toLowerCase().includes(q));
  }

  function facetRailHtml(res) {
    return res.facets.map((f) => {
      const latest = f.versions[f.versions.length - 1];
      const label = f.variant ? `${f.kind}:${f.variant}` : f.kind;
      return `<button class="facet-btn" data-action="select-facet" data-facet="${f.kind}" aria-pressed="${f.kind === state.facetKey}"><b>${esc(label)}</b><span>v${latest.v}</span></button>`;
    }).join('');
  }

  function versionStripHtml(facet) {
    return `<div class="version-strip">
      <span class="vs-label">版本链 · ${facet.variant ? `${facet.kind}:${facet.variant}` : facet.kind}</span>
      ${facet.versions.map((v, i) => `<button class="version-btn" data-action="select-version" data-version="${i}" aria-pressed="${(state.versionIndex == null ? facet.versions.length - 1 : state.versionIndex) === i}">v${v.v}</button>`).join('<span class="version-hash">→</span>')}
      <span class="version-hash">${esc(activeVersion(facet).hash)} · ${esc(activeVersion(facet).producer)}</span>
    </div>`;
  }

  function previewHtml(res) {
    const facet = activeFacet();
    const ver = activeVersion(facet);
    const tabs = [['readable', 'md', '正文'], ['struct', 'struct', '结构化'], ['raw', 'raw', '原始']];
    let body = '';
    if (facet.kind === 'md') {
      body = (ver.text || []).map((p) => `<p>${esc(p)}</p>`).join('');
    } else if (facet.kind === 'struct') {
      body = `<pre class="preview-json">${esc(ver.yaml || '')}</pre>`;
    } else if (facet.kind === 'raw') {
      body = `<pre class="preview-json">${esc(rawJsonFor(res))}</pre>`;
    } else if (facet.kind === 'links') {
      const links = ver.links || [];
      body = `<div class="link-list">${links.map((l) => `<div class="link-item"><div class="link-title">${esc(l.label)}</div><div class="link-url">${esc(l.url)}</div></div>`).join('')}</div>`;
    } else {
      body = `<div class="facts"><div class="fact"><div class="fact-label">MODEL</div><div class="fact-value mono">${esc(facet.variant || '—')}</div></div><div class="fact"><div class="fact-label">DIMENSIONS</div><div class="fact-value mono">${ver.dims || '—'} 维</div></div><div class="fact"><div class="fact-label">SIZE</div><div class="fact-value mono">${esc(ver.size)}</div></div><div class="fact"><div class="fact-label">PRODUCER</div><div class="fact-value mono">${esc(ver.producer)}</div></div></div><p class="hint-inline">向量正文不在控制台展开。查询以快照为界，向量检索由 Strata 执行。</p>`;
    }
    return `<div class="preview-panel">
      <div class="preview-head">
        <div class="preview-tabs" role="tablist" aria-label="facet 预览">
          ${tabs.map(([tab, kind, label]) => `<button class="preview-tab" role="tab" data-action="preview-tab" data-tab="${tab}" data-facet="${kind}" aria-selected="${facet.kind === kind && state.previewTab === tab}">${label}</button>`).join('')}
        </div>
        <span class="preview-meta">${esc(ver.at)} · ${esc(ver.size)}</span>
      </div>
      <div class="preview-body">${body}</div>
    </div>`;
  }

  function rawJsonFor(r) {
    return JSON.stringify({
      locator: { requested: r.url, final: r.url, redirects: 0 },
      status: 200,
      headers: ['content-type: text/html; charset=utf-8', 'cache-control: max-age=300', 'etag: "8f21c4"'],
      fetched_at: '2026-10-21T14:27:51+08:00',
      parts: [
        { role: 'document', declared: 'text/html', charset: 'utf-8', bytes: 184231 },
        { role: 'network-response', declared: 'application/json', bytes: 12044 }
      ],
      candidateCursor: 'cur_8f21c4'
    }, null, 2);
  }

  function lineageSvg(res) {
    const src = SOURCES.find((s) => s.id === res.source);
    const acc = src ? src.accessor : 'http';
    const active = state.facetKey;
    const fmap = Object.fromEntries(res.facets.map((f) => [f.kind, f]));
    const last = (k) => {
      const f = fmap[k];
      if (!f) return null;
      return f.versions[f.versions.length - 1];
    };
    const raw = last('raw');
    const md = last('md');
    const links = last('links');
    const struct = last('struct');
    const emb = last('embedding');
    const nodes = [
      { id: 'accessor', label: `accessor/${acc}`, sub: 'v2.3.0', x: 4, y: 74 },
      { id: 'raw', label: 'raw', sub: raw ? `v${raw.v} · ${raw.size}` : '—', x: 148, y: 74 },
      { id: 'detect', label: 'struct:detect', sub: 'v1 · 识别共享', x: 296, y: 12 },
      { id: 'md', label: 'md:readable', sub: md ? `v${md.v} · ${md.size}` : '—', x: 296, y: 74 },
      { id: 'links', label: 'links', sub: links ? `v${links.v} · ${links.size}` : '—', x: 296, y: 136 },
      { id: 'struct', label: `struct:${struct && struct.variant ? struct.variant : 'article'}`, sub: struct ? `v${struct.v} · ${struct.size}` : '—', x: 456, y: 74 },
      { id: 'embedding', label: 'embedding:bge-m3', sub: emb ? `v${emb.v}` : '—', x: 456, y: 136 }
    ];
    const edges = [
      ['accessor', 'raw'], ['raw', 'detect'], ['raw', 'md'], ['raw', 'links'], ['md', 'struct'], ['md', 'embedding']
    ];
    const byId = Object.fromEntries(nodes.map((n) => [n.id, n]));
    let svg = '<svg class="lineage-svg" viewBox="0 0 600 190" role="img" aria-label="facet 血缘图：访问器到原始信封，再经识别与适配器派生 Markdown、结构化、链接与向量。">';
    edges.forEach(([a, b]) => {
      const from = byId[a]; const to = byId[b];
      const x1 = from.x + 128; const y1 = from.y + 17; const x2 = to.x; const y2 = to.y + 17;
      const mid = (x1 + x2) / 2;
      const isOn = a === active || b === active;
      svg += `<path class="lineage-edge ${isOn ? 'is-active' : ''}" d="M${x1} ${y1} C${mid} ${y1}, ${mid} ${y2}, ${x2 - 4} ${y2}"/><path class="lineage-arrow ${isOn ? 'is-active' : ''}" d="M${x2 - 4} ${y2 - 3} L${x2 + 1} ${y2} L${x2 - 4} ${y2 + 3} Z" style="${isOn ? 'fill:rgba(226,85,47,.7)' : ''}"/>`;
    });
    nodes.forEach((n) => {
      const isOn = n.id === active;
      svg += `<g class="lineage-node ${isOn ? 'is-active' : ''}">
        <rect x="${n.x}" y="${n.y}" width="128" height="34" rx="2"/>
        <text class="node-label" x="${n.x + 9}" y="${n.y + 14}">${esc(n.label)}</text>
        <text class="node-sub" x="${n.x + 9}" y="${n.y + 26}">${esc(n.sub)}</text>
      </g>`;
    });
    svg += '</svg>';
    return svg;
  }

  function renderStrata() {
    const res = activeResource();
    const facet = activeFacet();
    const rows = filteredResources();
    const resIndex = RESOURCES.indexOf(res);
    const commitHash = ['a1f9c2e', '8d7a330', '3c2e817', '2b6e9f1', 'e9c1a75'][resIndex] || 'a1f9c2e';
    return `<header class="page-head">
      <div>
        <div class="page-kicker"><span class="kicker-bar" aria-hidden="true"><i></i><i></i><i></i></span>L2 · STRATA</div>
        <h1 class="page-title">Strata 资源</h1>
        <p class="page-desc">原始数据与派生数据都以不透明 RID 锚定。facet 只追加，版本不可变，识别与抽取只通过区间引用表达。当前读取快照 <span class="mono">${currentSnap().label}</span>。</p>
      </div>
      <div class="page-head-actions">
        <span class="tag tag-muted">${isHistorical() ? '历史快照 · 只读' : '可回溯 · 按快照读取'}</span>
        <button class="btn btn-quiet" data-action="copy-rid">${icon('copy', 13)}复制 RID</button>
      </div>
    </header>
    <div class="strata-layout">
      <section class="resource-pane" aria-label="资源列表">
        <label class="field" for="resourceSearch">${icon('search', 13)}<input id="resourceSearch" type="search" placeholder="搜索标题、RID 或来源…" aria-label="搜索资源标题、RID 或来源" autocomplete="off" value="${esc(state.resourceQuery)}"></label>
        <div class="resource-list" id="resourceList">
          ${rows.length ? rows.map(resourceItem).join('') : `<div class="empty-state"><strong>没有匹配的资源</strong>换一个关键词，或清除搜索。</div>`}
        </div>
        <p class="hint-inline">${rows.length} / ${RESOURCES.length} 个资源 · 展示最近写入的可见快照</p>
      </section>
      <section class="detail-panel" aria-label="资源详情">
        <div class="detail-header">
          <div class="drawer-eyebrow">RESOURCE · ${esc(res.rid.split(':')[1].slice(0, 8))} · 命名空间 data</div>
          <h2 class="detail-title">${esc(res.title)}</h2>
          <div class="rid-row">
            <span class="rid">${esc(res.rid)}</span>
            <button class="icon-btn" data-action="copy-rid" aria-label="复制 RID" title="复制 RID">${icon('copy', 14)}</button>
            <span class="tag tag-muted">${esc(res.sourceName)}</span>
            <span class="tag tag-muted">更新于 ${esc(res.updated)}</span>
          </div>
        </div>
        <div class="facet-rail" role="group" aria-label="facet 类型">${facetRailHtml(res)}</div>
        ${versionStripHtml(facet)}
        ${previewHtml(res)}
        <div class="lineage-panel">
          <div class="block-head" style="margin-bottom:10px"><div class="block-title"><span class="block-kicker">LINEAGE</span>血缘</div><span class="block-note">悬停/选择节点查看高亮 · 引用输入 facet 版本</span></div>
          <div class="lineage-scroll">${lineageSvg(res)}</div>
        </div>
        <div class="detail-meta">
          <div class="fact"><div class="fact-label">来源关系</div><div class="fact-value mono">discovered_by → ${esc(res.source)}</div></div>
          <div class="fact"><div class="fact-label">墓碑</div><div class="fact-value">否 · 身份键映射保留</div></div>
          <div class="fact"><div class="fact-label">最近提交</div><div class="fact-value mono">${commitHash} · ${esc(res.updated)}</div></div>
          <div class="fact"><div class="fact-label">facet 总数</div><div class="fact-value mono">${res.facets.length} 组 · ${res.facets.reduce((a, f) => a + f.versions.length, 0)} 个版本</div></div>
        </div>
        <div class="drawer-section" style="margin-top:18px">
          <div class="drawer-section-title">身份键与关系</div>
          <div class="identity-list">${res.identity.map((k) => `<span class="identity-item">${esc(k)}</span>`).join('')}</div>
          <div class="identity-list" style="margin-top:8px">${res.relations.map((r) => `<span class="identity-item">${r.dir === 'out' ? '→' : '←'} ${esc(r.rel)} · ${esc(r.target)}</span>`).join('')}</div>
        </div>
        <p class="detail-footnote">${icon('lock', 12)} facet 不可变：内容变化产生新版本；内容哈希与最新版本相同时不产生新版本。删除以墓碑表示。</p>
      </section>
    </div>`;
  }

  function renderGovernance() {
    const openCount = EGRESS.filter((d) => d.circuit === 'open').length;
    const halfCount = EGRESS.filter((d) => d.circuit === 'half-open').length;
    const attention = openCount + halfCount;
    const attentionNames = EGRESS.filter((d) => d.circuit !== 'closed').map((d) => d.domain.split('.')[0]).join(' · ');
    const domainRows = EGRESS.map((d) => {
      const expanded = state.expandedDomain === d.domain;
      const circuitLabel = d.circuit === 'closed' ? '闭合' : d.circuit === 'half-open' ? '半开' : '熔断';
      const resettable = d.circuit !== 'closed';
      return `<tr class="domain-row" data-action="toggle-domain" data-domain="${esc(d.domain)}">
          <td class="cell-lead" data-label="域">
            <button class="domain-toggle" data-action="toggle-domain" data-domain="${esc(d.domain)}" aria-expanded="${expanded}">
              ${icon(expanded ? 'chevron-down' : 'chevron-right', 12)}
              <span class="cell-main"><span class="cell-name">${esc(d.domain)}</span><span class="cell-id">${esc(d.last)}</span></span>
            </button>
          </td>
          <td data-label="速率"><span class="cell-strong">${esc(d.rate)}</span></td>
          <td data-label="并发"><span class="cell-strong">${esc(d.concurrency)}</span></td>
          <td data-label="熔断"><span class="status" data-status="${d.circuit}"><span class="status-mark" aria-hidden="true"></span>${circuitLabel}</span><div class="cell-sub">${esc(d.cooldown)}</div></td>
          <td data-label="错误率"><span class="cell-strong ${d.error !== '0.0%' && parseFloat(d.error) > 3 ? 'delta-warn' : ''}">${esc(d.error)}</span></td>
        </tr>
        ${expanded ? `<tr class="domain-detail"><td colspan="5"><div class="domain-detail-inner">
          <div><div class="dv-label">自适应节流</div><div class="dv-value mono">${esc(d.throttle)}</div></div>
          <div><div class="dv-label">robots</div><div class="dv-value mono">${esc(d.robots)}</div></div>
          <div><div class="dv-label">会话</div><div class="dv-value mono">${esc(d.sessions)}</div></div>
          <div><div class="dv-label">请求身份</div><div class="dv-value mono">${esc(d.identity)}</div></div>
          <div class="dv-actions">
            <span class="dv-note">${esc(d.note)}</span>
            ${resettable ? `<button class="btn btn-sm btn-quiet" data-action="reset-circuit" data-domain="${esc(d.domain)}" ${isHistorical() ? 'disabled' : ''}>${icon('refresh', 12)}重置熔断</button>` : `<span class="tag tag-muted">无熔断动作</span>`}
          </div>
        </div></td></tr>` : ''}`;
    }).join('');

    return `<header class="page-head">
      <div>
        <div class="page-kicker"><span class="kicker-bar" aria-hidden="true"><i></i><i></i><i></i></span>L3 · GOVERNANCE</div>
        <h1 class="page-title">治理</h1>
        <p class="page-desc">所有出站请求都经过同一套治理：SSRF 防护、按域限速、自适应节流、主机熔断、robots 与付费预算。浏览器内核产生的子请求也不例外。</p>
      </div>
      <div class="page-head-actions"><span class="tag tag-warn">${attention} 个域需要关注</span></div>
    </header>
    <section class="gov-summary" aria-label="治理摘要">
      <div class="gov-stat"><div class="metric-label">当前出站</div><div class="gov-num">${currentSnap().egress}<small>/min</small></div><div class="metric-sub">上限 60 /min</div></div>
      <div class="gov-stat"><div class="metric-label">熔断 / 半开</div><div class="gov-num">${openCount}<small>+ ${halfCount}</small></div><div class="metric-sub">${esc(attentionNames)}</div></div>
      <div class="gov-stat"><div class="metric-label">预算余量</div><div class="gov-num">62<small>%</small></div><div class="metric-sub">SearchAPI · 24h 窗口</div></div>
      <div class="gov-stat"><div class="metric-label">robots 豁免</div><div class="gov-num">${ROBOTS.length}<small>条</small></div><div class="metric-sub">全部带理由与审计编号</div></div>
    </section>
    <section class="block grid-full" aria-label="出站域">
      <div class="block-head"><div class="block-title"><span class="block-kicker">EGRESS</span>出站域</div><span class="block-note">按注册域限速 · 状态在多个执行方之间共享</span></div>
      <div class="table-scroll">
        <table class="data-table domain-table">
          <thead><tr><th>域</th><th>速率</th><th>并发</th><th>熔断</th><th>错误率</th></tr></thead>
          <tbody>${domainRows}</tbody>
        </table>
      </div>
    </section>
    <div class="gov-grid">
      <section class="block" aria-label="robots 豁免">
        <div class="block-head"><div class="block-title"><span class="block-kicker">ROBOTS</span>豁免审计</div><span class="block-note">系统级锁定 · 不可绕过</span></div>
        <div class="robots-list">${ROBOTS.map((r) => `<div class="robots-item">
          <div class="robots-head"><span class="cell-strong">${esc(r.source)}</span><span class="tag">${esc(r.category)}</span><span class="cell-sub">${esc(r.last)}</span></div>
          <p class="robots-reason">${esc(r.reason)}</p>
          <div class="robots-meta"><span>${esc(r.audit)}</span><span>理由已写入审计</span></div>
        </div>`).join('')}</div>
        <p class="drawer-note">robots.txt 不可达时按 RFC 9309 处理：4xx 视为无限制，5xx 或网络不可达视为完全禁止。</p>
      </section>
      <section class="block" aria-label="付费预算">
        <div class="block-head"><div class="block-title"><span class="block-kicker">PAID</span>付费预算</div><span class="block-note">预算耗尽不计入失败</span></div>
        ${BUDGETS.map((b) => {
          const pct = Math.round((b.spend / b.limit) * 100);
          return `<div class="budget">
            <div class="budget-head"><span class="budget-name">${esc(b.name)}</span><span class="budget-num"><strong>¥${b.spend.toFixed(2)}</strong> / ¥${b.limit}</span></div>
            <div class="budget-meter ${pct > 45 ? 'is-warn' : ''}" role="progressbar" aria-label="${esc(b.name)}" aria-valuemin="0" aria-valuemax="${b.limit}" aria-valuenow="${b.spend}"><i style="width:${pct}%"></i></div>
            <div class="budget-foot"><span>${esc(b.window)}</span><span>${pct}%</span></div>
          </div>`;
        }).join('')}
        <div class="paid-block">
          <div class="drawer-section-title">最近回执</div>
          <table class="receipt-table">
            <thead><tr><th>任务</th><th>服务</th><th>状态</th><th>金额</th></tr></thead>
            <tbody>${RECEIPTS.map((r) => `<tr><td>${esc(r.task)}</td><td>${esc(r.service)}</td><td class="${r.state === '未知' ? 'delta-warn' : r.state === '完成' ? 'delta-up' : ''}">${esc(r.state)}</td><td>${esc(r.amount)}</td></tr>`).join('')}</tbody>
          </table>
          <p class="drawer-note">结果未知的付费调用不重发；回执与数据在同一个变更集中可见。</p>
        </div>
      </section>
    </div>
    <section class="block grid-full" aria-label="渲染级别">
      <div class="block-head"><div class="block-title"><span class="block-kicker">RENDER</span>渲染级别</div><span class="block-note">按需升级 · 级别记忆存放在 Strata</span></div>
      <div class="render-ladder">
        ${LADDER.map((l, i) => `<div class="ladder-row ${i === 3 ? 'is-top' : ''}">
          <span class="ladder-name">${esc(l.name)}</span>
          <div class="ladder-bar"><i style="width:${l.width}%"></i></div>
          <span class="ladder-count">${l.count} 个信源 · ${esc(l.note)}</span>
        </div>`).join('')}
      </div>
      <p class="ladder-note">截图与渲染级别无关；需要截图时在当前级别开启布局。升级依据来自适配器反馈、拦截页特征与内核不支持信号。</p>
    </section>`;
  }

  function provValue(p) {
    if (p.key === 'schedule.floor' && !state.pending) return '15m';
    return p.value;
  }

  function allGens() {
    const arr = [];
    if (state.pending) arr.push(state.pending);
    return arr.concat(GENS);
  }

  function currentGenData() {
    const gens = allGens();
    return gens.find((g) => g.key === state.selectedGen) || gens[0];
  }

  function selectedFileFor(gen) {
    const files = Object.keys(gen.fileMap);
    return files.includes(state.selectedFile) ? state.selectedFile : files[0];
  }

  function genRailHtml() {
    return allGens().map((g) => `<button class="gen-step ${g.pending ? 'is-pending' : ''}" data-action="select-gen" data-gen="${g.key}" aria-pressed="${state.selectedGen === g.key}">
      <span class="gen-id">${g.id}${g.pending ? ' · 待校验' : ''}</span>
      <span class="gen-at">${g.at} · ${g.files} 文件</span>
    </button>`).join('');
  }

  function diffHtml(gen, file) {
    const payload = gen.fileMap[file];
    if (!payload) return '';
    const label = payload.status === 'modified' ? `${payload.add} 处新增 · ${payload.del} 处删除` : '未变更';
    return `<div class="diff-head">
        <span class="diff-title">${esc(file)}</span>
        <span class="diff-tag">${esc(label)}</span>
      </div>
      <div class="diff">
        ${payload.lines.map((l) => `<div class="diff-line ${l.t}"><span class="sign">${l.t === 'add' ? '+' : l.t === 'del' ? '-' : ' '}</span><span class="ln">${l.n}</span><span>${esc(l.text)}</span></div>`).join('')}
      </div>
      <div class="diff-summary"><span>整体校验：通过</span><span>锁定路径未被改写</span><span>配置代际 ${esc(gen.id)}</span></div>`;
  }

  function renderConfig() {
    const gen = currentGenData();
    const file = selectedFileFor(gen);
    const files = Object.keys(gen.fileMap);
    const prov = PROV.find((p) => p.key === state.selectedProv) || PROV[0];
    return `<header class="page-head">
      <div>
        <div class="page-kicker"><span class="kicker-bar" aria-hidden="true"><i></i><i></i><i></i></span>L4 · CONFIGURATION</div>
        <h1 class="page-title">配置</h1>
        <p class="page-desc">信源、访问器、适配器与策略以 YAML 声明，运行期状态存放在 Strata。配置是只读输入；控制台不提供编辑，只展示整体校验、差异与逐键溯源。</p>
      </div>
      <div class="page-head-actions">
        <span class="tag ${state.pending ? 'tag-warn' : 'tag-muted'}">${state.pending ? '待校验变更' : '已同步'}</span>
        <button class="btn btn-quiet" data-action="revalidate" ${isHistorical() ? 'disabled' : ''}>${icon('refresh', 13)}重新校验</button>
      </div>
    </header>
    ${state.pending
      ? `<div class="pending-banner">
          <div class="pending-copy">${icon('alert', 15)}<span>检测到配置目录变更</span><span class="pending-file">${esc(state.pending.fileMap['sources/arxiv.yaml'] ? 'sources/arxiv.yaml' : '')}</span><span class="pending-diff">${esc(state.pending.note)}</span></div>
          <div class="pending-actions">
            <button class="btn btn-sm btn-quiet" data-action="select-pending">查看差异</button>
            <button class="btn btn-sm btn-primary" data-action="apply-config" ${isHistorical() ? 'disabled' : ''}>校验并生成 G-38</button>
          </div>
        </div>`
      : `<div class="pending-banner is-resolved">
          <div class="pending-copy">${icon('check', 15)}<span>配置目录无待处理变更</span><span class="pending-diff">当前代际 ${esc(state.currentGen)} 已生效 · 文件监听防抖中</span></div>
          <div class="pending-actions"><button class="btn btn-sm btn-quiet" data-action="revalidate">${icon('refresh', 12)}重新校验</button></div>
        </div>`}
    <div class="gen-rail" role="group" aria-label="配置代际">${genRailHtml()}</div>
    <div class="config-layout">
      <aside class="file-pane" aria-label="配置文件">
        <div class="drawer-section-title" style="margin-bottom:8px">文件 · ${gen.files}</div>
        <div class="file-list">
          ${files.map((f) => {
            const p = gen.fileMap[f];
            return `<button class="file-item" data-action="select-file" data-file="${esc(f)}" aria-pressed="${file === f}">${esc(f)}<span class="file-tag">${p.status === 'modified' ? `${p.add} 新增 · ${p.del} 删除` : '未变更'}</span></button>`;
          }).join('')}
        </div>
      </aside>
      <div class="diff-pane">
        ${diffHtml(gen, file)}
        <div class="validation-line"><span class="ok">${icon('check', 11)} 整体校验通过 · 0 错误</span><span class="warn">1 提示 · 渲染上限被系统策略裁剪</span><span>最近校验 ${esc(state.lastCheck)}</span></div>
      </div>
      <aside class="prov-pane" aria-label="配置溯源">
        <div class="drawer-section-title" style="margin-bottom:6px">逐键溯源</div>
        <div class="prov-list">
          ${PROV.map((p) => `<button class="prov-row" data-action="select-prov" data-prov="${esc(p.key)}" aria-pressed="${state.selectedProv === p.key}">
            <span class="prov-key">${esc(p.key)}</span><span class="prov-val">${esc(provValue(p))}</span><span class="prov-file">${esc(p.file)}</span>
          </button>`).join('')}
        </div>
        <div class="prov-panel">
          <div class="prov-panel-title">最终取值来源</div>
          <div class="prov-chain">${prov.chain.map((c, i) => `<span class="prov-step ${i === prov.chain.length - 1 ? 'is-final' : ''}">${esc(c)}</span>`).join('')}</div>
          ${prov.locked ? `<div class="prov-lock">${icon('lock', 11)} 系统级锁定 · 模块不可覆盖</div>` : ''}
        </div>
        <p class="hint-inline" style="display:block;margin-top:12px;line-height:1.7">锁定路径在合并完成后检查：任何模块对锁定键的取值与系统配置不同，整个配置快照校验失败。</p>
      </aside>
    </div>`;
  }

  function nextRuns(s) {
    const base = s.nextIn || 15;
    return [0, 1, 2, 3, 4].map((i) => {
      const total = clockStart + (base + i * 12) * 60;
      const h = Math.floor(total / 3600) % 24;
      const m = Math.floor(total / 60) % 60;
      return `${pad(h)}:${pad(m)}`;
    });
  }

  function drawerShell(s) {
    return `<div class="drawer-head">
      <div class="drawer-topline">
        <div class="drawer-eyebrow">${icon('sources', 12)} SOURCE · ${esc((ACCESSOR[s.accessor] || s.accessor).toUpperCase())} ${statusPill(s.status)}</div>
        <button class="icon-btn" data-action="close-drawer" aria-label="关闭详情">${icon('close', 16)}</button>
      </div>
      <h2 class="drawer-title" id="drawerTitle">${esc(s.name)}</h2>
      <div class="drawer-sub">${esc(s.id)} · ${esc(s.desc)}</div>
      <div class="drawer-actions">
        <button class="btn btn-sm btn-primary" data-action="run-source" ${isHistorical() || s.status === 'paused' ? 'disabled' : ''}>${icon('refresh', 12)}立即运行</button>
        <button class="btn btn-sm btn-quiet" data-action="toggle-source" ${isHistorical() ? 'disabled' : ''}>${icon(s.status === 'paused' ? 'play' : 'pause', 12)}${s.status === 'paused' ? '恢复采集' : '暂停信源'}</button>
        <button class="icon-btn" data-action="copy" data-value="${esc(s.id)}" aria-label="复制信源 ID" title="复制信源 ID">${icon('copy', 14)}</button>
      </div>
      <div class="drawer-tabs" role="tablist" aria-label="信源详情">
        ${[['pace', '节奏'], ['health', '健康'], ['tasks', '任务'], ['config', '配置']].map(([k, label]) => `<button class="drawer-tab" role="tab" data-action="drawer-tab" data-tab="${k}" aria-selected="${state.sourceTab === k}">${label}</button>`).join('')}
      </div>
    </div>`;
  }

  function sourcePace(s) {
    return `<div class="drawer-section">
      <div class="drawer-section-title">节奏推导</div>
      <div class="facts">
        <div class="fact"><div class="fact-label">当前间隔</div><div class="fact-value mono">${esc(s.interval)}</div></div>
        <div class="fact"><div class="fact-label">下次到期</div><div class="fact-value mono">${esc(s.next)}</div></div>
        <div class="fact"><div class="fact-label">下限 / 上限</div><div class="fact-value mono">${esc(s.floor)} / ${esc(s.ceiling)}</div></div>
        <div class="fact"><div class="fact-label">合并分组</div><div class="fact-value mono">${esc(s.group)}</div></div>
        <div class="fact"><div class="fact-label">游标</div><div class="fact-value mono">${esc(s.cursor)}</div></div>
        <div class="fact"><div class="fact-label">策略预设</div><div class="fact-value mono">${esc(s.preset)}</div></div>
      </div>
    </div>
    <div class="drawer-section">
      <div class="drawer-section-title">三项取最大</div>
      <div class="signal"><span class="signal-label">产出自适应 · 近期新增 ${Math.max(2, Math.round(14 * s.trend))} 条</span><span class="signal-value">${esc(s.interval)}</span><div class="signal-bar"><i style="width:100%"></i></div><span class="signal-desc">首次发现才计入，回填不计入产出。</span></div>
      <div class="signal"><span class="signal-label">源站信号 · Feed ttl / Retry-After</span><span class="signal-value">${esc(s.floor)}</span><div class="signal-bar"><i style="width:58%"></i></div></div>
      <div class="signal"><span class="signal-label">下限 · 信源类型与付费约束</span><span class="signal-value">${esc(s.floor)}</span><div class="signal-bar"><i style="width:46%"></i></div><span class="signal-desc">周期收敛在申明的上下限之间；实际下次时间由提交后的状态推进。</span></div>
    </div>
    <div class="drawer-section">
      <div class="drawer-section-title">接下来的调度</div>
      <div class="timeline">${nextRuns(s).map((t, i) => `<div class="tl-item"><span class="tl-mark ${i === 0 ? 'is-now' : ''}"></span><span><span class="tl-title">${i === 0 ? '到期派发 · 延迟为 0' : '计划访问'}</span><span class="tl-sub">trigger: ${i === 0 ? 'scheduled' : 'adaptive'}</span></span><span class="tl-time">${t}</span></div>`).join('')}</div>
    </div>
    <p class="drawer-note">游标只在所有适配器成功之后，与数据、回执和任务确认一起提交。失败时不推进游标，下次从相同位置继续。</p>`;
  }

  function sourceHealth(s) {
    const sig = [
      { label: 'yield_trend', value: `${s.trend.toFixed(2)}× 长期均值`, pct: Math.min(100, Math.round(s.trend * 100)), tone: s.trend < 0.5 ? 'hot' : s.trend < 0.8 ? 'warn' : '' },
      { label: 'consecutive_failures', value: `${s.failures} 次 · ${s.failure || '—'}`, pct: Math.min(100, s.failures * 16), tone: s.failures > 3 ? 'hot' : s.failures ? 'warn' : '' },
      { label: 'selector_miss', value: `${s.selectorMiss} 次连续未命中`, pct: Math.min(100, s.selectorMiss * 20), tone: s.selectorMiss > 2 ? 'warn' : '' },
      { label: 'blocked_rate', value: `${Math.round(s.blockedRate * 100)}% 拦截页 / 空壳页`, pct: Math.min(100, Math.round(s.blockedRate * 100)), tone: s.blockedRate > 0.2 ? 'hot' : s.blockedRate > 0.05 ? 'warn' : '' },
      { label: 'render_level', value: `${s.render} · 最近成功记忆`, pct: 44, tone: '' },
      { label: 'cost', value: `${s.cost} · 24h 窗口`, pct: s.accessor === 'browser' || s.preset === 'minimal' ? 32 : 6, tone: '' }
    ];
    return `<div class="drawer-section">
      <div class="drawer-section-title">健康信号 · 存放在 Strata</div>
      <div class="signal-list">${sig.map((x) => `<div class="signal"><span class="signal-label">${x.label}</span><span class="signal-value">${esc(x.value)}</span><div class="signal-bar ${x.tone}"><i style="width:${x.pct}%"></i></div></div>`).join('')}</div>
    </div>
    <div class="drawer-section">
      <div class="drawer-section-title">状态解释</div>
      <div class="facts">
        <div class="fact"><div class="fact-label">最近成功</div><div class="fact-value">${esc(s.lastOk)}</div></div>
        <div class="fact"><div class="fact-label">期望周期</div><div class="fact-value mono">${esc(s.interval)} · 超过即 suspect</div></div>
        <div class="fact"><div class="fact-label">最近错误</div><div class="fact-value mono">${esc(s.lastError || '无')}</div></div>
        <div class="fact"><div class="fact-label">判定规则</div><div class="fact-value">按错误类别分别计数；劣化不退避，触发渲染升级判定</div></div>
      </div>
    </div>
    <p class="drawer-note">健康结果反馈给调度器，用于调整节奏和熔断，并作为事件对外发布。阈值由 policy.health 配置。</p>`;
  }

  function sourceTasks(s) {
    const rows = [
      { trig: 'scheduled', at: '14:28:04', dur: '842ms', state: '成功 · 3 个 facet', cls: 'ok', commit: 'a1f9c2e' },
      { trig: 'follow', at: '14:24:51', dur: '1.2s', state: '成功 · 1 个发现记录', cls: 'ok', commit: '3c2e817' },
      { trig: 'render-upgrade', at: '14:18:22', dur: '4.8s', state: s.status === 'failing' ? '失败 · 403' : '成功 · 结构', cls: s.status === 'failing' ? 'err' : 'ok', commit: '7b0d4aa' },
      { trig: 'scheduled', at: '14:12:10', dur: '706ms', state: s.failures ? '失败 · 429，退避 90s' : '成功 · 0 新增', cls: s.failures ? 'warn' : 'ok', commit: '8d7a330' },
      { trig: 'manual', at: '13:58:31', dur: '1.0s', state: '成功 · 游标未变', cls: 'ok', commit: 'f04b1c9' }
    ];
    return `<div class="drawer-section">
      <div class="drawer-section-title">最近任务</div>
      <div class="task-list">${rows.map((r) => `<div class="task-row">
        <span class="task-title">${esc(r.trig)} · ${esc(r.at)}</span>
        <span class="task-state ${r.cls}">${esc(r.state)}</span>
        <span class="task-meta">${esc(r.dur)} · commit ${esc(r.commit)}</span>
        <span class="task-state" aria-hidden="true"></span>
      </div>`).join('')}</div>
    </div>
    <p class="drawer-note">任务至少执行一次；重复执行由幂等键与内容去重消化。成功确认与结果变更集一同生效。</p>`;
  }

  function yamlBlock(text) {
    return text.split('\n').map((line, i) => {
      const m = line.match(/^(\s*)([-\w.]+):(.*)$/);
      const content = m ? `${m[1]}<span class="code-k">${m[2]}</span>:${esc(m[3])}` : esc(line);
      return `<div class="code-line"><span class="ln">${i + 1}</span><span>${content || ' '}</span></div>`;
    }).join('');
  }

  function sourceConfig(s) {
    const yaml = `id: ${s.id}
kind: source
name: ${s.name}
labels: { tier: ${s.tier}, lang: ${s.lang} }
extends: [${s.extends}]
policy:
  preset: ${s.preset}
  raw: { retain: 180d }
access:
  accessor: accessor/${s.accessor}
  locator: { url: "${LOCATORS[s.id] || 'https://example.org/'}" }
  render: auto
schedule:
  adaptive: true
  floor: ${s.floor}
  ceiling: ${s.ceiling}
admission:
  initialBackfill: { limit: 30 }`;
    return `<div class="drawer-section">
      <div class="drawer-section-title">有效配置 · 只读</div>
      <div class="code-block">${yamlBlock(yaml)}</div>
    </div>
    <p class="drawer-note">配置是只读输入。编辑 YAML 文件后，整体校验通过会生成新的配置代际；在途任务继续使用派发时的快照。</p>`;
  }

  function sourceDrawerHtml(id) {
    const s = SOURCES.find((x) => x.id === id);
    if (!s) return '<p>信源不存在。</p>';
    const tab = state.sourceTab || 'pace';
    const body = tab === 'health' ? sourceHealth(s) : tab === 'tasks' ? sourceTasks(s) : tab === 'config' ? sourceConfig(s) : sourcePace(s);
    return drawerShell(s) + `<div class="drawer-body">${body}</div>`;
  }

  function commitDrawerHtml(hash) {
    const c = COMMITS.find((x) => x.hash === hash);
    if (!c) return '<p>提交不存在。</p>';
    const genMatch = c.title.match(/G-\d+/);
    const genId = genMatch ? genMatch[0] : state.currentGen;
    const ops = c.kind === 'config'
      ? [`setState configGeneration = ${genId}`, 'label config:audited', 'link diff → commit:a1f9c2e']
      : c.kind === 'health'
        ? ['setState health.status = failing', 'emit health.degraded', 'link signal → source/reuters-world']
        : c.kind === 'pause'
          ? ['setState source/enabled = false', 'label operator:manual', 'link pause → source/producthunt']
          : ['ensure identity → rid', 'putFacet md:readable ×42', 'setState cursor', 'ack task-01JA7K…/attempt-1'];
    return `<div class="drawer-head">
      <div class="drawer-topline">
        <div class="drawer-eyebrow">COMMIT · ${esc(c.hash)} · SNAPSHOT #${esc(c.snap)}</div>
        <button class="icon-btn" data-action="close-drawer" aria-label="关闭提交详情">${icon('close', 16)}</button>
      </div>
      <h2 class="drawer-title" id="drawerTitle">${esc(c.title)}</h2>
      <div class="drawer-sub">${esc(c.time)} · ${esc(c.producer)} · 变更集整体可见</div>
    </div>
    <div class="drawer-body">
      <div class="drawer-section">
        <div class="facts">
          <div class="fact"><div class="fact-label">幂等键</div><div class="fact-value mono">task-${esc(c.hash)}/attempt-1</div></div>
          <div class="fact"><div class="fact-label">配置代际</div><div class="fact-value mono">${esc(state.currentGen)}</div></div>
          <div class="fact"><div class="fact-label">可见性</div><div class="fact-value">已可见 · 同一快照</div></div>
          <div class="fact"><div class="fact-label">操作数</div><div class="fact-value mono">${ops.length}</div></div>
        </div>
      </div>
      <div class="drawer-section">
        <div class="drawer-section-title">变更集操作</div>
        <div class="code-block">${ops.map((o, i) => `<div class="code-line"><span class="ln">${i + 1}</span><span>${esc(o)}</span></div>`).join('')}</div>
      </div>
      <div class="drawer-section">
        <div class="drawer-section-title">一致性</div>
        <p class="robots-reason">任务确认与结果变更集一同生效；游标只在全部适配器成功后推进。进程崩溃后重放同一幂等键不会产生重复数据。</p>
      </div>
    </div>`;
  }

  function syncModalLock() {
    const locked = !$('#drawerLayer').hidden || !$('#paletteLayer').hidden || !$('#confirmLayer').hidden;
    document.body.classList.toggle('is-modal', locked);
  }

  function renderDrawer() {
    const layer = $('#drawerLayer');
    if (!state.drawer) {
      layer.hidden = true;
      syncModalLock();
      return;
    }
    const wasHidden = layer.hidden;
    $('#drawer').innerHTML = state.drawer.type === 'source' ? sourceDrawerHtml(state.drawer.id) : commitDrawerHtml(state.drawer.hash);
    layer.hidden = false;
    if (wasHidden) lastFocus = document.activeElement;
    syncModalLock();
    requestAnimationFrame(() => $('#drawer').focus());
  }

  function closeDrawer() {
    state.drawer = null;
    $('#drawerLayer').hidden = true;
    syncModalLock();
    if (lastFocus && typeof lastFocus.focus === 'function') lastFocus.focus();
  }

  function openSource(id) {
    state.sourceTab = 'pace';
    state.drawer = { type: 'source', id };
    renderDrawer();
  }

  function openCommit(hash) {
    state.drawer = { type: 'commit', hash };
    renderDrawer();
  }

  function toast(title, desc = '', kind = 'ok') {
    const region = $('#toastRegion');
    const el = document.createElement('div');
    el.className = `toast ${kind === 'ok' ? 'is-ok' : kind === 'warn' ? 'is-warn' : ''}`;
    el.innerHTML = `<span class="toast-icon">${icon(kind === 'warn' ? 'alert' : 'check', 14)}</span>
      <div class="toast-body"><div class="toast-title">${esc(title)}</div>${desc ? `<div class="toast-desc">${esc(desc)}</div>` : ''}</div>
      <button class="toast-close" type="button" aria-label="关闭提示">${icon('close', 12)}</button>`;
    region.appendChild(el);
    const close = () => { el.remove(); };
    el.querySelector('.toast-close').addEventListener('click', close);
    setTimeout(close, 4200);
  }

  function updateConfigChrome() {
    $('#genChip').textContent = state.currentGen;
    $('#footGeneration').textContent = state.currentGen;
    $('#configPendingDot').hidden = !state.pending;
    $('#footSnapshot').textContent = currentSnap().label;
  }

  function syncChrome() {
    const labels = { overview: '总览', sources: '信源', strata: 'Strata', governance: '治理', config: '配置' };
    $$('.nav-item').forEach((b) => {
      if (b.dataset.route === state.route) b.setAttribute('aria-current', 'page');
      else b.removeAttribute('aria-current');
    });
    $('#crumbCurrent').textContent = labels[state.route] || '总览';
    document.title = `${labels[state.route] || '总览'} · ModalPrism 沉积观测台`;
    const govAlert = $('#navGovAlert');
    if (govAlert) govAlert.textContent = EGRESS.filter((d) => d.circuit !== 'closed').length;
    document.body.classList.toggle('is-historical', isHistorical());
    $('#liveLabel').textContent = isHistorical() ? '历史' : '实时';
    $('#snapId').textContent = currentSnap().label;
    $('#snapTime').textContent = currentSnap().time;
    $('#snapMode').textContent = isHistorical() ? '历史快照' : '实时';
    $('#footSnapshot').textContent = currentSnap().label;
    $('#snapLive').hidden = !isHistorical();
    $$('.snap-tick').forEach((b) => {
      b.setAttribute('aria-pressed', String(b.dataset.snap === currentSnap().id));
      b.classList.toggle('is-filled', Number(b.dataset.snap) <= Number(currentSnap().id));
    });
    const prev = $('#snapPrev'); const next = $('#snapNext');
    if (prev) prev.disabled = state.asOf === 0;
    if (next) next.disabled = state.asOf === SNAPSHOTS.length - 1;
    const banner = $('#historyBanner');
    if (isHistorical()) {
      banner.hidden = false;
      banner.innerHTML = `<div class="hb-copy">${icon('clock', 14)}<span>正在读取历史快照 <strong>${currentSnap().label}</strong> · ${currentSnap().time}</span><span class="hb-note">写操作已冻结，仅可读取与导航。</span></div><button class="btn btn-sm btn-quiet" data-action="back-live">回到实时</button>`;
    } else {
      banner.hidden = true;
      banner.innerHTML = '';
    }
    updateConfigChrome();
    const taskCount = $('#navTaskCount');
    if (taskCount) taskCount.textContent = currentSnap().tasks;
  }

  function buildSnapshotTrack() {
    $('#snapTrack').innerHTML = SNAPSHOTS.map((s) => `<button class="snap-tick is-filled" data-action="select-snapshot" data-snap="${s.id}" aria-pressed="${s.id === currentSnap().id}" aria-label="快照 ${s.label}，${s.time}" title="${s.label} · ${s.time}">${s.id.slice(-2)}</button>`).join('');
  }

  function chartTipHtml(idx) {
    const m = chartModel();
    const d = m.series[idx];
    if (!d) return '';
    return `<div class="chart-tip-time">${binLabel(m.range, idx, m.bins)} · ${fmt(d.total)} 条</div>
      <div class="chart-tip-row"><i class="swatch" style="background:#5F6E64"></i>原始信封 <strong>${fmt(d.raw)}</strong></div>
      <div class="chart-tip-row"><i class="swatch" style="background:#A6C09A"></i>Markdown <strong>${fmt(d.md)}</strong></div>
      <div class="chart-tip-row"><i class="swatch" style="background:#D6A24A"></i>结构化 <strong>${fmt(d.struct)}</strong></div>
      <div class="chart-tip-row"><i class="swatch" style="background:#E2552F"></i>向量 <strong>${fmt(d.emb)}</strong></div>`;
  }

  function bindChart() {
    const wrap = $('.chart-wrap');
    if (!wrap) return;
    const tip = $('#chartTip', wrap);
    $$('.chart-hit', wrap).forEach((hit) => {
      const show = () => {
        if (!tip) return;
        const idx = Number(hit.dataset.idx);
        tip.innerHTML = chartTipHtml(idx);
        const r = hit.getBoundingClientRect();
        const wr = wrap.getBoundingClientRect();
        tip.style.left = `${Math.max(6, Math.min(r.left - wr.left + r.width / 2 - 79, wr.width - 172))}px`;
        tip.style.top = '8px';
        tip.classList.add('is-on');
      };
      hit.addEventListener('mouseenter', show);
      hit.addEventListener('mousemove', show);
      hit.addEventListener('mouseleave', () => tip && tip.classList.remove('is-on'));
    });
  }

  function updateSourceRows() {
    const rows = filteredSources();
    const tb = $('#sourceRows');
    if (!tb) return;
    tb.innerHTML = rows.length ? rows.map(sourceRow).join('') : `<tr><td colspan="6"><div class="empty-state"><strong>没有匹配的信源</strong>调整关键词或状态筛选。</div></td></tr>`;
    const count = $('#sourceFilteredCount');
    if (count) count.textContent = `${rows.length} 个信源`;
  }

  function renderCurrentView() {
    const view = $('#viewRoot');
    if (state.route === 'sources') view.innerHTML = renderSources();
    else if (state.route === 'strata') view.innerHTML = renderStrata();
    else if (state.route === 'governance') view.innerHTML = renderGovernance();
    else if (state.route === 'config') view.innerHTML = renderConfig();
    else view.innerHTML = renderOverview();
    const ss = $('#sourceSearch');
    if (ss) ss.addEventListener('input', (e) => { state.sourceQuery = e.target.value; updateSourceRows(); });
    const rs = $('#resourceSearch');
    if (rs) rs.addEventListener('input', (e) => {
      state.resourceQuery = e.target.value;
      const list = $('#resourceList');
      if (list) {
        const rows = filteredResources();
        list.innerHTML = rows.length ? rows.map(resourceItem).join('') : `<div class="empty-state"><strong>没有匹配的资源</strong>换一个关键词，或清除搜索。</div>`;
      }
    });
    bindChart();
    syncChrome();
  }

  function navigate(route) {
    if (!['overview', 'sources', 'strata', 'governance', 'config'].includes(route)) return;
    const changed = state.route !== route || location.hash !== `#/${route}`;
    state.route = route;
    if (location.hash !== `#/${route}`) history.pushState(null, '', `#/${route}`);
    renderCurrentView();
    if (changed) window.scrollTo({ top: 0, behavior: 'instant' in window ? 'instant' : 'auto' });
  }

  function withLoading(btn, fn, delay = 850) {
    if (!btn) { setTimeout(fn, 0); return; }
    if (btn.disabled) return;
    const original = btn.innerHTML;
    btn.classList.add('is-loading');
    btn.disabled = true;
    btn.innerHTML = `<span class="spinner"></span>${btn.dataset.loading || '处理中'}`;
    setTimeout(() => {
      btn.classList.remove('is-loading');
      btn.disabled = false;
      btn.innerHTML = original;
      fn();
    }, delay);
  }

  function runScan(btn) {
    if (isHistorical()) { toast('历史快照为只读', '回到实时后才能触发扫描。', 'warn'); return; }
    withLoading(btn, () => {
      currentSnap().tasks += 6;
      const c = addCommit('手动触发扫描 · 生成 6 个访问任务', 'ingest', 'console/operator');
      toast('扫描完成', `6 个访问任务进入免费队列 · 变更集 ${c.hash} 已接收`, 'ok');
      renderCurrentView();
    }, 800);
  }

  function runSource(btn) {
    if (isHistorical()) { toast('历史快照为只读', '回到实时后才能派发任务。', 'warn'); return; }
    const s = SOURCES.find((x) => x.id === state.drawer.id);
    if (!s || s.status === 'paused') return;
    withLoading(btn, () => {
      s.lastOk = '刚刚';
      const c = addCommit(`手动运行 · ${s.id} 完成一次访问`, 'ingest', 'console/operator');
      toast('任务已派发', `${s.id} · 变更集 ${c.hash} 已接收`, 'ok');
      renderDrawer();
      renderCurrentView();
    }, 950);
  }

  function toggleSource(btn) {
    if (isHistorical()) { toast('历史快照为只读', '回到实时后才能修改运行状态。', 'warn'); return; }
    const s = SOURCES.find((x) => x.id === state.drawer.id);
    if (!s) return;
    const pausing = s.status !== 'paused';
    if (pausing) s.resumeTo = s.status;
    s.status = pausing ? 'paused' : (s.resumeTo || 'ok');
    const c = addCommit(`${pausing ? '暂停' : '恢复'}采集 · ${s.id}`, 'pause', 'console/operator');
    toast(pausing ? '信源已暂停' : '信源已恢复', `变更集 ${c.hash} 已接收 · 调度器将按新状态决策`, 'ok');
    renderDrawer();
    renderCurrentView();
  }

  function applyConfig(btn) {
    if (isHistorical()) { toast('历史快照为只读', '回到实时后才能生成新的配置代际。', 'warn'); return; }
    withLoading(btn, () => {
      const p = state.pending;
      if (!p) return;
      const applied = { ...p, key: 'G-38', id: 'G-38', at: nowHM(), pending: false };
      GENS.unshift(applied);
      state.pending = null;
      state.currentGen = 'G-38';
      state.selectedGen = 'G-38';
      state.selectedFile = 'sources/arxiv.yaml';
      state.lastCheck = '刚刚';
      const c = addCommit('配置代际 G-38 生效 · arxiv 节奏下限 15m', 'config', 'config/loader v0.6.0');
      toast('配置代际 G-38 已生效', `174 个文件通过整体校验 · 变更集 ${c.hash} 已可见`, 'ok');
      renderCurrentView();
    }, 1200);
  }

  function revalidate(btn) {
    withLoading(btn, () => {
      state.lastCheck = '刚刚';
      toast('校验通过', '0 错误 · 1 提示 · 当前配置快照不可变', 'ok');
      renderCurrentView();
    }, 750);
  }

  function askConfirm({ title, body, label, onAccept }) {
    $('#confirmTitle').textContent = title;
    $('#confirmBody').textContent = body;
    $('#confirmAccept').textContent = label || '确认';
    state.confirmHandler = onAccept;
    $('#confirmLayer').hidden = false;
    syncModalLock();
    requestAnimationFrame(() => $('#confirmAccept').focus());
  }

  function closeConfirm() {
    $('#confirmLayer').hidden = true;
    state.confirmHandler = null;
    syncModalLock();
  }

  function resetCircuit(domain) {
    if (isHistorical()) { toast('历史快照为只读', '回到实时后才能重置熔断。', 'warn'); return; }
    askConfirm({
      title: `重置熔断 · ${domain}`,
      body: '重置会让该域的熔断计数与冷却时间归零，并立即允许一次探测请求。出站治理仍会按域限速。',
      label: '确认重置',
      onAccept: () => {
        const d = EGRESS.find((x) => x.domain === domain);
        if (!d) return;
        d.circuit = 'closed';
        d.cooldown = '—';
        d.throttle = '0%';
        const c = addCommit(`重置熔断 · ${domain}`, 'health', 'console/operator');
        toast('熔断已重置', `${domain} · 变更集 ${c.hash} 已接收`, 'ok');
        renderCurrentView();
      }
    });
  }

  async function copyText(text) {
    try {
      await navigator.clipboard.writeText(text);
      toast('已复制', text, 'ok');
    } catch (err) {
      const ta = document.createElement('textarea');
      ta.value = text;
      ta.style.position = 'fixed';
      ta.style.opacity = '0';
      document.body.appendChild(ta);
      ta.select();
      try { document.execCommand('copy'); toast('已复制', text, 'ok'); }
      catch (e) { toast('复制失败', '当前环境不允许访问剪贴板。', 'warn'); }
      ta.remove();
    }
  }

  function selectSnapshot(id) {
    const idx = SNAPSHOTS.findIndex((s) => s.id === id);
    if (idx < 0) return;
    state.asOf = idx;
    if (state.drawer) closeDrawer();
    renderCurrentView();
  }

  function paletteCommands() {
    const cmds = [];
    const routes = [
      ['overview', '总览', '系统状态、采集脉冲与提交', 'overview'],
      ['sources', '信源', '健康矩阵与节奏推导', 'sources'],
      ['strata', 'Strata 资源', 'facet 版本链与血缘', 'strata'],
      ['governance', '治理', '出站、robots、预算与渲染', 'governance'],
      ['config', '配置', '代际、差异与逐键溯源', 'config']
    ];
    routes.forEach(([route, label, sub, ic]) => cmds.push({ type: 'route', route, label, sub, icon: ic, group: '视图' }));
    SOURCES.forEach((s) => cmds.push({ type: 'source', id: s.id, label: s.name, sub: s.id, icon: s.accessor, group: '信源' }));
    cmds.push({ type: 'action', id: 'run', label: '运行一轮扫描', sub: '生成访问任务并提交', icon: 'refresh', group: '动作' });
    cmds.push({ type: 'action', id: 'revalidate', label: '重新校验配置', sub: '检查当前 YAML 快照', icon: 'check', group: '动作' });
    cmds.push({ type: 'action', id: 'live', label: '回到实时快照', sub: '解除历史只读状态', icon: 'clock', group: '动作' });
    return cmds;
  }

  function paletteFiltered(query) {
    const q = query.trim().toLowerCase();
    const all = paletteCommands();
    return q ? all.filter((c) => `${c.label} ${c.sub} ${c.group}`.toLowerCase().includes(q)) : all;
  }

  function renderPalette(query) {
    const items = paletteFiltered(query);
    state.paletteFiltered = items;
    if (state.paletteIndex >= items.length) state.paletteIndex = 0;
    const results = $('#paletteResults');
    if (!items.length) {
      results.innerHTML = `<div class="palette-empty">没有匹配的命令<br>试试「reuters」「配置」或「运行」</div>`;
      return;
    }
    let html = '';
    let lastGroup = '';
    items.forEach((c, i) => {
      if (c.group !== lastGroup) { html += `<div class="palette-group-label">${c.group}</div>`; lastGroup = c.group; }
      html += `<button class="palette-item" type="button" role="option" data-pi="${i}" aria-selected="${i === state.paletteIndex}">
        <span class="pi-icon">${icon(c.icon, 15)}</span>
        <span class="pi-main"><span class="pi-title">${esc(c.label)}</span><span class="pi-sub">${esc(c.sub)}</span></span>
        <span class="pi-meta">${c.group}</span>
      </button>`;
    });
    results.innerHTML = html;
    const sel = results.querySelector(`[data-pi="${state.paletteIndex}"]`);
    if (sel) sel.scrollIntoView({ block: 'nearest' });
  }

  function openPalette() {
    lastFocus = document.activeElement;
    $('#paletteLayer').hidden = false;
    $('#paletteInput').value = '';
    state.paletteIndex = 0;
    renderPalette('');
    syncModalLock();
    requestAnimationFrame(() => $('#paletteInput').focus());
  }

  function closePalette() {
    $('#paletteLayer').hidden = true;
    syncModalLock();
    if (lastFocus && typeof lastFocus.focus === 'function') lastFocus.focus();
  }

  function executePalette(cmd) {
    closePalette();
    if (!cmd) return;
    if (cmd.type === 'route') { navigate(cmd.route); return; }
    if (cmd.type === 'source') { openSource(cmd.id); return; }
    if (cmd.type === 'action') {
      if (cmd.id === 'run') { if (isHistorical()) { toast('历史快照为只读', '回到实时后才能运行扫描。', 'warn'); return; } runScan(null); }
      if (cmd.id === 'revalidate') { if (isHistorical()) { toast('历史快照为只读', '回到实时后才能校验。', 'warn'); return; } revalidate(null); }
      if (cmd.id === 'live') { state.asOf = SNAPSHOTS.length - 1; renderCurrentView(); }
    }
  }

  function handleAction(action, el) {
    switch (action) {
      case 'run-scan': runScan(el); break;
      case 'open-source': openSource(el.dataset.id); break;
      case 'open-commit': openCommit(el.dataset.hash); break;
      case 'close-drawer': closeDrawer(); break;
      case 'drawer-tab': state.sourceTab = el.dataset.tab; renderDrawer(); break;
      case 'run-source': runSource(el); break;
      case 'toggle-source': toggleSource(el); break;
      case 'copy-rid': copyText(activeResource().rid); break;
      case 'copy': copyText(el.dataset.value); break;
      case 'select-facet':
        state.facetKey = el.dataset.facet;
        state.versionIndex = null;
        if (['md', 'struct', 'raw'].includes(state.facetKey)) state.previewTab = state.facetKey === 'md' ? 'readable' : state.facetKey;
        renderCurrentView();
        break;
      case 'select-version': state.versionIndex = Number(el.dataset.version); renderCurrentView(); break;
      case 'preview-tab':
        state.previewTab = el.dataset.tab;
        state.facetKey = el.dataset.facet;
        state.versionIndex = null;
        renderCurrentView();
        break;
      case 'select-resource':
        state.resourceId = el.dataset.rid;
        state.facetKey = 'md';
        state.versionIndex = null;
        state.previewTab = 'readable';
        renderCurrentView();
        break;
      case 'source-filter':
        state.sourceFilter = el.dataset.value;
        renderCurrentView();
        break;
      case 'chart-range':
        state.range = el.dataset.range;
        renderCurrentView();
        break;
      case 'toggle-domain':
        state.expandedDomain = state.expandedDomain === el.dataset.domain ? null : el.dataset.domain;
        renderCurrentView();
        break;
      case 'reset-circuit': resetCircuit(el.dataset.domain); break;
      case 'select-gen': {
        const gen = allGens().find((g) => g.key === el.dataset.gen);
        if (!gen) return;
        state.selectedGen = gen.key;
        const f = Object.keys(gen.fileMap).find((k) => gen.fileMap[k].status === 'modified') || Object.keys(gen.fileMap)[0];
        state.selectedFile = f;
        renderCurrentView();
        break;
      }
      case 'select-file': state.selectedFile = el.dataset.file; renderCurrentView(); break;
      case 'select-prov': state.selectedProv = el.dataset.prov; renderCurrentView(); break;
      case 'select-pending':
        if (!state.pending) return;
        state.selectedGen = 'pending';
        state.selectedFile = 'sources/arxiv.yaml';
        renderCurrentView();
        break;
      case 'apply-config': applyConfig(el); break;
      case 'revalidate': revalidate(el); break;
      case 'back-live': state.asOf = SNAPSHOTS.length - 1; renderCurrentView(); break;
      case 'select-snapshot': selectSnapshot(el.dataset.snap); break;
      case 'snap-prev': if (state.asOf > 0) selectSnapshot(SNAPSHOTS[state.asOf - 1].id); break;
      case 'snap-next': if (state.asOf < SNAPSHOTS.length - 1) selectSnapshot(SNAPSHOTS[state.asOf + 1].id); break;
      default: break;
    }
  }

  document.addEventListener('click', (e) => {
    if (e.target.closest('[data-close-drawer]')) { closeDrawer(); return; }
    if (e.target.closest('[data-close-palette]')) { closePalette(); return; }
    if (e.target.closest('[data-close-confirm]')) { closeConfirm(); return; }
    const act = e.target.closest('[data-action]');
    if (act) {
      e.preventDefault();
      handleAction(act.dataset.action, act);
      return;
    }
    const route = e.target.closest('[data-route]');
    if (route) { navigate(route.dataset.route); }
  });

  document.addEventListener('keydown', (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
      e.preventDefault();
      if ($('#paletteLayer').hidden) openPalette(); else closePalette();
      return;
    }
    if (e.key === 'Escape') {
      if (!$('#confirmLayer').hidden) { closeConfirm(); return; }
      if (!$('#paletteLayer').hidden) { closePalette(); return; }
      if (!$('#drawerLayer').hidden) { closeDrawer(); return; }
    }
    if (e.key === '/' && !['INPUT', 'TEXTAREA'].includes(document.activeElement.tagName)) {
      e.preventDefault();
      openPalette();
      return;
    }
    if (e.key === 'Tab') {
      const layer = !$('#confirmLayer').hidden ? $('#confirmLayer')
        : !$('#paletteLayer').hidden ? $('#paletteLayer')
          : !$('#drawerLayer').hidden ? $('#drawer') : null;
      if (!layer) return;
      const focusables = $$('button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])', layer).filter((n) => !n.disabled && n.offsetParent !== null);
      if (!focusables.length) return;
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      if (!layer.contains(document.activeElement)) { e.preventDefault(); first.focus(); }
      else if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
    }
  });

  $('#paletteTrigger').addEventListener('click', openPalette);
  const skipLink = $('.skip-link');
  if (skipLink) skipLink.addEventListener('click', (e) => {
    e.preventDefault();
    $('#viewRoot').focus();
  });
  $('#snapLive').addEventListener('click', () => { state.asOf = SNAPSHOTS.length - 1; renderCurrentView(); });
  $('#snapPrev').addEventListener('click', () => { if (state.asOf > 0) selectSnapshot(SNAPSHOTS[state.asOf - 1].id); });
  $('#snapNext').addEventListener('click', () => { if (state.asOf < SNAPSHOTS.length - 1) selectSnapshot(SNAPSHOTS[state.asOf + 1].id); });
  $('#genChip').addEventListener('click', () => navigate('config'));

  $('#paletteInput').addEventListener('input', (e) => { state.paletteIndex = 0; renderPalette(e.target.value); });
  $('#paletteInput').addEventListener('keydown', (e) => {
    if (e.key === 'ArrowDown') { e.preventDefault(); state.paletteIndex = Math.min(state.paletteIndex + 1, state.paletteFiltered.length - 1); renderPalette(e.target.value); }
    if (e.key === 'ArrowUp') { e.preventDefault(); state.paletteIndex = Math.max(state.paletteIndex - 1, 0); renderPalette(e.target.value); }
    if (e.key === 'Enter') { e.preventDefault(); executePalette(state.paletteFiltered[state.paletteIndex]); }
  });
  $('#paletteResults').addEventListener('click', (e) => {
    const item = e.target.closest('[data-pi]');
    if (item) executePalette(state.paletteFiltered[Number(item.dataset.pi)]);
  });

  $('#confirmAccept').addEventListener('click', () => {
    const fn = state.confirmHandler;
    closeConfirm();
    if (fn) fn();
  });

  $$('[data-icon]').forEach((el) => { el.innerHTML = icon(el.dataset.icon, 17); });

  setInterval(() => {
    clockSec += 1;
    const clock = $('#clock');
    if (clock) clock.textContent = clockString();
  }, 1000);

  buildSnapshotTrack();
  state.route = (location.hash || '#/overview').replace(/^#\/?/, '') || 'overview';
  if (!['overview', 'sources', 'strata', 'governance', 'config'].includes(state.route)) state.route = 'overview';
  if (location.hash !== `#/${state.route}`) history.replaceState(null, '', `#/${state.route}`);
  $('#clock').textContent = clockString();
  renderCurrentView();
  window.addEventListener('hashchange', () => {
    const route = (location.hash || '#/overview').replace(/^#\/?/, '') || 'overview';
    if (route !== state.route) navigate(route);
  });
})();
