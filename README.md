# ModalPrism

A multi-source data collection system written in Rust: it collects data from RSS, web pages, JS-rendered pages, APIs, pushes, and other sources and stores it verbatim, with heterogeneous multi-modal content, append-only and replayable, queryable the moment it lands.

[![License](https://img.shields.io/badge/license-Apache--2.0-blue)](#license)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-ed9e28?logo=rust&logoColor=white)](#quick-start)
[![Store](https://img.shields.io/badge/store-SQLite%20%E2%86%92%20PostgreSQL-336791)](#architecture)

**English** · [简体中文](README.zh-CN.md)

## Highlights

- **All source types in one process**: RSS/Atom, web list & detail pages, JS-rendered pages, open APIs, metered/paid APIs, search engines, inbound pushes, local files; 23 built-in source configs, and a new source is one YAML file.
- **Raw data kept byte-for-byte — append-only, replayable**: reprocess what you already have when a policy or recognition changes, instead of re-crawling.
- **End-to-end lineage**: every processing layer records who produced it and with what config — auditable data, traceable origins.
- **Queryable the moment it's collected**: text, structured data, images, and media unified as addressable resources with automatic de-duplication; runs locally on a single file, switch to PostgreSQL when needed.
- **Filter, full-text, and vector search fused in one query interface** — business code never writes SQL.
- **Centralized traffic governance**: rate limiting, circuit breaking, robots compliance, private-network protection; paid calls get budgets and duplicate-request merging.
- **Hot-reloaded configuration**: strictly validated YAML, changes applied atomically.
- **New sources without writing code**: templates for search, RSS, web lists, and JSON APIs; form editing, bulk enable/disable/delete, per-source proxy; shipped adapters are protected, everything else is editable and deletable.
- **Content processing workflows**: models over API, local endpoints, or CLI; steps compose LLM, template, and HTTP; outputs are written back as lineage-carrying facets; webhook and Feishu notifications.
- **Three public entry points**: fixed `/api/v1` routes, read-only MCP tools, and a SKILL package — scripts and agents need no knowledge of internals.

## Positioning

Tools like Firecrawl, Crawl4AI, and RSSHub solve "extract once" or "emit a feed". ModalPrism covers the full chain — scheduling, access, adaptation, storage, and query — and turns what it collects into durable data assets.

| | ModalPrism | Firecrawl / Crawl4AI | RSSHub | Colly / Crawlee |
|---|---|---|---|---|
| Core stage | Collection + resource store + query | Web → Markdown / structured extraction | Generating RSS feeds for sites | Code-first scraping frameworks |
| Raw data | Preserved verbatim, append-only, replayable | Reduced to Markdown/JSON | Emits feeds | In-memory, transient |
| Lineage | Per-layer producer / version / config | — | — | — |
| Storage | Unified resources + facets, SQLite → PostgreSQL | Bring-your-own sink | — | Bring-your-own sink |
| Query | Filter + full-text + vector DSL | Search / extract endpoints | RSS subscription | — |
| Paid-API governance | Budgets, logical request keys, idempotent receipts | Credit-based (SaaS) | — | DIY |
| Adding sources | YAML declaration, or templates in the console | Templates / code | Code routes | Code |
| Agent interface | Read-only MCP tools · HTTP · SKILL package | API/SDK | Feeds | — |

## Features

### One model for heterogeneous sources
Sources, accessors, and adapters are all declared in YAML; the repository ships 23 source configs (mostly AI/tech sites). Accessors fetch; adapters interpret; they compose freely — the same raw payload can be processed by multiple adapters for different purposes.

### Raw data is append-only and replayable
Accessors never explain content. Saved originals stay byte-for-byte intact, so any capture can be replayed. Recognition and classification results reference the source by byte spans instead of editing it; facets grow as immutable versions; every derived layer records lineage back to the module, version, and config that produced it.

### Strata, the unified resource store
Text, structured objects, images, audio/video are unified as one addressable resource (RID), each with multiple facets (`raw`, `html`, `md`, `struct:article`, `image:cover`, …). Large objects are content-addressed, de-duplicated, and readable by byte range. Adapters have no write access; all writes go through declarative ChangeSets — cursors, data, and task acknowledgments take effect in one commit, or none.

### Filter, full-text, and vector in one query
A typed query DSL fuses metadata filtering, full-text, and vector search over consistent snapshots; business code never writes SQL. Default deployments depend on a single-file SQLite database only; switch to PostgreSQL when needed.

### Centralized egress governance
Every outbound request — including a headless browser's sub-requests — goes through one gate: per-domain rate limiting, adaptive throttle, host circuit breaking, session/proxy binding, SSRF protection, and RFC 9309 `robots.txt` handling. Paid calls have a separate governance layer: logical request keys, budget windows, idempotent receipts.

### Declarative configuration with atomic hot-reload
Sources, accessors, adapters, and policy presets are validated YAML — unknown fields fail validation, config never touches a database. Hot reload builds an immutable generation and switches atomically; in-flight tasks keep the generation they were dispatched with.

### Source authoring without code
Adding a site needs no commit: start from a template (RSS/Atom, web list, search query, JSON API), fill in the address and selectors in the console, and the save goes through the same loader that validates and hot-reloads the directory. Sources, accessors, and adapters all support create, rewrite, enable/disable, delete, and batch operations; each declaration carries a reference count so the blast radius is visible before deleting. Adapters shipped with the code cannot be deleted, but they can be disabled, rewritten, or copied under a new name and changed freely.

### Content processing
Collection is followed by a processing stage: model declarations (`api` / `local` / `cli`) plus workflow declarations (trigger, step sequence, outputs, notifications). A step is an LLM call, a template, or an HTTP request; outputs are written back as new facets with lineage and never touch the original; every run leaves a `struct:workflow-run` record, successful or not. Notifications support webhooks and Feishu bots, testable from the console. See [`docs/specs/content-processing.md`](docs/specs/content-processing.md).

### Read-only surface for agents
Collected data is exposed over MCP: typed queries, health status, source inventory, workflows and runs, and config provenance for any MCP-capable client; scenario-agnostic abstract types (`Article`, `HotList`) provide business views. Except for a recoverable write whitelist (creating, rewriting, and toggling source/accessor/adapter declarations), write methods never appear in the tool list, and the read-only process opens the data directory read-only.

## Architecture

```text
Config (YAML) ─► Scheduler ─► Task queue (Strata) ─► Runner
                                                    │ Accessors (via egress / paid governance)
                                                    ▼
                                             Raw envelope ─► Router ─► Search adapter ─► discoveries
                                                        └─► Content adapter ─► facets
                                                    │ assembled into a ChangeSet
                                                    ▼
                     Strata (relational DB + content-addressed Blob) ─► Abstract types / consumers
```

| Component | Responsibility |
|---|---|
| Scheduler | Emits access tasks from source config, cadence, and health |
| Accessor | Fetches raw data by protocol into a raw envelope; never parses |
| Egress gate | Rate limit, circuit break, robots, SSRF, session & proxy |
| Adapters | Search adapters emit discoveries; content adapters emit facets — both read-only |
| Strata | Owns resources, facets, relations, identity, lineage, state, task queue |
| Abstract types | Business views (`Article`, `HotList`) built on Strata, owning their schema |

Boundaries: only Strata touches storage; accessors reach the network solely through the egress gate; run-time state lives in Strata, separate from read-only config.

### Runtime shapes

| Process | Description | Surface |
|---|---|---|
| `modalprism-collector` | One command: initialize storage, expose the domain API, run the collection and processing loops | Loopback `/api/v1` on `:17880`, forward proxy `:17881`, push `:17882` |
| `modalprism-console` (Node) | Builds and serves the SPA and proxies `/api/v1` byte-for-byte | HTTP, `127.0.0.1:17890` |
| `modalprism-agent` | Read-only surface (query / health / source inventory / workflows & runs / provenance / type views) | MCP stdio, tools named `instance.method` |

## Quick start

### Build

Building requires `protoc` (the Protocol Buffers compiler) on your system; the `frame/port-grpc` build script calls it. On Debian/Ubuntu the package is `protobuf-compiler`.

```sh
cargo build --workspace
```

That is the complete build — no extra flags. Object storage defaults to an external RustFS release (feature `rustfs-bin`): it looks in the configured path, then `<data>/tools/rustfs/<version>/`, then `PATH`, and downloads a prebuilt release only if none is found; you never compile RustFS. Fully offline: set `strata.gateway.rustfs.offline: true` to fail fast instead of waiting on the network.

Optional backend replacements (not needed by default; enable only when deliberately changing the storage shape):

```sh
cargo build -p modalprism-collector --no-default-features   # file BlobStore, no object store
cargo build -p modalprism-collector --features s3           # external S3-compatible service
cargo build -p modalprism-collector --features embedded-s3  # not recommended: in-process RustFS (compiles rustfs; very long build)
```

### Start collecting

Run from the repository root (defaults `--data var --config config`):

```sh
cargo run -p modalprism-collector
```

Listen addresses are individually configurable: `--http 127.0.0.1:17880 --proxy 127.0.0.1:17881 --push 127.0.0.1:17882`. On Ctrl-C or SIGTERM the process releases task leases and exits.

### Start the console

The frontend is Vite + React + TypeScript (pnpm). Build the web assets first, then start the Node shell:

```sh
cd apps/modalprism-console && pnpm install && pnpm build
node apps/modalprism-console/index.mjs --backend http://127.0.0.1:17880 --bind 127.0.0.1:17890
```

Open `http://127.0.0.1:17890`. Navigation has eight pages and is hash-based: `#/overview`, `#/sources`, `#/tasks`, `#/content`, `#/production`, `#/outbound`, `#/logs`, `#/settings`. The Node process only hosts the SPA, serves the SPA fallback, reports Node/collector reachability at `/healthz`, and proxies `/api/v1` without transforming requests or responses.

## Declaring a source

Sources are declarative YAML entries under `config/sources/*.yaml`: what to access, which adapters, at what cadence, under which policy.

```yaml
id: source/openai-news
kind: source
name: "OpenAI News"
labels:
  tier: T1
access:
  accessor: accessor/rss
  locator:
    url: https://openai.com/news/rss.xml
pipeline:
  - run: [adapter/feed-items]
schedule:
  interval: 2h
admission:
  initialBackfill:
    limit: 8
    maxAge: 365d
```

Merge order: common config → type base → policy preset → profile → instance → environment override, then a system-level lock check yields an immutable snapshot.

## Content processing

Workflows and models live under `config/workflows/` and `config/models/`, or can be created in the console's processing page. The smallest workflow:

```yaml
# config/models/digest.yaml
name: Digest model
mode: api
endpoint: https://api.deepseek.com/v1
model: deepseek-chat
keyEnv: DEEPSEEK_API_KEY
```

```yaml
# config/workflows/daily-digest.yaml
name: Daily digest
trigger:
  on: commit
model: digest
steps:
  - id: summary
    kind: llm
    prompt: "Summarize in three sentences:\n{{article.text}}"
outputs:
  facet:
    kind: struct
    variant: digest
    body: '{"summary":"{{steps.summary}}"}'
notify:
  - kind: feishu
    url: https://open.feishu.cn/open-apis/bot/v2/hook/xxxx
    on: [failure]
```

Three model channels: `api` talks to a remote OpenAI-compatible endpoint (only the env var name is declared), `local` targets a loopback or LAN endpoint (Ollama, vLLM, LM Studio), `cli` pipes the prompt into a subprocess (`claude -p`, `llm`). The processing loop advances the cursor every 20 seconds; you can also run one article or one batch manually.

Feishu setup: group bot → custom bot → copy the webhook; pick keyword verification with `ModalPrism` as the keyword (signature verification is not supported). Paste it into the console's processing → notifications view and send a test.

## Public API

Collector owns the single domain contract at `/api/v1`; the Node console proxies it unchanged. Full fields, state machines and OpenAPI are in [`docs/specs/public-api.md`](docs/specs/public-api.md) and [`public-api.openapi.yaml`](docs/specs/public-api.openapi.yaml).

### HTTP

```sh
curl -s http://127.0.0.1:17880/api/v1/system/status
curl -s "http://127.0.0.1:17880/api/v1/catalog/sources?limit=20"
curl -s "http://127.0.0.1:17880/api/v1/content/items?query=agent&limit=10"
curl -s http://127.0.0.1:17880/api/v1/operations/op_01...
curl -s -X POST http://127.0.0.1:17880/api/v1/collection/runs \
  -H 'idempotency-key: manual-1' -H 'content-type: application/json' \
  -d '{"sourceId":"openai-news","mode":"normal"}'
```

Async actions return `202`, an Operation body and `Location`. The same surface covers catalog, collection, content, production, publication, settings, operations, observability and maintenance; there is no generic ability invoke and no raw config-file CRUD.

Projects and the first-run wizard are first-class: resources bind to projects under `/projects/{projectId}/resources`, the wizard's `complete`/`skip` are order-gated (412 `preconditionFailed` when an earlier step is still pending), and `POST /search` accepts an optional `projectId` — the project must exist (404 otherwise) and a successful search is recorded as a `search_record` bound to that project. `POST /fetch` and `POST /parse` round-trip a URL to text and structured extraction; parse returns 412 until the rawprism hook is wired.

### MCP

For any MCP-capable client to query collected data:

```sh
cargo run -p modalprism-agent -- --data var --config config
```

Tools are named `instance.method`, e.g. `strata_read.query`, `type_article.project`, `catalog_admin.list`, `workflow.runs`. Writes are limited to a recoverable whitelist (`catalog_admin.create` / `update` / `toggle`, which write validated declaration files); destructive deletes (`remove`), system settings (`proxy`) and collection-driving writes (`workflow.save`, …) never appear in the tool list, and forcing one is rejected by the gateway.

### SKILL package

`skills/modalprism/` is an Agent Skill: call order, field notes, and task recipes, plus a zero-dependency HTTP client for clients without MCP:

```sh
node skills/modalprism/scripts/prism.mjs health
node skills/modalprism/scripts/prism.mjs articles --source hugging-face --limit 5
node skills/modalprism/scripts/prism.mjs run daily-digest r_01ARZ3NDEKTSV4RRFFQ69G5FAV
```

## Repository layout

```text
frame/                     LEAF framework runtime & protocol ports (vendored, pinned upstream)
SDK/                       LEAF ability SDKs (Rust / Python / TypeScript)
apps/ability-core/         LEAF ability host
apps/modalprism-domain/    strata-model · strata-sqlite-vec · strata-gateway · strata-client · config · types · surface · workflow · api
apps/modalprism-collector/ Collection process (ability/ = in-proc abilities; subsystems/ = accessors, adapters, egress, …)
apps/modalprism-agent/     Read-only MCP process
apps/modalprism-console/   Single package: Vite React SPA + Node SPA host / transparent `/api/v1` proxy
abilitys/                  Shared abilities: strata_read · config_inspect · health · type_article · type_hotlist · catalog_admin · workflow
config/                    System config, accessors, adapters, sources, presets, schemas (YAML / JSON)
docs/                      Design document & implementation specs
manual/                    LEAF framework manual
skills/                    Agent Skill package (SKILL.md + zero-dependency HTTP script)
```

Dependency boundaries: domain libraries depend on none of `frame-core` / `ability-core` / `ability-support` / ports; ability crates depend only on `ability-support`; the composition root depends on `frame-core` and `ability-core`.

## Development

Tests and the clippy gate cover only this project's crates; `frame/`, `SDK/`, and `apps/ability-core` are vendored LEAF code compiled as-is from upstream (their own upstream lints are outside the gate), hence `--no-deps` for clippy.

```sh
cargo test --workspace --exclude 'frame-*' --exclude 'ability-support*' --exclude ai-protocol --exclude ability-core
cargo clippy --workspace --all-targets --no-deps --exclude 'frame-*' --exclude 'ability-support*' --exclude ai-protocol --exclude ability-core -- -D warnings
```

The workspace forbids `unsafe_code`; releases build with `lto = true`, `codegen-units = 1`. The frontend is built with pnpm under `apps/modalprism-console` (`pnpm build` then `pnpm start`).

## License

Apache-2.0
