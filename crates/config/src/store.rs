//! 快照代际、重启项与文件监听。

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use notify::event::EventKind;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::{Map, Value};

use crate::error::{Diagnostics, Error};
use crate::load::{LoadOptions, validate};
use crate::snapshot::Snapshot;

pub const DEFAULT_DEBOUNCE: Duration = Duration::from_secs(2);

const SYSTEM_RESTART: &[&str] = &[
    "strata.lance.uri",
    "strata.lance.storage_options",
    "strata.lance.new_table_data_storage_version",
    "strata.lance.new_table_enable_v2_manifest_paths",
    "strata.control_store",
    "strata.staging.dir",
];

#[derive(Clone, Debug, PartialEq)]
pub struct RestartChange {
    pub path: String,
    pub applied: Option<Value>,
    pub on_disk: Option<Value>,
}

struct Inner {
    snapshot: Arc<Snapshot>,
    applied: Arc<Snapshot>,
    held: Arc<Snapshot>,
    pending: Vec<RestartChange>,
    last_error: Option<Diagnostics>,
}

pub struct ConfigStore {
    dir: PathBuf,
    options: LoadOptions,
    inner: RwLock<Inner>,
}

impl ConfigStore {
    pub fn open(dir: impl Into<PathBuf>, options: LoadOptions) -> Result<Self, Error> {
        let dir = dir.into();
        let mut snapshot = validate(&dir, &options)?;
        snapshot.generation = 1;
        let snapshot = Arc::new(snapshot);
        Ok(Self {
            dir,
            options,
            inner: RwLock::new(Inner {
                applied: Arc::clone(&snapshot),
                held: Arc::clone(&snapshot),
                snapshot,
                pending: Vec::new(),
                last_error: None,
            }),
        })
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        Arc::clone(&self.inner.read().expect("配置锁").snapshot)
    }

    pub fn applied(&self) -> Arc<Snapshot> {
        Arc::clone(&self.inner.read().expect("配置锁").applied)
    }

    pub fn pending_restart(&self) -> Vec<RestartChange> {
        self.inner.read().expect("配置锁").pending.clone()
    }

    pub fn last_error(&self) -> Option<Diagnostics> {
        self.inner.read().expect("配置锁").last_error.clone()
    }

    pub fn reload(&self) -> Result<(), Error> {
        match validate(&self.dir, &self.options) {
            Ok(mut snapshot) => {
                let mut inner = self.inner.write().expect("配置锁");
                snapshot.generation = inner.snapshot.generation.saturating_add(1);
                let (applied, pending) = project(&snapshot, &inner.held);
                inner.snapshot = Arc::new(snapshot);
                inner.applied = Arc::new(applied);
                inner.pending = pending;
                inner.last_error = None;
                Ok(())
            }
            Err(error) => {
                if let Error::Invalid(diagnostics) = &error {
                    self.inner.write().expect("配置锁").last_error = Some(diagnostics.clone());
                }
                Err(error)
            }
        }
    }
}

pub struct WatchHandle {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn watch(store: Arc<ConfigStore>, debounce: Duration) -> Result<WatchHandle, Error> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<Event, notify::Error>>();
    let mut watcher =
        RecommendedWatcher::new(tx, notify::Config::default()).map_err(|source| Error::Watch {
            path: store.dir.display().to_string(),
            source,
        })?;
    watcher
        .watch(&store.dir, RecursiveMode::Recursive)
        .map_err(|source| Error::Watch {
            path: store.dir.display().to_string(),
            source,
        })?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let thread = std::thread::spawn(move || {
        let _watcher = watcher;
        let mut dirty = false;
        let mut last = Instant::now();
        loop {
            if flag.load(Ordering::Relaxed) {
                break;
            }
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(event)) => {
                    if is_data_change(&event) {
                        dirty = true;
                        last = Instant::now();
                    }
                }
                Ok(Err(_)) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if dirty && last.elapsed() >= debounce {
                        let mut postponed = false;
                        while let Ok(Ok(event)) = rx.try_recv() {
                            if is_data_change(&event) {
                                postponed = true;
                            }
                        }
                        if postponed {
                            last = Instant::now();
                        } else {
                            dirty = false;
                            let _ = store.reload();
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    Ok(WatchHandle {
        stop,
        thread: Some(thread),
    })
}

pub fn watch_hangup(store: Arc<ConfigStore>) -> std::io::Result<std::thread::JoinHandle<()>> {
    let mut signals = signal_hook::iterator::Signals::new([signal_hook::consts::SIGHUP])?;
    Ok(std::thread::spawn(move || {
        for _ in signals.forever() {
            let _ = store.reload();
        }
    }))
}

fn is_data_change(event: &Event) -> bool {
    matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

fn project(latest: &Snapshot, held: &Snapshot) -> (Snapshot, Vec<RestartChange>) {
    let mut applied = latest.clone();
    let mut pending = Vec::new();
    for path in SYSTEM_RESTART {
        let on_disk = lookup(&latest.system, path).cloned();
        let held_value = lookup(&held.system, path).cloned();
        if on_disk != held_value {
            pending.push(RestartChange {
                path: format!("system.{path}"),
                applied: held_value.clone(),
                on_disk,
            });
        }
        match &held_value {
            Some(value) => set_path(&mut applied.system, path, value.clone()),
            None => remove_path(&mut applied.system, path),
        }
        if let Some(origin) = held.origins.get(&("system".into(), (*path).to_string())) {
            applied
                .origins
                .insert(("system".into(), (*path).to_string()), origin.clone());
        }
    }
    let held_accessors = accessor_ids(held);
    let disk_accessors = accessor_ids(latest);
    applied.modules.retain(|id, module| {
        module.get("kind").and_then(Value::as_str) != Some("accessor")
            || held_accessors.contains(id)
    });
    for id in &held_accessors {
        let Some(old) = held.modules.get(id) else {
            continue;
        };
        if !disk_accessors.contains(id) {
            applied.modules.insert(id.clone(), old.clone());
            pending.push(RestartChange {
                path: id.clone(),
                applied: Some(old.clone()),
                on_disk: None,
            });
            continue;
        }
        let Some(module) = applied.modules.get_mut(id) else {
            continue;
        };
        let old_impl = old.get("implementation").cloned();
        let new_impl = module.get("implementation").cloned();
        if old_impl != new_impl {
            pending.push(RestartChange {
                path: format!("{id}.implementation"),
                applied: old_impl.clone(),
                on_disk: new_impl,
            });
        }
        if let Some(value) = &old_impl {
            module
                .as_object_mut()
                .expect("模块是对象")
                .insert("implementation".into(), value.clone());
        }
        let old_version = lookup(old, "spec.moli.version").cloned();
        let new_version = lookup(module, "spec.moli.version").cloned();
        if old_version != new_version {
            pending.push(RestartChange {
                path: format!("{id}.spec.moli.version"),
                applied: old_version.clone(),
                on_disk: new_version,
            });
        }
        match &old_version {
            Some(value) => set_path(module, "spec.moli.version", value.clone()),
            None => remove_path(module, "spec.moli.version"),
        }
    }
    for id in &disk_accessors {
        if !held_accessors.contains(id) {
            pending.push(RestartChange {
                path: id.clone(),
                applied: None,
                on_disk: latest.modules.get(id).cloned(),
            });
        }
    }
    applied.generation = latest.generation;
    (applied, pending)
}

fn accessor_ids(snapshot: &Snapshot) -> BTreeSet<String> {
    snapshot
        .modules
        .iter()
        .filter(|(_, module)| module.get("kind").and_then(Value::as_str) == Some("accessor"))
        .map(|(id, _)| id.clone())
        .collect()
}

fn lookup<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

fn set_path(value: &mut Value, path: &str, next: Value) {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = value;
    for (index, part) in parts.iter().enumerate() {
        if index + 1 == parts.len() {
            if let Some(object) = current.as_object_mut() {
                object.insert((*part).to_string(), next);
            }
            return;
        }
        let object = current.as_object_mut().expect("路径中间是对象");
        let entry = object
            .entry((*part).to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        current = entry;
    }
}

fn remove_path(value: &mut Value, path: &str) {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = value;
    for (index, part) in parts.iter().enumerate() {
        let Some(object) = current.as_object_mut() else {
            return;
        };
        if index + 1 == parts.len() {
            object.remove(*part);
            return;
        }
        let Some(child) = object.get_mut(*part) else {
            return;
        };
        current = child;
    }
}
