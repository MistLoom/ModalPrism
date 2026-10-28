//! 查询 CLI：在途链路 / 最近事件 / 日志尾读。
//! 入口应用可在自己的命令处理里调用；离线日志查询见 bin `monitor log`。

use crate::MonitorApp;
use ability_support::Result;
use serde_json;

/// 监测查询子命令。
#[derive(Clone, Debug)]
pub enum MonitorCmd {
    /// 在途调用链。
    Chains,
    /// 最近事件。
    Events {
        tail: usize,
    },
    /// JSONL 日志尾读。
    Log {
        tail: usize,
    },
    Stats,
    Graph,
}

/// 执行查询并输出到 stdout。返回打印的行数。
pub fn run_cli(app: &MonitorApp, cmd: MonitorCmd) -> Result<usize> {
    match cmd {
        MonitorCmd::Chains => {
            let chains = app.chains();
            println!("{:=<58}", " 在途调用链 ");
            if chains.is_empty() {
                println!("(无在途链路)");
            }
            for c in &chains {
                println!(
                    "{:<38} {} {} span={} running={} started_at={} span_id={} parent={} ability={}",
                    c.chain_id,
                    c.instance,
                    c.method.as_deref().unwrap_or("-"),
                    c.span,
                    c.running,
                    c.started_at
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".into()),
                    c.span_id.as_deref().unwrap_or("-"),
                    c.parent_span_id.as_deref().unwrap_or("-"),
                    c.ability_id.as_deref().unwrap_or("-"),
                );
            }
            println!("{:=<58}", "");
            Ok(chains.len())
        }
        MonitorCmd::Events { tail } => {
            let evs = app.recent_events();
            let start = evs.len().saturating_sub(tail);
            let mut count = 0;
            println!("{:=<58}", " 最近监测事件 ");
            for ev in &evs[start..] {
                println!(
                    "{} [{}] {} {} {}",
                    ev.at,
                    ev.kind,
                    ev.phase.as_deref().unwrap_or("-"),
                    ev.instance.as_deref().unwrap_or("-"),
                    ev.method.as_deref().unwrap_or("-"),
                );
                count += 1;
            }
            println!("{:=<58}", "");
            Ok(count)
        }
        MonitorCmd::Log { tail } => {
            let lines = app.tail_log(tail)?;
            for l in &lines {
                println!("{l}");
            }
            Ok(lines.len())
        }
        MonitorCmd::Stats => {
            let s = app.stats();
            println!(
                "dropped={} queued={} recent={}",
                s.dropped, s.queued, s.recent
            );
            Ok(1)
        }
        MonitorCmd::Graph => {
            let g = app.chain_graph();
            println!("{}", serde_json::to_string_pretty(&g).unwrap_or_default());
            Ok(g.nodes.len())
        }
    }
}
