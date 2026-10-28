//! 业务逻辑：run

use ability_support::{AbilityCtx, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const STATE_KEY: &str = "todos";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TodoItem {
    id: String,
    content: String,
    status: String,
}

#[derive(Deserialize)]
struct Args {
    todos: Vec<TodoItem>,
    #[serde(default)]
    merge: bool,
}

pub async fn run(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let a: Args = serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
    for t in &a.todos {
        if t.id.is_empty() || t.content.is_empty() {
            return Err(Error::invalid_args("todo id/content 不能为空"));
        }
        match t.status.as_str() {
            "pending" | "in_progress" | "completed" | "cancelled" => {}
            other => return Err(Error::invalid_args(format!("非法 status: {other}"))),
        }
    }
    let next = if a.merge {
        let mut ordered: Vec<TodoItem> = match ctx.local_get(STATE_KEY).await? {
            Some(v) => serde_json::from_value(v)
                .map_err(|e| Error::failed(format!("todo 会话状态损 {e}")))?,
            None => Vec::new(),
        };
        for t in a.todos {
            if let Some(pos) = ordered.iter().position(|x| x.id == t.id) {
                ordered[pos] = t;
            } else {
                ordered.push(t);
            }
        }
        ordered
    } else {
        a.todos
    };
    ctx.local_set(
        STATE_KEY,
        serde_json::to_value(&next).map_err(|e| Error::failed(e.to_string()))?,
    )
    .await?;
    Ok(json!({ "todos": next }))
}
