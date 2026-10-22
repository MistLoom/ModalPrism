//! 调用链可见性策略：裁剪 `ctx.catalog` / `ctx.call` 可达实例。
//!
//! 与 [`AbilityExpose`](crate) 分离：Expose 管装载与 Port→Gateway；
//! InvokeMask 仅在单次调用链内生效，不 unload 实例、无全局竞态。

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::DiscoveryPayload;

/// 调用链内实例可见性。默认不限制。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeMask {
    /// 若非空，仅允许集合内实例 id（`instance` 字段）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow: Option<BTreeSet<String>>,
    /// 始终拒绝的实例 id。
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub deny: BTreeSet<String>,
}

impl InvokeMask {
    pub fn allow_all() -> Self {
        Self::default()
    }

    pub fn allow_only(instances: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            allow: Some(instances.into_iter().map(Into::into).collect()),
            deny: BTreeSet::new(),
        }
    }

    pub fn allows(&self, instance: &str) -> bool {
        if self.deny.contains(instance) {
            return false;
        }
        match &self.allow {
            None => true,
            Some(set) => set.contains(instance),
        }
    }

    pub fn filter_discoveries(&self, items: Vec<DiscoveryPayload>) -> Vec<DiscoveryPayload> {
        if self.is_default() {
            return items;
        }
        items
            .into_iter()
            .filter(|d| self.allows(d.instance.as_str()))
            .collect()
    }

    pub fn is_default(&self) -> bool {
        self.allow.is_none() && self.deny.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AbilityId, InstanceId, Lifecycle, MethodMeta};

    fn disc(id: &str) -> DiscoveryPayload {
        DiscoveryPayload {
            instance: InstanceId::new(id).unwrap(),
            ability_id: AbilityId::new(id).unwrap(),
            phase: Lifecycle::Ready,
            stateful: false,
            methods: vec![MethodMeta::new("run", "", serde_json::json!({}))],
            events: vec![],
        }
    }

    #[test]
    fn allow_list_filters() {
        let mask = InvokeMask::allow_only(["read_file", "session"]);
        assert!(mask.allows("read_file"));
        assert!(!mask.allows("write_file"));
        let filtered = mask.filter_discoveries(vec![disc("read_file"), disc("write_file")]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].instance.as_str(), "read_file");
    }
}
