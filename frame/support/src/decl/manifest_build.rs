//! 从 `ability.yml` 声明构造运行时 manifest。

use crate::AbilityId;
use crate::manifest::{AbilityManifest, EventDecl, MethodMeta};

use super::ability::{AbilityDecl, DepDecl};

pub fn manifest_from_decl(decl: &AbilityDecl) -> Result<AbilityManifest, String> {
    let id = AbilityId::new(&decl.id).map_err(|e| e.to_string())?;
    let depends = decl
        .depends
        .iter()
        .cloned()
        .map(DepDecl::into_dependency)
        .collect::<Result<Vec<_>, _>>()?;
    let methods = decl
        .methods
        .iter()
        .map(|m| {
            let meta = MethodMeta::new(&m.id, m.description.clone(), m.input.clone());
            if let Some(out) = &m.output {
                meta.with_output(out.clone())
            } else {
                meta
            }
        })
        .collect();
    let events = decl
        .events
        .iter()
        .map(|e| e.clone().into_event_decl())
        .collect::<Vec<EventDecl>>();
    Ok(AbilityManifest {
        id,
        depends,
        stateful: decl.stateful,
        config_schema: decl.config_schema.clone(),
        methods,
        events,
    })
}
