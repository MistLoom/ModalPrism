//! Ability 方法实现入口。各 Ability 调用 `invoke`，不经过 `HostResources::call` 总线。

use serde_json::Value;

use crate::Error;
use crate::HostResources;

pub async fn invoke(
    host: &HostResources,
    ability: &str,
    method: &str,
    args: Value,
) -> Result<Value, Error> {
    let key = format!("{ability}.{method}");
    dispatch(host, &key, args).await
}

pub fn map_ability(
    result: Result<Value, Error>,
) -> ability_support::Result<Value> {
    result.map_err(|error| {
        if crate::invalid_input(&error.message) {
            ability_support::Error::invalid_args(error.message)
        } else {
            ability_support::Error::failed(error.to_string())
        }
    })
}

async fn dispatch(host: &HostResources, method: &str, args: Value) -> Result<Value, Error> {
    crate::dispatch(host, method, args).await
}
