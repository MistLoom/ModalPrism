//! 由 Expose 计划推导调用链 InvokeMask（不触发 load/unload）。

use std::collections::BTreeSet;

use ability_support::InvokeMask;

use crate::AbilityExpose;

/// 按 Expose 解析结果生成 allow 列表；仅含 `instances` 中且 enabled 的实例。
pub fn invoke_mask_from_expose(expose: &AbilityExpose, instances: &[&str]) -> InvokeMask {
    let allow: BTreeSet<String> = instances
        .iter()
        .copied()
        .filter(|id| expose.resolve(id).enabled)
        .map(str::to_string)
        .collect();
    InvokeMask {
        allow: Some(allow),
        deny: BTreeSet::new(),
    }
}
