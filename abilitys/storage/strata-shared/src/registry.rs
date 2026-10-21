//! 同一数据目录只打开一次 `Strata`；仅 `committer` 工厂注册，其余 Ability 取用。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ability_support::{Error, Result};
use strata_client::Strata;

static REGISTRY: Mutex<Option<HashMap<PathBuf, Arc<Strata>>>> = Mutex::new(None);

fn map() -> std::sync::MutexGuard<'static, Option<HashMap<PathBuf, Arc<Strata>>>> {
    let mut guard = REGISTRY.lock().expect("strata registry");
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
    guard
}

pub async fn register_owner(data_dir: &Path, strata: Arc<Strata>) -> Result<()> {
    let key = data_dir.to_path_buf();
    let mut guard = map();
    let table = guard.as_mut().expect("strata registry");
    if table.contains_key(&key) {
        return Err(Error::failed("Strata 已注册"));
    }
    table.insert(key, strata);
    Ok(())
}

pub async fn borrow(data_dir: &Path) -> Result<Arc<Strata>> {
    let key = data_dir.to_path_buf();
    let guard = map();
    let table = guard.as_ref().expect("strata registry");
    table
        .get(&key)
        .cloned()
        .ok_or_else(|| Error::unavailable("Strata 未就绪"))
}
