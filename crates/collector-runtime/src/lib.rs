//! 采集执行引擎。不含 LEAF Gateway 与 Ability 装配。

pub mod error;
pub mod execute;
pub mod listen;
pub mod plans;
pub mod queue;
pub mod stage;
pub mod transport;

use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};

use config::{LoadOptions, Snapshot};
use serde_json::Value;
use strata_client::Options;

pub use error::{Error, invalid_input};
pub use listen::{PushCommit, serve_proxy, serve_push};
pub use plans::{capacities, flush_interval, snapshot_interval};
pub use transport::{EgressGw, PaidGw, policy_of};

/// 三个环回监听地址。
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

pub fn strata_options(system: &Value) -> Options {
    let mut options = Options::default();
    if let Some(size) = system
        .pointer("/strata/blob/inline_size_threshold")
        .and_then(Value::as_str)
        .and_then(parse_size)
    {
        options.blob_inline_threshold = size;
    }
    if let Some(size) = system
        .pointer("/strata/blob/dedicated_size_threshold")
        .and_then(Value::as_str)
        .and_then(parse_size)
    {
        options.blob_dedicated_threshold = size;
    }
    if let Some(ttl) = system
        .pointer("/strata/staging/ttl")
        .and_then(Value::as_str)
        .and_then(parse_duration)
    {
        options.staging_ttl = ttl;
    }
    options
}

fn parse_size(text: &str) -> Option<usize> {
    let split = text.find(|ch: char| !ch.is_ascii_digit())?;
    let count: u64 = text[..split].parse().ok()?;
    let scale = match &text[split..] {
        "B" => 1,
        "KiB" => 1024,
        "MiB" => 1024 * 1024,
        "GiB" => 1024 * 1024 * 1024,
        _ => return None,
    };
    usize::try_from(count * scale).ok()
}

fn parse_duration(text: &str) -> Option<std::time::Duration> {
    let split = text.find(|ch: char| !ch.is_ascii_digit())?;
    let count: u64 = text[..split].parse().ok()?;
    let unit = &text[split..];
    let secs = match unit {
        "s" => count,
        "m" => count * 60,
        "h" => count * 3600,
        _ => return None,
    };
    Some(std::time::Duration::from_secs(secs))
}
