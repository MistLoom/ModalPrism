# Catalog, protocols & expose

English · [中文](catalog-and-expose.zh-CN.md)

Scenario apps assemble Abilities in three steps: discover sources, decide which instances to load (`AbilityExpose`), then open Ports on the resulting `Gateway`.

The framework scans `ability.yml` under repo-root `abilitys/` and each `apps/<app>/ability/`. Shared Abilities are callable, listenable, and interceptable by the entry app. Each source records its owning app and `open`. Set the process entry app with `AppBuilder::app` or `ComposeOptions::app`. A call into another app requires `open: true`. Listen and intercept go through `AppHandle::control`.

## Protocol declaration

Each Ability package declares supported protocols in `ability.yml`. Protocol directories (`mcp/`, `cli/`, `tools/`, …) hold projection metadata when required.

```yaml
framework_version: "1"
id: read_file
protocols: [tools, mcp, cli, http, fun]
methods:
  - id: run
    description: Read a file
    input:
      type: object
      properties:
        path: { type: string }
      required: [path]
```

| Protocol | Declaration | Role |
|---|---|---|
| `tools` | `protocols` (+ optional `tools/info.yml`) | LLM tool discovery / `dispatch` |
| `mcp` | `protocols` + `mcp/info.yml` | MCP tool projection |
| `cli` | `protocols` + `cli/info.yml` | CLI port / standalone CLI mapping |
| `grpc` | `protocols` + `grpc/info.yml` | gRPC mapping |
| `skill` | `protocols` + `skill/info.yml` | Skill metadata |
| `http` / `fun` | `protocols` only | HTTP / in-process Fun |

## Workspace catalog by protocol

```rust
use ability_support::ProtocolKind;
use frame_compose::WorkspaceCatalog;

let catalog = WorkspaceCatalog::discover();

let tools = catalog.sources_by_protocol(ProtocolKind::Tools);
let mcp = catalog.sources_by_protocol(ProtocolKind::Mcp);

let linked_tools: Vec<_> = tools
    .into_iter()
    .filter(|s| s.linked)
    .collect();
```

`SourceEntry` fields used here: `id`, `rel_root`, `root`, `linked` (whether the current binary links the Provider).

## Expose filters

```rust
use frame_core::AbilityExpose;
use serde_json::json;

let expose = AbilityExpose::from_value(&json!({
    "unknown": "deny",
    "defaults": { "enabled": true },
    "abilities": {
        "write_file": { "enabled": false },
        "read_file": {
            "enabled": true,
            "config": { "max_size": 1_048_576 }
        }
    }
}))?;

assert!(expose.is_enabled("read_file"));
assert!(!expose.is_enabled("write_file"));

let enabled = expose.filter_enabled(["read_file", "write_file", "bash"]);
// unknown=deny ⇒ unlisted ids are off ⇒ ["read_file"]
```

| API | Behavior |
|---|---|
| `AbilityExpose::from_value` | Parse expose JSON (`abilities` map; legacy keys accepted) |
| `AbilityExpose::open_all` | Default: enable unspecified sources |
| `is_enabled` / `resolve` | Per-instance enabled flag + bind-time `config` |
| `filter_enabled` | Keep ids that resolve to enabled |

Expose gates **Port → Gateway** load/visibility. Internal `ctx.call` is not limited by Expose.

## Combine protocol list and expose

```rust
use ability_support::ProtocolKind;
use frame_compose::{AppBuilder, WorkspaceCatalog};
use frame_core::AbilityExpose;
use serde_json::json;

let catalog = WorkspaceCatalog::discover();
let expose = AbilityExpose::from_value(&json!({
    "unknown": "deny",
    "defaults": { "enabled": true },
    "abilities": {
        "write_file": { "enabled": false },
        "bash": { "enabled": false }
    }
}))?;

let mcp_surface: Vec<_> = catalog
    .sources_by_protocol(ProtocolKind::Mcp)
    .into_iter()
    .filter(|s| s.linked && expose.is_enabled(&s.id))
    .map(|s| s.id.clone())
    .collect();

let tools_surface: Vec<_> = catalog
    .sources_by_protocol(ProtocolKind::Tools)
    .into_iter()
    .filter(|s| s.linked && expose.is_enabled(&s.id))
    .map(|s| s.id.clone())
    .collect();

let app = AppBuilder::new()
    .catalog(catalog)
    .expose(expose)
    .bootstrap()
    .await;
```

After bootstrap, Ports apply an access gate: only loaded instances that registered the Port’s protocol remain visible on that surface.

## Related

- [Getting started](getting-started.md)
- [Write an Ability](writing-abilities.md)
