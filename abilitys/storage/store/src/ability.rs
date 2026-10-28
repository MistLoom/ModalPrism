//! Ability `store`

use std::sync::Arc;

use ability_support::{
    Ability, AbilityCtx, AbilityManifest, AbilityProvider, AbilitySpec, Error, Lifecycle, MethodId,
    Result,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::engine::Engine;
use crate::generated::manifest;
use crate::types::Space;

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::from_manifest(manifest::manifest()).into_factory(|config| async move {
        Ok(Arc::new(StoreAbility {
            manifest: manifest::manifest(),
            engine: Engine::from_config(&config)?,
        }) as Arc<dyn Ability>)
    })
}

struct StoreAbility {
    manifest: AbilityManifest,
    engine: Arc<Engine>,
}

#[derive(Deserialize)]
struct Common {
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    space: Option<String>,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    codec: Option<String>,
    #[serde(default)]
    prefix: Option<String>,
    #[serde(default)]
    value: Option<Value>,
    #[serde(default)]
    patch: Option<Value>,
    #[serde(default)]
    data: Option<String>,
    #[serde(default)]
    append: Option<bool>,
    #[serde(default)]
    offset: Option<u64>,
    #[serde(default)]
    len: Option<u64>,
    #[serde(default)]
    last: Option<bool>,
    #[serde(default)]
    token: Option<String>,
}

fn owner_of(ctx: &dyn AbilityCtx, explicit: Option<&str>) -> String {
    match explicit {
        Some(o) if !o.is_empty() => o.to_string(),
        _ => crate::caller_owner(ctx),
    }
}

fn to_json<T: serde::Serialize>(v: T) -> Result<Value> {
    serde_json::to_value(v).map_err(|e| Error::failed(e.to_string()))
}

#[async_trait]
impl Ability for StoreAbility {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn on_lifecycle(&self, phase: Lifecycle, ctx: &dyn AbilityCtx) -> Result<()> {
        match phase {
            Lifecycle::Started => {
                if let Some(mut rx) = self.engine.take_watch_rx() {
                    let ctx = ctx.retain();
                    tokio::spawn(async move {
                        while let Some(payload) = rx.recv().await {
                            let _ = ctx.emit("changed", payload).await;
                        }
                    });
                }
                Ok(())
            }
            Lifecycle::Stopping => {
                self.engine.stop_watch();
                Ok(())
            }
            _ => Ok(()),
        }
    }

    async fn invoke(&self, method: &MethodId, ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
        let a: Common =
            serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
        let space = Space::parse(a.space.as_deref()).map_err(Error::invalid_args)?;
        let owner = owner_of(ctx, a.owner.as_deref());
        let key = a.key.as_deref().unwrap_or("");
        let codec = a.codec.as_deref();
        match method.as_str() {
            "get" => to_json(self.engine.get(space, &owner, key, codec)?),
            "put" => {
                let v = a.value.ok_or_else(|| Error::invalid_args("缺少 value"))?;
                to_json(self.engine.put(space, &owner, key, &v, codec)?)
            }
            "merge" => {
                let p = a.patch.ok_or_else(|| Error::invalid_args("缺少 patch"))?;
                to_json(self.engine.merge(space, &owner, key, p, codec)?)
            }
            "delete" => to_json(self.engine.delete(space, &owner, key, codec)?),
            "exists" => to_json(self.engine.exists(space, &owner, key, codec)?),
            "stat" => to_json(self.engine.stat(space, &owner, key, codec)?),
            "list" => to_json(self.engine.list(
                space,
                &owner,
                a.prefix.as_deref().unwrap_or(""),
            )?),
            "purge" => to_json(self.engine.purge(
                space,
                &owner,
                a.prefix.as_deref().unwrap_or(key),
            )?),
            "blob_get" => {
                to_json(
                    self.engine
                        .blob_get(space, &owner, key, a.offset.unwrap_or(0), a.len)?,
                )
            }
            "blob_put" => {
                let data = a
                    .data
                    .as_deref()
                    .ok_or_else(|| Error::invalid_args("缺少 data"))?;
                to_json(self.engine.blob_put(
                    space,
                    &owner,
                    key,
                    data,
                    a.append.unwrap_or(false),
                )?)
            }
            "blob_write" => {
                let data = a
                    .data
                    .as_deref()
                    .ok_or_else(|| Error::invalid_args("缺少 data"))?;
                to_json(self.engine.blob_write(
                    space,
                    &owner,
                    key,
                    a.offset.unwrap_or(0),
                    data,
                    a.last.unwrap_or(false),
                )?)
            }
            "acquire" => to_json(self.engine.acquire(space, &owner, key)?),
            "release" => {
                let token = a
                    .token
                    .as_deref()
                    .ok_or_else(|| Error::invalid_args("缺少 token"))?;
                to_json(self.engine.release(token)?)
            }
            other => Err(Error::not_found(other)),
        }
    }
}
