//! 内容加工的管理与执行面。
//!
//! 方法与界面一一对应：声明（`save`/`remove`/`toggle`）、模型（`save_model`/`remove_model`）、
//! 运行（`run`/`tick`/`runs`）与通知（`notify_test`）。引擎本身在 `workflow` crate 内，
//! 这里只负责参数形状与错误码，不重复任何加工逻辑。

mod logic;

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec, ProtocolKind};
use serde_json::Value;

/// 单条工作流/模型声明的定位键。
fn slug() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": { "slug": { "type": "string" } },
        "required": ["slug"]
    })
}

pub fn provider(ctx: Arc<dyn types::ReadCtx>) -> Arc<dyn AbilityProvider> {
    let engine = Arc::new(workflow::Engine::new(ctx));
    let mut spec = AbilitySpec::must("workflow").protocols(&[
        ProtocolKind::Fun,
        ProtocolKind::Http,
        ProtocolKind::Mcp,
        ProtocolKind::Tools,
    ]);
    spec = spec.on(
        "catalog",
        "工作流、模型与新建模板",
        serde_json::json!({"type": "object"}),
        {
            let engine = Arc::clone(&engine);
            move |_ctx, args| {
                let engine = Arc::clone(&engine);
                ability_support::box_fut(logic::catalog::run(engine, args))
            }
        },
    );
    spec = spec.on("get", "读取一条工作流声明", slug(), {
        let engine = Arc::clone(&engine);
        move |_ctx, args| {
            let engine = Arc::clone(&engine);
            ability_support::box_fut(logic::get::run(engine, args))
        }
    });
    spec = spec.on(
        "save",
        "新建或改写工作流",
        serde_json::json!({
            "type": "object",
            "description": "WorkflowSpec 的 camelCase JSON，由工作流引擎校验"
        }),
        {
            let engine = Arc::clone(&engine);
            move |_ctx, args| {
                let engine = Arc::clone(&engine);
                ability_support::box_fut(logic::save::run(engine, args))
            }
        },
    );
    spec = spec.on("remove", "删除一条工作流", slug(), {
        let engine = Arc::clone(&engine);
        move |_ctx, args| {
            let engine = Arc::clone(&engine);
            ability_support::box_fut(logic::remove::run(engine, args))
        }
    });
    spec = spec.on(
        "toggle",
        "启用或停用一条工作流",
        serde_json::json!({
            "type": "object",
            "properties": {
                "slug": { "type": "string" },
                "enabled": { "type": "boolean" }
            },
            "required": ["slug", "enabled"]
        }),
        {
            let engine = Arc::clone(&engine);
            move |_ctx, args| {
                let engine = Arc::clone(&engine);
                ability_support::box_fut(logic::toggle::run(engine, args))
            }
        },
    );
    spec = spec.on(
        "save_model",
        "新建或改写模型",
        serde_json::json!({
            "type": "object",
            "description": "ModelSpec 的 camelCase JSON，由工作流引擎校验"
        }),
        {
            let engine = Arc::clone(&engine);
            move |_ctx, args| {
                let engine = Arc::clone(&engine);
                ability_support::box_fut(logic::save_model::run(engine, args))
            }
        },
    );
    spec = spec.on("remove_model", "删除一个模型声明", slug(), {
        let engine = Arc::clone(&engine);
        move |_ctx, args| {
            let engine = Arc::clone(&engine);
            ability_support::box_fut(logic::remove_model::run(engine, args))
        }
    });
    spec = spec.on(
        "run",
        "手动对一篇文章运行工作流",
        serde_json::json!({
            "type": "object",
            "properties": {
                "slug": { "type": "string" },
                "rid": { "type": "string" }
            },
            "required": ["slug", "rid"]
        }),
        {
            let engine = Arc::clone(&engine);
            move |_ctx, args| {
                let engine = Arc::clone(&engine);
                ability_support::box_fut(logic::run::run(engine, args))
            }
        },
    );
    spec = spec.on(
        "tick",
        "处理游标之后的新文章",
        serde_json::json!({"type": "object"}),
        {
            let engine = Arc::clone(&engine);
            move |_ctx, args| {
                let engine = Arc::clone(&engine);
                ability_support::box_fut(logic::tick::run(engine, args))
            }
        },
    );
    spec = spec.on(
        "runs",
        "最近的运行记录",
        serde_json::json!({
            "type": "object",
            "properties": { "limit": { "type": "integer" } }
        }),
        {
            let engine = Arc::clone(&engine);
            move |_ctx, args| {
                let engine = Arc::clone(&engine);
                ability_support::box_fut(logic::runs::run(engine, args))
            }
        },
    );
    spec = spec.on(
        "notify_test",
        "试投一条 webhook 或飞书通知",
        serde_json::json!({"type": "object"}),
        {
            let engine = Arc::clone(&engine);
            move |_ctx, args| {
                let engine = Arc::clone(&engine);
                ability_support::box_fut(logic::notify_test::run(engine, args))
            }
        },
    );
    spec.into_provider()
}

/// 声明写错是改 YAML 就能恢复的错误，其余按运行失败上报。
pub(crate) fn fail(error: workflow::Error) -> ability_support::Error {
    match error {
        workflow::Error::Invalid(message) => ability_support::Error::invalid_args(message),
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
