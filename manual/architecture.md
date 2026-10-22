# Architecture

English · [中文](architecture.zh-CN.md)

## Model

An **app** owns its Abilities. Ports and cross-ability calls share one **Gateway** envelope: `instance + method + args`.

```
Ports (CLI / HTTP / MCP / Fun / …)
        │
     Gateway
        │
  entry app
        ├─ owned Abilities
        └─ open Abilities of other apps
```

`ability.yml` field `open` defaults to false. Another app may call the Ability only when `open: true`.

Event listen and intercept are registered through `AppControl` on the entry app. Intercept applies to Abilities that app owns. Listen also accepts events from other apps' open Abilities. The engine implements the filter; apps do not keep a second bus.

## Layers

| Layer | Role |
|---|---|
| Contract | Manifest, Ctx, lifecycle, client codegen |
| Transport | Gateway, Ports, Expose, controllers |
| Engine | Host, event bus, admission for calls and events |
| Composition | Discovery, bootstrap, `AppControl` |
| Abilities | Shared under `abilitys/<category>/<name>` at repo root; app-owned under `apps/<app>/ability/<name>` |
| Apps | Scenario processes and assembly policy |

Node / Python Abilities use the **guest wire protocol** with the same `ability.yml` model as Rust. See [Guest protocol](ability-guest-protocol.zh-CN.md).

## Visibility and governance

- **Shared Abilities**: callable, listenable, and interceptable by the entry app (exact set is assembly-defined).
- **App-owned Abilities**: owner-only by default; `open: true` allows other apps to call or listen.
- **Out-of-tree packages**: attach via Guest SDK under repo-root `SDK/` and the wire protocol — not by linking the runtime kernel.

## Rules

1. Ports depend on `Gateway`; they do not call Ability implementations directly
2. Abilities do not branch on whether the caller was MCP, CLI, or LLM tools
3. Expose gates **Port → Gateway** only; it does not grant a cross-app call
4. A cross-app call requires `open: true` on the target
5. Intercept stays with the owning app; listening to another app requires `open: true`
6. Protocol identity lives in declarations and Ports — not as kernel business types
