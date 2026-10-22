# Write an Ability

English · [中文](writing-abilities.zh-CN.md)

## Package layout

One directory per Ability with `ability.yml` at the root. Typical Rust package:

```
ability.yml
build.rs
src/
  logic/          # business code
  generated/      # declaration-driven codegen
cli|mcp|tools|grpc|skill/
  info.yml        # protocol projection (when needed)
```

Shared Abilities live under `abilitys/<category>/<name>`; app-owned under `apps/<app>/ability/<name>`. Package layout is the same.

## Handler and factory

- **Handler** (default): one `src/logic/<method>.rs` per method; export `generated::provider::provider`
- **Factory** (`factory: true`): custom `Ability` + `into_factory` when bind-time config constructs the instance

## Minimal declaration

```yaml
framework_version: "1"
id: echo
protocols: [tools, mcp, cli, http, fun]
depends: []
stateful: false
methods:
  - id: run
    description: Echo text
    input:
      type: object
      properties:
        text: { type: string }
      required: [text]
```

Keep `framework_version` aligned with the framework. Protocol `info.yml` files, when present, must match.

## Visibility

`open` defaults to false: only the owning app may call the Ability. Set `open: true` when other apps may call it. Event intercept stays with the owner. Other apps may listen to an open Ability through `AppControl`.

## Cross-ability calls

Depend on the peer **client** contract only:

```rust
use ability_echo::EchoExt;

ctx.echo().run(args).await?;
```

## Surface checklist

| Goal | Declare |
|---|---|
| LLM tools | `tools` in `protocols` |
| MCP | `mcp` + `mcp/info.yml` |
| CLI | `cli` + `cli/info.yml` |
| HTTP / Fun | `http` / `fun` in `protocols` |

Enable the instance through `AbilityExpose` in the app. See [Catalog & expose](catalog-and-expose.md).
