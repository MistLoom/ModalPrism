//! Leaf 可选底层监测。
//!
//! 提供全局配置、异步记录、查询 CLI、实时 TUI、可选 HTTP 与进程内查询。
//! 依赖本包并配置即可使用；去掉本包及入口依赖后不再提供监测查询入口。
//!
//! 本包依赖 `ability-core` 并启用其 `monitor` feature，
//! 从而把探针编进二进制；该 feature 是实现细节，不必单独当作产品开关。

pub mod cli;
pub mod graph;
pub mod http;
pub mod tui;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ability_core::AbilityHost;
use ability_support::monitor::{MonitorEvent, MonitorSink};
use ability_support::{Error, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

/// 全局配置。默认读 `monitor.yml`，缺省值见 [`MonitorConfig::default`]。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MonitorConfig {
    /// 异步 JSONL 日志路径。空字符串 = 不落盘。
    pub log_path: String,
    /// 事件队列容量；满则丢最旧，绝不阻塞调用方。
    pub queue_capacity: usize,
    /// 内存中保留的最近事件数（供查询 / TUI）。
    pub recent_capacity: usize,
    /// 可选 HTTP 面（默认关）。
    pub http: HttpConfig,
    /// TUI 刷新间隔（毫秒）。
    pub tui_refresh_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HttpConfig {
    /// 默认关闭；开启时本机绑定，与 mgmt 端口分离。
    pub enabled: bool,
    pub bind: String,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            log_path: "monitor.jsonl".into(),
            queue_capacity: 4096,
            recent_capacity: 1024,
            http: HttpConfig {
                enabled: false,
                bind: "127.0.0.1:8792".into(),
            },
            tui_refresh_ms: 500,
        }
    }
}

impl MonitorConfig {
    /// 读盘加载；文件不存在用默认配置。
    pub fn load(path: Option<&std::path::Path>) -> Self {
        let path = path
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("monitor.yml"));
        match std::fs::read_to_string(&path) {
            Ok(raw) => serde_yaml::from_str(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }
}

/// 有界队列（满则丢最旧）+ 异步落盘 worker 的 recorder。
struct QueueSink {
    queue: Mutex<Queue>,
    notify: Notify,
    dropped: AtomicU64,
}

struct Queue {
    pending: VecDeque<MonitorEvent>,
    recent: VecDeque<MonitorEvent>,
    queue_capacity: usize,
    recent_capacity: usize,
}

impl MonitorSink for QueueSink {
    fn record(&self, event: MonitorEvent) {
        let mut q = self.queue.lock().expect("monitor queue");
        if q.pending.len() >= q.queue_capacity {
            q.pending.pop_front();
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        q.pending.push_back(event.clone());
        if q.recent.len() >= q.recent_capacity {
            q.recent.pop_front();
        }
        q.recent.push_back(event);
        drop(q);
        self.notify.notify_one();
    }
}

/// 监测 app 运行句柄。持有着宿主查询面、recorder 与落盘 worker。
pub struct MonitorApp {
    host: AbilityHost,
    queue: Arc<QueueSink>,
    stop: Arc<AtomicBool>,
    pub config: MonitorConfig,
}

/// Sink / buffer statistics for drop-aware querying.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MonitorStats {
    /// Events dropped from the bounded pending queue (oldest discarded).
    pub dropped: u64,
    /// Current pending queue length (awaiting JSONL flush).
    pub queued: usize,
    /// Current recent ring length.
    pub recent: usize,
}

impl MonitorApp {
    /// 在途调用链快照（根链记录，嵌套同链升 span）。
    pub fn chains(&self) -> Vec<ability_support::ChainStatus> {
        self.host.chains_snapshot()
    }

    /// 最近事件（内存环形缓冲，最新在后）。
    pub fn recent_events(&self) -> Vec<MonitorEvent> {
        self.queue
            .queue
            .lock()
            .expect("monitor queue")
            .recent
            .iter()
            .cloned()
            .collect()
    }

    /// Drop / queue / recent stats (drop-aware).
    pub fn stats(&self) -> MonitorStats {
        let q = self.queue.queue.lock().expect("monitor queue");
        MonitorStats {
            dropped: self.queue.dropped.load(Ordering::Relaxed),
            queued: q.pending.len(),
            recent: q.recent.len(),
        }
    }

    /// Rebuild a call-chain graph from recent events (explicit parent_span_id only).
    pub fn chain_graph(&self) -> crate::graph::ChainGraph {
        crate::graph::build_chain_graph(&self.recent_events())
    }

    /// 尾读 JSONL 日志文件（离线查询也用它）。
    pub fn tail_log(&self, n: usize) -> Result<Vec<String>> {
        tail_log_file(&self.config.log_path, n)
    }

    /// 关闭监测：卸下全局回调并停掉落盘 worker。
    pub fn shutdown(&self) {
        ability_support::monitor::clear_monitor_sink();
        self.stop.store(true, Ordering::SeqCst);
        self.queue.notify.notify_one();
    }
}

/// 接入监测：登记全局回调、启动异步落盘 worker。
/// 入口应用（组合根）在拿到 `AbilityHost` 后调用一次。
pub fn attach(host: &AbilityHost, config: MonitorConfig) -> Arc<MonitorApp> {
    let queue = Arc::new(QueueSink {
        queue: Mutex::new(Queue {
            pending: VecDeque::new(),
            recent: VecDeque::new(),
            queue_capacity: config.queue_capacity.max(16),
            recent_capacity: config.recent_capacity.max(16),
        }),
        notify: Notify::new(),
        dropped: AtomicU64::new(0),
    });
    ability_support::monitor::set_monitor_sink(Arc::clone(&queue) as Arc<dyn MonitorSink>);

    let stop = Arc::new(AtomicBool::new(false));
    if !config.log_path.is_empty() {
        let worker_queue = Arc::clone(&queue);
        let worker_stop = Arc::clone(&stop);
        let log_path = PathBuf::from(&config.log_path);
        tokio::spawn(async move {
            worker(worker_queue, worker_stop, log_path).await;
        });
    }

    Arc::new(MonitorApp {
        host: host.clone(),
        queue,
        stop,
        config,
    })
}

async fn worker(queue: Arc<QueueSink>, stop: Arc<AtomicBool>, log_path: PathBuf) {
    use tokio::io::AsyncWriteExt;
    let mut file: Option<tokio::fs::File> = None;
    loop {
        let batch = {
            let mut q = queue.queue.lock().expect("monitor queue");
            std::mem::take(&mut q.pending)
        };
        if !batch.is_empty() {
            if file.is_none() {
                file = tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_path)
                    .await
                    .ok();
            }
            if let Some(f) = file.as_mut() {
                let mut buf = String::new();
                for ev in &batch {
                    if let Ok(line) = serde_json::to_string(ev) {
                        buf.push_str(&line);
                        buf.push('\n');
                    }
                }
                let _ = f.write_all(buf.as_bytes()).await;
                let _ = f.flush().await;
            }
        }
        if stop.load(Ordering::SeqCst) {
            return;
        }
        if batch.is_empty() {
            // 无事件时挂起等待；带超时兜底，避免错过极端竞争。
            let notified = queue.notify.notified();
            tokio::select! {
                _ = notified => {}
                _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {}
            }
        }
    }
}

/// 尾读日志文件最后 n 行（离线 `monitor log` 用）。
pub fn tail_log_file(path: &str, n: usize) -> Result<Vec<String>> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| Error::io(format!("读取监测日志失败 {path}: {e}")))?;
    let lines: Vec<&str> = raw.lines().collect();
    let start = lines.len().saturating_sub(n);
    Ok(lines[start..].iter().map(|s| s.to_string()).collect())
}

/// 开放 fun：程序内调用的监测查询入口（供 FunPort / 其它能力复用）。
/// `target` 支持 `monitor.chains` / `monitor.events` / `monitor.log`。
pub async fn fun_call(
    app: &MonitorApp,
    target: &str,
    args: serde_json::Value,
) -> Result<serde_json::Value> {
    match target {
        "monitor.chains" => Ok(serde_json::json!({ "chains": app.chains() })),
        "monitor.events" => {
            let n = args.get("tail").and_then(|v| v.as_u64()).unwrap_or(64) as usize;
            let evs = app.recent_events();
            let start = evs.len().saturating_sub(n);
            Ok(serde_json::json!({ "events": &evs[start..] }))
        }
        "monitor.log" => {
            let n = args.get("tail").and_then(|v| v.as_u64()).unwrap_or(64) as usize;
            let lines = app.tail_log(n)?;
            Ok(serde_json::json!({ "lines": lines }))
        }
        "monitor.stats" => Ok(serde_json::json!(app.stats())),
        "monitor.graph" => Ok(serde_json::json!(app.chain_graph())),
        other => Err(Error::not_found(format!(
            "unknown monitor fun target: {other}"
        ))),
    }
}
