# leaf-monitor（`apps/monitor`）

Leaf 可选监测：配置、异步 JSONL、查询 CLI、**实时 TUI**、可选 HTTP、进程内查询。

需要时依赖本包并配置即可；不需要则不引入本包及入口依赖。

## 实时 TUI

```bash
# 独立打开（默认 DEMO：事件泵 + 慢 echo 调用，标题栏标 DEMO）
cargo run -p leaf-monitor -- tui

# 更快刷新
cargo run -p leaf-monitor -- tui --refresh-ms 200 --pump-ms 300

# 关闭 demo 泵（独立进程下几乎无事件）
cargo run -p leaf-monitor -- tui --no-demo

# 冒烟例
cargo run -p leaf-monitor --example tui_smoke
```

嵌入宿主（真实 LIVE，无 DEMO 标）：

```rust
use leaf_monitor::{attach, MonitorConfig};
use leaf_monitor::tui::run_tui;

let app = attach(&host, MonitorConfig::load(None));
run_tui(&app)?; // 或 run_tui_opts(&app, TuiOpts { demo: false, .. })
```

### 键位

| 键 | 作用 |
|----|------|
| `q` / `Esc` | 退出 |
| `Tab` | 在 chains / events / **graph** 间切换焦点（亦可用 `g` 打开图视图） |
| `↑` `↓` / `j` `k` | 滚动当前面板 |
| `PgUp` `PgDn` | 翻页 |
| `Home` `End` | 事件：更旧 / 回到最新；链：顶 / 底 |

### 能看到什么

- **标题栏**：`LIVE` 或 `DEMO`、刷新间隔、距上次刷新年龄、当前焦点、`dropped/queued/recent`
- **chains**：`ChainStatus` — `chain_id`、`instance`、`method`、`span`、`running`、`started_at`、`span_id` / `parent_span_id`、`ability_id`
- **events**：`MonitorEvent` — 时间、`seq`、`kind` / `phase`、instance、method、`ability_id`、chain/span 拓扑、`started_at` / `ended_at`、`from` / `to`、error
- **graph**：按 `span_id`→`parent_span_id` 重建的调用树（无 parent 不臆造边）

## 其它入口

| 入口 | 说明 |
|------|------|
| `monitor log --file monitor.jsonl --tail 64` | 离线 JSONL |
| `leaf_monitor::cli::run_cli` | 进程内 chains / events / log / **stats** / **graph** |
| HTTP（默认关） | `GET /monitor/chains`、`/events`、`/log`、`/stats`、`/graph` |
| `fun_call` | `monitor.chains` / `events` / `log` / `stats` / `graph` |

配置见 `monitor.yml`（缺省用 `MonitorConfig::default`）：`log_path`、`queue_capacity`、`recent_capacity`、`http.enabled`/`bind`、`tui_refresh_ms`。

## 建模字段（链路 / 状态 / 统计）

事件与链快照携带拓扑与时间字段，客户端可据此建图，无需猜测：

| 字段 | 位置 | 用途 |
|------|------|------|
| `span_id` / `parent_span_id` | MonitorEvent、ChainStatus | 显式调用树边（无 parent ⇒ 不臆造边） |
| `ability_id` | Event / ChainStatus | 能力 id，与实例名区分 |
| `started_at` / `ended_at` | Event（invoke） | 段起止 / 可算耗时 |
| `from` / `to` | lifecycle / host | 状态转移边 |
| `seq` | Event | 进程内顺序 |
| `dropped` / `queued` / `recent` | `monitor.stats` | 可感知丢弃的队列统计 |

查询：CLI `Stats`/`Graph`，HTTP `GET /monitor/stats` + `GET /monitor/graph`，fun `monitor.stats` / `monitor.graph`。  
重建：`leaf_monitor::graph::build_chain_graph(&events)`。  
TUI：`g` 打开图树（Tab 在 chains → events → graph 循环）；标题栏显示 `dropped/queued/recent`。
