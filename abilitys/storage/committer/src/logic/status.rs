use std::path::Path;

use ability_support::Result;
use serde_json::{Value, json};

pub fn status_paths(data_dir: &Path) -> Value {
    json!({
        "control": data_dir.join("control.db"),
        "lance": data_dir.join("lance"),
        "staging": data_dir.join("staging"),
    })
}

pub async fn run(data_dir: &Path, _args: &Value) -> Result<Value> {
    Ok(status_paths(data_dir))
}
