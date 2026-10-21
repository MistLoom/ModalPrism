use std::sync::Arc;

use ability_support::{InstanceId, MethodId};
use frame_core::{Gateway, InvokeRequest};
use serde_json::Value;

pub async fn call(gateway: &Arc<dyn Gateway>, method: &str, args: Value) -> Result<Value, String> {
    let (instance, method) = method
        .split_once('.')
        .ok_or_else(|| "spec 无效".to_string())?;
    let instance = InstanceId::new(instance).map_err(|e| e.to_string())?;
    let method = MethodId::new(method).map_err(|e| e.to_string())?;
    let response = gateway
        .invoke(InvokeRequest::new(instance, method, args))
        .await
        .map_err(|e| e.to_string())?;
    Ok(response.data)
}
