use std::collections::HashMap;

use ability_support::{CatalogEntry, Error, ErrorKind, InstanceId, MethodId};

use crate::bridge::{discovery_name, discovery_to_function_name};

/// function name (instance, method)
#[derive(Clone, Debug, Default)]
pub struct NameMap {
    to_target: HashMap<String, (InstanceId, MethodId)>,
}

impl NameMap {
    pub fn from_catalog(catalog: &[CatalogEntry]) -> Self {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for e in catalog {
            *counts.entry(e.instance.to_string()).or_insert(0) += 1;
        }
        let mut to_target = HashMap::new();
        for e in catalog {
            if e.ability_id.as_str() == "dispatch" {
                continue;
            }
            let name = discovery_to_function_name(&discovery_name(e, &counts));
            to_target.insert(name, (e.instance.clone(), e.method.clone()));
        }
        Self { to_target }
    }

    pub fn resolve(&self, function_name: &str) -> Result<&(InstanceId, MethodId), Error> {
        self.to_target.get(function_name).ok_or_else(|| {
            Error::from_ext(ErrorKind::NotFound, format!("工具名称未映 {function_name}"))
        })
    }
}
