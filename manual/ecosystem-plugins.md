# Ecosystem plugins

English · [中文](ecosystem-plugins.zh-CN.md)

Optional Abilities. They are not part of the default assembly. `claude_mod` and `dsh_plugin` are open shared Abilities that share one Node plugin host. Sample apps in the repository show how to enable them.

LEAF does not embed a Claude Code or DSH runtime. Enable the two Abilities in assembly config. Each keeps its own plugin list. When both are enabled they share one Node host process.

| Ability | Plugins |
|---|---|
| `claude_mod` | Claude Code mods (`hooks/hooks.json`) |
| `dsh_plugin` | DSH plugins (single-file scripts or Cordis packages) |

Enable the matching composition features in dependencies, set plugin paths in Expose `config.plugins`, and run `doctor` to verify Node and paths.

## Invoke

`claude_mod` forwards session and turn events to mod hooks (for example `session.created` → `session.start`). Direct invoke:

```rust
use frame_core::{Gateway, InstanceId, InvokeRequest, MethodId};
use serde_json::json;

let _ = gateway
    .invoke(InvokeRequest::new(
        InstanceId::new("claude_mod")?,
        MethodId::new("invoke")?,
        json!({
            "tool": "session.start",
            "args": { "session_id": "s1" }
        }),
    ))
    .await?;
```

Tools registered by a DSH plugin appear on `dispatch` under names that start with `eco__dsh__`, and run through `dispatch.run` when the model calls them in the current turn.
