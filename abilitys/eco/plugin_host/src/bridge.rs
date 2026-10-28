//! Node 插件宿主 NDJSON 桥。

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ability_support::{AbilityCtx, Error, Result};

use crate::claude_mod_shape::claude_mod_load_strict;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, oneshot};

static HOOK_ID: AtomicU64 = AtomicU64::new(1);

pub struct PluginHostBridge {
    stdin: Mutex<tokio::process::ChildStdin>,
    pending_hooks: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    pending_tools: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    ctx: Arc<dyn AbilityCtx>,
    tool_frames: std::sync::Mutex<Vec<Value>>,
}

impl PluginHostBridge {
    /// 加载前探测（独立短生命周期 Node 进程）。
    pub async fn probe(
        script: PathBuf,
        plugins: Vec<Value>,
        ctx: Arc<dyn AbilityCtx>,
    ) -> Result<Value> {
        let script_dir = script
            .parent()
            .ok_or_else(|| Error::config("插件宿主脚本路径无效"))?;
        let node = std::env::var("LEAF_PLUGIN_NODE").unwrap_or_else(|_| "node".into());
        let mut child = Command::new(node)
            .arg(&script)
            .current_dir(script_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| Error::failed(format!("启动探测宿主失败: {e}")))?;

        let stdin = child.stdin.take().ok_or_else(|| Error::failed("stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| Error::failed("stdout"))?;

        let bridge = Arc::new(Self {
            stdin: Mutex::new(stdin),
            pending_hooks: Mutex::new(HashMap::new()),
            pending_tools: Mutex::new(HashMap::new()),
            ctx,
            tool_frames: std::sync::Mutex::new(Vec::new()),
        });

        let (probe_tx, probe_rx) = oneshot::channel();
        let io = Arc::clone(&bridge);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            let mut probe_tx = Some(probe_tx);
            while let Ok(Some(line)) = lines.next_line().await {
                let frame: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let ty = frame.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if ty == "probe_result" {
                    if let Some(tx) = probe_tx.take() {
                        let _ = tx.send(frame);
                    }
                    break;
                }
                if ty == "host_call" {
                    let id = frame
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let ns = frame
                        .get("ns")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let method = frame
                        .get("method")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let args = frame.get("args").cloned().unwrap_or(json!([]));
                    let reply_bridge = Arc::clone(&io);
                    let ctx = Arc::clone(&io.ctx);
                    tokio::spawn(async move {
                        let result =
                            crate::host_api::dispatch_host_api(&*ctx, &ns, &method, &args).await;
                        let reply = match result {
                            Ok(v) => {
                                json!({ "type": "host_result", "id": id, "ok": true, "value": v })
                            }
                            Err(e) => json!({
                                "type": "host_result",
                                "id": id,
                                "ok": false,
                                "error": e.to_string()
                            }),
                        };
                        let _ = reply_bridge.send(&reply).await;
                    });
                }
            }
            let _ = child.wait().await;
        });

        let id = "probe_1";
        bridge
            .send(&json!({ "type": "probe", "id": id, "plugins": plugins }))
            .await?;
        let frame = probe_rx
            .await
            .map_err(|_| Error::failed("插件探测无响应"))?;
        Ok(frame)
    }

    pub async fn spawn(
        script: PathBuf,
        plugins: Vec<Value>,
        ctx: Arc<dyn AbilityCtx>,
    ) -> Result<(Arc<Self>, Value)> {
        let script_dir = script
            .parent()
            .ok_or_else(|| Error::config("插件宿主脚本路径无效"))?;
        let node = std::env::var("LEAF_PLUGIN_NODE").unwrap_or_else(|_| "node".into());
        let mut child = Command::new(node)
            .arg(&script)
            .current_dir(script_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| Error::failed(format!("启动插件宿主失败: {e}")))?;

        let stdin = child.stdin.take().ok_or_else(|| Error::failed("stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| Error::failed("stdout"))?;

        let bridge = Arc::new(Self {
            stdin: Mutex::new(stdin),
            pending_hooks: Mutex::new(HashMap::new()),
            pending_tools: Mutex::new(HashMap::new()),
            ctx,
            tool_frames: std::sync::Mutex::new(Vec::new()),
        });

        let (ready_tx, ready_rx) = oneshot::channel();
        let io = Arc::clone(&bridge);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            let mut ready_tx = Some(ready_tx);
            while let Ok(Some(line)) = lines.next_line().await {
                let frame: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let ty = frame.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match ty {
                    "ready" => {
                        if let Some(tx) = ready_tx.take() {
                            let _ = tx.send(frame);
                        }
                    }
                    "hook_result" => {
                        let id = frame.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        if let Some(tx) = io.pending_hooks.lock().await.remove(id) {
                            let value = hook_value(&frame);
                            let _ = tx.send(value);
                        }
                    }
                    "host_call" => {
                        let id = frame
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let ns = frame
                            .get("ns")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let method = frame
                            .get("method")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let args = frame.get("args").cloned().unwrap_or(json!([]));
                        let reply_bridge = Arc::clone(&io);
                        let ctx = Arc::clone(&io.ctx);
                        tokio::spawn(async move {
                            let result =
                                crate::host_api::dispatch_host_api(&*ctx, &ns, &method, &args)
                                    .await;
                            let reply = match result {
                                Ok(v) => {
                                    json!({ "type": "host_result", "id": id, "ok": true, "value": v })
                                }
                                Err(e) => json!({
                                    "type": "host_result",
                                    "id": id,
                                    "ok": false,
                                    "error": e.to_string()
                                }),
                            };
                            let _ = reply_bridge.send(&reply).await;
                        });
                    }
                    "tool_register" => {
                        io.tool_frames.lock().expect("tool_frames").push(frame);
                    }
                    "tool_result" => {
                        let id = frame.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        if let Some(tx) = io.pending_tools.lock().await.remove(id) {
                            let _ = tx.send(hook_value(&frame));
                        }
                    }
                    "ui_emit" => {
                        let payload = frame.get("payload").cloned().unwrap_or(json!({}));
                        crate::user_fanout::emit_user("eco.ui", payload);
                    }
                    _ => {}
                }
            }
            if let Some(tx) = ready_tx.take() {
                let _ = tx.send(json!({ "type": "error", "message": "host_exit" }));
            }
            let _ = child.wait().await;
        });

        bridge
            .send(&json!({ "type": "init", "plugins": plugins }))
            .await?;

        let ready = ready_rx
            .await
            .map_err(|_| Error::failed("插件宿主未就绪"))?;
        if ready.get("type").and_then(|v| v.as_str()) == Some("error") {
            return Err(Error::failed(
                ready
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("插件宿主启动失败")
                    .to_string(),
            ));
        }
        if let Some(msg) = plugin_load_errors_message(&ready) {
            if claude_mod_load_strict() {
                return Err(Error::failed(msg));
            }
            tracing::warn!("{msg}");
        }
        Ok((bridge, ready))
    }

    pub async fn send(&self, frame: &Value) -> Result<()> {
        let mut w = self.stdin.lock().await;
        let line = serde_json::to_string(frame).map_err(|e| Error::failed(e.to_string()))?;
        w.write_all(line.as_bytes())
            .await
            .map_err(|e| Error::failed(e.to_string()))?;
        w.write_all(b"\n")
            .await
            .map_err(|e| Error::failed(e.to_string()))?;
        w.flush().await.map_err(|e| Error::failed(e.to_string()))
    }

    pub async fn call_hook(&self, name: &str, payload: Value) -> Result<Value> {
        let id = HOOK_ID.fetch_add(1, Ordering::Relaxed).to_string();
        let (tx, rx) = oneshot::channel();
        self.pending_hooks.lock().await.insert(id.clone(), tx);
        self.send(&json!({
            "type": "hook",
            "id": id,
            "name": name,
            "payload": payload
        }))
        .await?;
        rx.await.map_err(|_| Error::failed("插件宿主钩子无响应"))
    }

    pub async fn execute_tool(&self, name: &str, args: Value) -> Result<Value> {
        let id = HOOK_ID.fetch_add(1, Ordering::Relaxed).to_string();
        let (tx, rx) = oneshot::channel();
        self.pending_tools.lock().await.insert(id.clone(), tx);
        self.send(&json!({
            "type": "tool_execute",
            "id": id,
            "name": name,
            "args": args,
        }))
        .await?;
        rx.await.map_err(|_| Error::failed("插件工具执行无响应"))
    }

    pub fn registered_tool_defs(&self) -> Vec<Value> {
        self.tool_frames
            .lock()
            .expect("tool_frames")
            .iter()
            .filter_map(|frame| {
                let tool_name = frame.get("name").and_then(|v| v.as_str())?;
                let origin = frame
                    .get("origin")
                    .and_then(|v| v.as_str())
                    .unwrap_or("dsh");
                let instance_id = frame
                    .get("instanceId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default");
                let llm_name = crate::naming::llm_tool_name(origin, instance_id, tool_name);
                Some(json!({
                    "type": "function",
                    "function": {
                        "name": llm_name,
                        "description": frame.get("description").cloned().unwrap_or(json!("")),
                        "parameters": frame.get("parameters").cloned().unwrap_or(json!({})),
                    }
                }))
            })
            .collect()
    }
}

fn plugin_load_errors_message(ready: &Value) -> Option<String> {
    let errors = ready.get("load_errors").and_then(|v| v.as_array())?;
    if errors.is_empty() {
        return None;
    }
    let lines: Vec<String> = errors
        .iter()
        .filter_map(|e| {
            let path = e.get("path").and_then(|v| v.as_str()).unwrap_or("?");
            let err = e.get("error").and_then(|v| v.as_str()).unwrap_or("?");
            Some(format!("{path}: {err}"))
        })
        .collect();
    Some(format!("插件加载失败: {}", lines.join("; ")))
}

fn hook_value(frame: &Value) -> Value {
    if frame.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        frame.get("value").cloned().unwrap_or(Value::Null)
    } else {
        json!({ "error": frame.get("error") })
    }
}
