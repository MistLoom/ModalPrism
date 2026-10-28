# modalprism-console

单包结构：Vite + React SPA 与 Node 静态托管 / 透明代理同目录。

领域逻辑全部在 Rust modalprism-collector；本包 Node 进程只做四件事：托管 dist/、SPA fallback、/healthz、把 /api/v1 原样代理到 collector。

## 布局

`	ext
apps/modalprism-console/
  index.mjs          # Node 入口（默认 127.0.0.1:17890）
  package.json       # build / start / dev
  index.html
  vite.config.ts
  src/               # React 源码
  scripts/           # 构建前 OpenAPI 契约检查
  dist/              # Vite 构建产物（start 默认托管这里）
`

## Build

在仓库根或本目录均可；依赖 pnpm，Node >= 22。

`sh
cd apps/modalprism-console
pnpm install
pnpm build
`

pnpm build 会：校验 src/lib/api.ts 调用命中 docs/specs/public-api.openapi.yaml → 	sc -b → ite build 写出 dist/。

## Start

需先有 collector 在回环监听（默认 http://127.0.0.1:17880）。

`sh
cd apps/modalprism-console
pnpm start
# 等价：
node index.mjs --backend http://127.0.0.1:17880 --bind 127.0.0.1:17890
`

打开 http://127.0.0.1:17890。可选参数：--bind / --http、--backend / --ops、--web（覆盖静态目录，默认本包 dist）。监听与 backend 主机必须是回环地址。

## 开发

两进程（推荐）：

1. cargo run -p modalprism-collector（:17880）
2. 本目录 pnpm start（:17890，托管已构建的 dist，或先 pnpm build）
3. 可选 HMR：另开终端 pnpm dev（Vite :5178，把 /api 代理到 :17890）

仅改前端时：pnpm build 后刷新 :17890，或用 pnpm dev 走 Vite。

## 边界

- 不把业务 API 写进 Node；不改写 /api/v1 方法、路径、正文、状态码或关键头。
- 前端不得构造 Strata 查询或调用通用 Ability / /api/ops。
