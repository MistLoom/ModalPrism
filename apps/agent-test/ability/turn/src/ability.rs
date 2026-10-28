//! Ability `turn`：agent 回合循环。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ability_support::{
    Ability, AbilityCtx, AbilityManifest, AbilityProvider, AbilitySpec, Error, MethodId, Result,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::generated::manifest;
use crate::loop_runner::{RunInput, SpawnInput, TurnConfig, run_turn, spawn_turn};

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::from_manifest(manifest::manifest()).into_factory(|config| async move {
        let max_rounds = config
            .get("max_rounds")
            .and_then(Value::as_u64)
            .map(|n| n as u32)
            .unwrap_or(32)
            .max(1);
        let system_prompt = config
            .get("system_prompt")
            .and_then(Value::as_str)
            .map(str::to_string);
        Ok(Arc::new(TurnAbility {
            manifest: manifest::manifest(),
            config: TurnConfig {
                max_rounds,
                system_prompt,
            },
            cancels: Arc::new(Mutex::new(HashMap::new())),
        }) as Arc<dyn Ability>)
    })
}

struct TurnAbility {
    manifest: AbilityManifest,
    config: TurnConfig,
    cancels: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

#[derive(Deserialize)]
struct RunArgs {
    session_id: Option<String>,
    text: String,
    #[serde(default)]
    create_session: bool,
    #[serde(default)]
    skill_paths: Vec<String>,
    skill_root: Option<String>,
}

#[derive(Deserialize)]
struct CancelArgs {
    turn_id: String,
}

#[derive(Deserialize)]
struct SpawnArgs {
    session_id: String,
    prompt: String,
    #[serde(default = "default_true")]
    summarize: bool,
}

fn default_true() -> bool {
    true
}

#[async_trait]
impl Ability for TurnAbility {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn invoke(&self, method: &MethodId, ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
        match method.as_str() {
            "run" => {
                let a: RunArgs =
                    serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
                let turn_id = uuid::Uuid::new_v4().to_string();
                let cancel = Arc::new(AtomicBool::new(false));
                {
                    let mut g = self.cancels.lock().expect("turn cancels");
                    g.insert(turn_id.clone(), Arc::clone(&cancel));
                }
                let out = run_turn(
                    ctx,
                    &self.config,
                    cancel,
                    &turn_id,
                    RunInput {
                        session_id: a.session_id,
                        text: a.text,
                        create_session: a.create_session,
                        skill_paths: a.skill_paths,
                        skill_root: a.skill_root,
                    },
                )
                .await;
                {
                    let mut g = self.cancels.lock().expect("turn cancels");
                    g.remove(&turn_id);
                }
                let out = out?;
                Ok(json!({
                    "session_id": out.session_id,
                    "turn_id": out.turn_id,
                    "reason": out.reason,
                    "text": out.text,
                    "rounds": out.rounds,
                }))
            }
            "cancel" => {
                let a: CancelArgs =
                    serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
                let g = self.cancels.lock().expect("turn cancels");
                if let Some(flag) = g.get(&a.turn_id) {
                    flag.store(true, Ordering::Relaxed);
                    Ok(json!({ "ok": true }))
                } else {
                    Ok(json!({ "ok": false, "reason": "turn_not_found" }))
                }
            }
            "spawn" => {
                let a: SpawnArgs =
                    serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
                let turn_id = uuid::Uuid::new_v4().to_string();
                let cancel = Arc::new(AtomicBool::new(false));
                {
                    let mut g = self.cancels.lock().expect("turn cancels");
                    g.insert(turn_id.clone(), Arc::clone(&cancel));
                }
                let out = spawn_turn(
                    ctx,
                    &self.config,
                    cancel,
                    &turn_id,
                    SpawnInput {
                        session_id: a.session_id,
                        prompt: a.prompt,
                        summarize: a.summarize,
                    },
                )
                .await;
                {
                    let mut g = self.cancels.lock().expect("turn cancels");
                    g.remove(&turn_id);
                }
                let out = out?;
                Ok(json!({
                    "parent_session_id": out.parent_session_id,
                    "child_session_id": out.child_session_id,
                    "turn_id": out.turn_id,
                    "reason": out.reason,
                    "text": out.text,
                }))
            }
            other => Err(Error::not_found(other)),
        }
    }
}
