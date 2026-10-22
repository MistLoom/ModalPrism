//! 生成 `generated/bridge.rs`：将 logic 模块桥接到 Handlers trait。

use ability_support::{AbilityDecl, AbilityPackageMeta};

use super::handlers::method_fn_name;

pub fn generate(meta: &AbilityPackageMeta) -> Result<String, String> {
    let mut methods = String::new();
    for m in &meta.decl.methods {
        let mod_name = AbilityDecl::logic_module_name(&m.id);
        let fn_name = method_fn_name(&m.id);
        methods.push_str(&format!(
            r#"
    async fn {fn_name}(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value> {{
        crate::logic::{mod_name}::{fn_name}(ctx, args).await
    }}
"#,
            fn_name = fn_name,
            mod_name = mod_name,
        ));
    }

    Ok(format!(
        r#"// 自动生成，勿手改
use std::sync::Arc;

use async_trait::async_trait;

use super::handlers_trait::Handlers;

pub struct Bridge;

#[async_trait]
impl Handlers for Bridge {{
{methods}
}}

pub fn instance() -> Arc<dyn Handlers> {{
    Arc::new(Bridge)
}}
"#,
        methods = methods,
    ))
}
