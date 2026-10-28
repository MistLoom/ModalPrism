//! 可选用户事件扇出（由 web-agent 等在启动时注册）。

use std::sync::{Arc, OnceLock};

use serde_json::Value;

type FanoutFn = dyn Fn(&str, Value) + Send + Sync;

static FANOUT: OnceLock<Arc<FanoutFn>> = OnceLock::new();

pub fn set_user_fanout(f: Arc<FanoutFn>) {
    let _ = FANOUT.set(f);
}

pub fn emit_user(name: &str, payload: Value) {
    if let Some(f) = FANOUT.get() {
        f(name, payload);
    }
}
