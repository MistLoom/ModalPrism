//! `probe.note`：带 config，由 hub `ctx.load` 拉起；`write` 发 `noted`。

use std::sync::Arc;

use ability_sdk::prelude::*;
use ability_sdk::{
    Ability, AbilityId, AbilityManifest, EventDecl, EventDir, FactoryProvider, MethodId, MethodMeta,
};
use async_trait::async_trait;

pub fn note_provider() -> Arc<dyn AbilityProvider> {
    FactoryProvider::new(note_manifest(), |config| async move {
        Ok(Arc::new(Note {
            manifest: note_manifest(),
            config,
        }) as Arc<dyn Ability>)
    })
}

fn note_manifest() -> AbilityManifest {
    AbilityManifest {
        id: AbilityId::new("probe.note").expect("id"),
        depends: vec![],
        stateful: true,
        config_schema: json!({
            "type": "object",
            "properties": { "label": { "type": "string" } }
        }),
        methods: vec![MethodMeta {
            id: MethodId::new("write").expect("m"),
            description: "写入并发 noted".into(),
            input_schema: json!({ "type": "object" }),
            output_schema: None,
        }],
        events: vec![EventDecl {
            name: "noted".into(),
            dir: EventDir::Emit,
            description: String::new(),
            payload_schema: json!({ "type": "object" }),
        }],
    }
}

struct Note {
    manifest: AbilityManifest,
    config: Value,
}

#[async_trait]
impl Ability for Note {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn invoke(&self, method: &MethodId, ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
        if method.as_str() != "write" {
            return Err(Error::not_found(method));
        }
        let out = json!({ "config": self.config, "args": args });
        emit!(ctx.noted(out.clone())).await?;
        Ok(out)
    }
}
