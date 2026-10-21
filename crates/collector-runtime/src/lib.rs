//! 采集纯领域逻辑：计划解析、任务抓取编排、暂存与 push 解析。

pub mod error;
pub mod execute;
pub mod listen;
pub mod plans;
pub mod stage;

use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};

use config::Snapshot;
use serde_json::Value;

pub use error::{Error, invalid_input};
pub use listen::{PushCommit, serve_push};

pub struct Binds {
    pub http: SocketAddr,
    pub proxy: SocketAddr,
    pub push: SocketAddr,
}

pub fn require_loopback(addr: SocketAddr) -> Result<(), Error> {
    if addr.ip().is_loopback() {
        Ok(())
    } else {
        Err(Error::new("监听地址不是环回地址"))
    }
}

pub fn refuse_remote(snapshot: &Snapshot, data: &Path) -> Result<(), Error> {
    let system = &snapshot.system;
    if system
        .pointer("/strata/lance/uri")
        .and_then(Value::as_str)
        .is_some_and(|uri| !uri.is_empty())
    {
        return Err(Error::new("存储位置未规定"));
    }
    if system
        .pointer("/strata/control_store/backend")
        .and_then(Value::as_str)
        == Some("postgres")
    {
        return Err(Error::new("存储位置未规定"));
    }
    if let Some(path) = system
        .pointer("/strata/control_store/path")
        .and_then(Value::as_str)
    {
        if normalize(Path::new(path)) != normalize(&data.join("control.db")) {
            return Err(Error::new("存储位置未规定"));
        }
    }
    if let Some(path) = system.pointer("/strata/staging/dir").and_then(Value::as_str) {
        if normalize(Path::new(path)) != normalize(&data.join("staging")) {
            return Err(Error::new("存储位置未规定"));
        }
    }
    Ok(())
}

pub fn normalize(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}
