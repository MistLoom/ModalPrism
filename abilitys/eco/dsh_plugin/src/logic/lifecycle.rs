use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use ability_plugin_host::PluginHostRuntime;
use ability_support::{Error, InstanceId, Result};
use tokio::sync::Mutex;

static RUNTIMES: StdMutex<Option<HashMap<String, Arc<Mutex<PluginHostRuntime>>>>> =
    StdMutex::new(None);

pub fn store_runtime(instance: &InstanceId, rt: Arc<Mutex<PluginHostRuntime>>) -> Result<()> {
    let mut g = RUNTIMES.lock().expect("runtimes");
    if g.is_none() {
        *g = Some(HashMap::new());
    }
    g.as_mut().expect("map").insert(instance.to_string(), rt);
    Ok(())
}

pub fn runtime_handle(instance: &InstanceId) -> Result<Arc<Mutex<PluginHostRuntime>>> {
    let g = RUNTIMES.lock().expect("runtimes");
    let map = g
        .as_ref()
        .ok_or_else(|| Error::failed("runtime 未初始化"))?;
    map.get(instance.as_str())
        .cloned()
        .ok_or_else(|| Error::failed("DSH runtime 不存在"))
}
