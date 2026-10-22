//! 生成 provider.rs。

use ability_support::AbilityPackageMeta;

use super::handlers::method_fn_name;

pub fn generate(meta: &AbilityPackageMeta) -> Result<String, String> {
    let decl = &meta.decl;
    let mut spec_chain = String::from("AbilitySpec::try_new(ABILITY_ID).expect(\"id\")");

    if decl.stateful {
        spec_chain.push_str("\n        .stateful()");
    }

    for dep in &decl.depends {
        let d = dep.clone().into_dependency()?;
        let id = d.id.to_string();
        if let Some(alias) = &d.alias {
            spec_chain.push_str(&format!("\n        .depends_alias(\"{id}\", \"{alias}\")"));
        } else if d.optional {
            spec_chain.push_str(&format!("\n        .depends_optional(\"{id}\")"));
        } else {
            spec_chain.push_str(&format!("\n        .depends_id(\"{id}\")"));
        }
    }

    let mut on_calls = String::new();
    for m in &decl.methods {
        let input = serde_json::to_string(&m.input).map_err(|e| e.to_string())?;
        let desc = m.description.replace('"', "\\\"");
        on_calls.push_str(&format!(
            r#"
        .on(
            "{mid}",
            "{desc}",
            serde_json::json!({input}),
            handler_impl!({fn_name}),
        )"#,
            mid = m.id,
            desc = desc,
            input = input,
            fn_name = method_fn_name(&m.id),
        ));
    }

    Ok(format!(
        r#"// 自动生成，勿手改
use std::sync::Arc;

use ability_support::{{AbilityProvider, AbilitySpec}};

use super::bridge::instance;
use super::manifest::{{ABILITY_ID, DECL_FRAMEWORK_VERSION}};

macro_rules! handler_impl {{
    ($method:ident) => {{
        move |ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value| {{
            let handlers = instance();
            ability_support::box_fut(async move {{
                handlers.$method(ctx, args).await
            }})
        }}
    }};
}}

pub fn provider() -> Arc<dyn AbilityProvider> {{
    assert_eq!(
        DECL_FRAMEWORK_VERSION,
        ability_support::FRAMEWORK_VERSION,
        "ability.yml framework_version 与当前框架不匹配"
    );
    {spec_chain}{on_calls}
        .into_provider()
}}
"#,
        spec_chain = spec_chain,
        on_calls = on_calls,
    ))
}

pub fn generate_manifest_only(_meta: &AbilityPackageMeta) -> Result<String, String> {
    Ok(r#"// 自动生成，勿手改
pub use super::manifest::{manifest as ability_manifest, ABILITY_ID};
"#
    .to_string())
}
