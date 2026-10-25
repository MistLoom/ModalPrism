//! 声明式目录的管理面。
//!
//! 信源、访问器与适配器都写在配置目录里，这里只做三件事：给出模板、把草稿写盘、
//! 用同一套加载器校验。界面和命令行因此不需要编译就能长出新形态，删除与启停也走同一处。
//! 判定「内置」的规则在 [`config`] 内，不在本层重复。

mod logic;

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec, ProtocolKind};
use serde_json::Value;

/// 新建与改写共用同一份草稿形状：`{kind, slug, doc}`。
fn draft() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "kind": { "type": "string", "enum": ["source", "accessor", "adapter"] },
            "slug": { "type": "string" },
            "doc": { "type": "object" }
        },
        "required": ["kind", "slug", "doc"]
    })
}

pub fn provider(ctx: Arc<dyn types::ReadCtx>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("catalog_admin").protocols(&[
        ProtocolKind::Fun,
        ProtocolKind::Http,
        ProtocolKind::Mcp,
        ProtocolKind::Tools,
    ]);
    spec = spec.on(
        "templates",
        "列出新建模板",
        serde_json::json!({"type": "object"}),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::templates::run(ctx, args))
            }
        },
    );
    spec = spec.on(
        "list",
        "列出三类声明与引用计数",
        serde_json::json!({"type": "object"}),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::list::run(ctx, args))
            }
        },
    );
    spec = spec.on("create", "新建一份声明并立即校验", draft(), {
        let ctx = Arc::clone(&ctx);
        move |_ctx, args| {
            let ctx = Arc::clone(&ctx);
            ability_support::box_fut(logic::create::run(ctx, args))
        }
    });
    spec = spec.on("update", "改写一份声明并立即校验", draft(), {
        let ctx = Arc::clone(&ctx);
        move |_ctx, args| {
            let ctx = Arc::clone(&ctx);
            ability_support::box_fut(logic::update::run(ctx, args))
        }
    });
    spec = spec.on(
        "proxy",
        "设置系统级出站代理",
        serde_json::json!({
            "type": "object",
            "properties": { "value": { "type": "string" } }
        }),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::proxy::run(ctx, args))
            }
        },
    );
    spec = spec.on(
        "remove",
        "删除一份声明",
        serde_json::json!({
            "type": "object",
            "properties": {
                "kind": { "type": "string", "enum": ["source", "accessor", "adapter"] },
                "slug": { "type": "string" }
            },
            "required": ["kind", "slug"]
        }),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::remove::run(ctx, args))
            }
        },
    );
    spec = spec.on(
        "toggle",
        "启用或停用一份声明",
        serde_json::json!({
            "type": "object",
            "properties": {
                "kind": { "type": "string", "enum": ["source", "accessor", "adapter"] },
                "slug": { "type": "string" },
                "enabled": { "type": "boolean" }
            },
            "required": ["kind", "slug", "enabled"]
        }),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::toggle::run(ctx, args))
            }
        },
    );
    spec = spec.on(
        "batch",
        "批量启用、停用或删除同类声明",
        serde_json::json!({
            "type": "object",
            "properties": {
                "kind": { "type": "string", "enum": ["source", "accessor", "adapter"] },
                "action": { "type": "string", "enum": ["enable", "disable", "remove"] },
                "slugs": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["kind", "action", "slugs"]
        }),
        {
            let ctx = Arc::clone(&ctx);
            move |_ctx, args| {
                let ctx = Arc::clone(&ctx);
                ability_support::box_fut(logic::batch::run(ctx, args))
            }
        },
    );
    spec.into_provider()
}

/// 声明写错是改配置就能恢复的错误，读写失败才需要重试。
pub(crate) fn fail(error: config::Error) -> ability_support::Error {
    match error {
        error @ config::Error::Invalid(_) => {
            ability_support::Error::invalid_args(error.to_string())
        }
        error => ability_support::Error::failed(error.to_string()),
    }
}

pub(crate) fn text(args: &Value, key: &str) -> ability_support::Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ability_support::Error::invalid_args(format!("缺少 {key}")))
}

pub(crate) fn flag(args: &Value, key: &str) -> ability_support::Result<bool> {
    args.get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| ability_support::Error::invalid_args(format!("缺少 {key}")))
}
