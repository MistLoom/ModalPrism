# Getting started

English · [中文](getting-started.zh-CN.md)

## Requirements

- Recent Rust toolchain
- Build and run from the project root

## Create an application

Add an application crate, depend on the runtime composition library, and bootstrap. The app name matches the entry app in assembly config.

```rust
use frame_compose::AppBuilder;

let app = AppBuilder::new()
    .app("my-app")
    .bootstrap()
    .await;
```

`AppHandle` exposes `gateway`, `host`, `expose`, `catalog`, and `control()`.

## Invoke

```rust
use frame_core::{Gateway, InstanceId, InvokeRequest, MethodId};
use serde_json::json;

let response = app
    .gateway
    .invoke(InvokeRequest::new(
        InstanceId::new("read_file")?,
        MethodId::new("run")?,
        json!({ "path": "README.md" }),
    ))
    .await?;
```

Same-app `ctx.call` is not gated by `AbilityExpose`. A call into another app requires `open: true` on the target. Default assembly links shared Abilities from repo-root `abilitys/`.

## Subscribe

```rust
use ability_support::{EventClass, EventFilter, EventListener};

let mut events = app
    .control()
    .listen(EventFilter {
        class: Some(EventClass::Ability),
        ..Default::default()
    })
    .await?;

while let Some(event) = events.next().await {
    println!("{} {}", event.name(), event.payload());
}
```

`listen` delivers events from this app, from shared Abilities, and from other apps' open Abilities. `intercept` applies to this app and to shared Abilities.

## Examples

The repository ships runnable sample apps (agent and web scenarios). From the root: `cargo run -p <app-crate> -- …`. Crate names are listed in workspace members.

Optional ecosystem plugins: [Ecosystem plugins](ecosystem-plugins.md).

## Next

- [Catalog & expose](catalog-and-expose.md) — protocol lists and instance filters in code
- [Write an Ability](writing-abilities.md)
