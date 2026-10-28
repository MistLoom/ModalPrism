//! TUI：实时双面板（在途链 + 事件流）。
//!
//! - 默认按 `MonitorConfig::tui_refresh_ms` 刷新，标题栏显示 LIVE/DEMO 与距上次刷新年龄
//! - Tab 切换面板焦点；↑↓/jk / PgUp/PgDn 滚动；q / Esc 退出
//! - 终端变小只影响可视行数，不 panic
//! - 独立进程可用 `monitor tui --demo`：内置事件泵 + 慢调用，UI 标明 DEMO

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ability_support::Result;
use crossterm::event::{Event, KeyCode, KeyEventKind, poll, read};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::MonitorApp;

/// TUI 运行选项。
#[derive(Clone, Debug)]
pub struct TuiOpts {
    /// 标题栏标明 DEMO（独立 bin / smoke 用）。
    pub demo: bool,
    /// 覆盖配置里的刷新间隔（毫秒）；`None` 用 `app.config.tui_refresh_ms`。
    pub refresh_ms: Option<u64>,
}

impl Default for TuiOpts {
    fn default() -> Self {
        Self {
            demo: false,
            refresh_ms: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    Chains,
    Events,
    Graph,
}

pub(crate) struct ViewState {
    focus: Focus,
    /// 事件面板：从最新端往上滚的偏移（0 = 贴底看最新）。
    events_scroll: usize,
    /// 在途链面板：从列表头往下的偏移。
    chains_scroll: usize,
    graph_scroll: usize,
    last_refresh: Instant,
    demo: bool,
    refresh_ms: u64,
}

impl ViewState {
    fn new(demo: bool, refresh_ms: u64) -> Self {
        Self {
            focus: Focus::Events,
            events_scroll: 0,
            chains_scroll: 0,
            graph_scroll: 0,
            last_refresh: Instant::now(),
            demo,
            refresh_ms,
        }
    }
}

/// 运行 TUI 直到按 `q` / Esc（嵌入宿主、非 demo）。
pub fn run_tui(app: &MonitorApp) -> Result<()> {
    run_tui_opts(app, TuiOpts::default())
}

/// 带选项运行 TUI。
pub fn run_tui_opts(app: &MonitorApp, opts: TuiOpts) -> Result<()> {
    enable_raw_mode().map_err(|e| ability_support::Error::io(e.to_string()))?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)
        .map_err(|e| ability_support::Error::io(e.to_string()))?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal =
        Terminal::new(backend).map_err(|e| ability_support::Error::io(e.to_string()))?;

    let refresh_ms = opts.refresh_ms.unwrap_or(app.config.tui_refresh_ms).max(50);
    let mut state = ViewState::new(opts.demo, refresh_ms);
    let result = tui_loop(&mut terminal, app, &mut state);

    crossterm::execute!(io::stdout(), LeaveAlternateScreen)
        .map_err(|e| ability_support::Error::io(e.to_string()))?;
    disable_raw_mode().map_err(|e| ability_support::Error::io(e.to_string()))?;
    result
}

fn tui_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &MonitorApp,
    state: &mut ViewState,
) -> Result<()> {
    let refresh = Duration::from_millis(state.refresh_ms);
    loop {
        state.last_refresh = Instant::now();
        terminal
            .draw(|f| draw(f, app, state))
            .map_err(|e| ability_support::Error::io(e.to_string()))?;

        if poll(refresh).map_err(|e| ability_support::Error::io(e.to_string()))? {
            match read().map_err(|e| ability_support::Error::io(e.to_string()))? {
                Event::Key(key)
                    if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat =>
                {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('g') => state.focus = Focus::Graph,
                        KeyCode::Char('c') => state.focus = Focus::Chains,
                        KeyCode::Char('e') => state.focus = Focus::Events,
                        KeyCode::Tab => {
                            state.focus = match state.focus {
                                Focus::Chains => Focus::Events,
                                Focus::Events => Focus::Graph,
                                Focus::Graph => Focus::Chains,
                            };
                        }
                        KeyCode::Up | KeyCode::Char('k') => scroll(state, -1),
                        KeyCode::Down | KeyCode::Char('j') => scroll(state, 1),
                        KeyCode::PageUp => scroll(state, -10),
                        KeyCode::PageDown => scroll(state, 10),
                        KeyCode::Home => match state.focus {
                            Focus::Events => state.events_scroll = usize::MAX / 4,
                            Focus::Chains => state.chains_scroll = 0,
                            Focus::Graph => state.graph_scroll = 0,
                        },
                        KeyCode::End => match state.focus {
                            Focus::Events => state.events_scroll = 0,
                            Focus::Chains => state.chains_scroll = usize::MAX / 4,
                            Focus::Graph => state.graph_scroll = usize::MAX / 4,
                        },
                        _ => {}
                    }
                }
                Event::Resize(_, _) => {
                    // 下一帧按新尺寸重绘即可，不崩
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn scroll(state: &mut ViewState, delta: i32) {
    let apply = |v: &mut usize, delta: i32| {
        if delta > 0 {
            *v = v.saturating_add(delta as usize);
        } else {
            *v = v.saturating_sub((-delta) as usize);
        }
    };
    match state.focus {
        Focus::Events => apply(&mut state.events_scroll, delta),
        Focus::Chains => apply(&mut state.chains_scroll, delta),
        Focus::Graph => apply(&mut state.graph_scroll, delta),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn fmt_clock(at_ms: u64) -> String {
    let secs = at_ms / 1000;
    let ms = at_ms % 1000;
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    format!("{h:02}:{m:02}:{s:02}.{ms:03}")
}

fn fmt_age(at_ms: u64) -> String {
    let age = now_ms().saturating_sub(at_ms);
    if age < 1000 {
        format!("{age}ms")
    } else if age < 60_000 {
        format!("{:.1}s", age as f64 / 1000.0)
    } else {
        format!("{}m", age / 60_000)
    }
}

/// 绘制实时双面板（真实 TUI 与 TestBackend 冒烟共用）。
pub(crate) fn draw(f: &mut ratatui::Frame, app: &MonitorApp, state: &ViewState) {
    let area = f.area();
    // 极小终端：至少还能画一行，不 panic
    if area.width < 10 || area.height < 3 {
        f.render_widget(Paragraph::new("terminal too small"), area);
        return;
    }

    let [header, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(2)]).areas(area);
    let age = state.last_refresh.elapsed();
    let mode = if state.demo { "DEMO" } else { "LIVE" };
    let focus_hint = match state.focus {
        Focus::Chains => "focus=chains",
        Focus::Events => "focus=events",
        Focus::Graph => "focus=graph",
    };
    let st = app.stats();
    let header_line = Line::from(vec![
        Span::styled(
            format!(" LEAF monitor · {mode} "),
            Style::default()
                .fg(if state.demo {
                    Color::Yellow
                } else {
                    Color::Green
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            " refresh={}ms  last={:.0}ms ago  {focus_hint}  dropped={} queued={} recent={}  | Tab/g/c/e  ↑↓/jk  q",
            state.refresh_ms,
            age.as_secs_f64() * 1000.0,
            st.dropped,
            st.queued,
            st.recent,
        )),
    ]);
    f.render_widget(Paragraph::new(header_line), header);

    if state.focus == Focus::Graph {
        let g = app.chain_graph();
        let title = "● monitor · graph (span_id edges; g=graph c=chains e=events)";
        let mut lines: Vec<Line> = vec![Line::from(Span::styled(
            format!(
                "nodes={} edges={}  | edge only when parent_span_id present",
                g.nodes.len(),
                g.edges.len()
            ),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ))];
        // Build adjacency for a simple tree dump
        use std::collections::{BTreeMap, BTreeSet};
        let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut has_parent: BTreeSet<String> = BTreeSet::new();
        for e in &g.edges {
            children
                .entry(e.parent_span_id.clone())
                .or_default()
                .push(e.span_id.clone());
            has_parent.insert(e.span_id.clone());
        }
        let node_map: BTreeMap<_, _> = g.nodes.iter().map(|n| (n.span_id.clone(), n)).collect();
        let mut roots: Vec<String> = g
            .nodes
            .iter()
            .map(|n| n.span_id.clone())
            .filter(|id| !has_parent.contains(id))
            .collect();
        roots.sort();
        fn fmt_node(n: &crate::graph::GraphNode) -> String {
            format!(
                "{}  {}::{}  span={}  {}..{}{}",
                truncate(&n.span_id, 12),
                truncate(n.instance.as_deref().unwrap_or("-"), 12),
                truncate(n.method.as_deref().unwrap_or("-"), 10),
                n.span.map(|s| s.to_string()).unwrap_or_else(|| "-".into()),
                n.started_at
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "-".into()),
                n.ended_at
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "-".into()),
                n.error
                    .as_deref()
                    .map(|e| format!(" err={}", truncate(e, 20)))
                    .unwrap_or_default(),
            )
        }
        fn walk(
            id: &str,
            depth: usize,
            children: &BTreeMap<String, Vec<String>>,
            node_map: &BTreeMap<String, &crate::graph::GraphNode>,
            lines: &mut Vec<Line>,
        ) {
            let pad = "  ".repeat(depth);
            if let Some(n) = node_map.get(id) {
                lines.push(Line::from(format!("{pad}└─ {}", fmt_node(n))));
            } else {
                lines.push(Line::from(format!("{pad}└─ {id}")));
            }
            if let Some(kids) = children.get(id) {
                for k in kids {
                    walk(k, depth + 1, children, node_map, lines);
                }
            }
        }
        if roots.is_empty() && g.nodes.is_empty() {
            lines.push(Line::from("(no span_id events yet)"));
        } else if roots.is_empty() {
            for n in &g.nodes {
                lines.push(Line::from(fmt_node(n)));
            }
        } else {
            for r in &roots {
                walk(r, 0, &children, &node_map, &mut lines);
            }
        }
        // also list raw edges
        if !g.edges.is_empty() {
            lines.push(Line::from(Span::styled(
                "edges:",
                Style::default().fg(Color::DarkGray),
            )));
            for e in &g.edges {
                lines.push(Line::from(format!(
                    "  {} -> {}",
                    truncate(&e.parent_span_id, 16),
                    truncate(&e.span_id, 16)
                )));
            }
        }
        let visible_h = body.height.saturating_sub(3) as usize;
        let max_scroll = lines.len().saturating_sub(visible_h.max(1));
        let gscroll = state.graph_scroll.min(max_scroll);
        let view: Vec<Line> = lines
            .into_iter()
            .skip(gscroll)
            .take(visible_h.max(1))
            .collect();
        f.render_widget(
            Paragraph::new(view)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Magenta))
                        .title(title),
                )
                .wrap(Wrap { trim: false }),
            body,
        );
        return;
    }

    let [top, bottom] =
        Layout::vertical([Constraint::Percentage(35), Constraint::Percentage(65)]).areas(body);

    let chains = app.chains();
    let chain_title = if state.focus == Focus::Chains {
        "▶ monitor · chains (在途)"
    } else {
        "monitor · chains (在途)"
    };
    let mut chain_lines: Vec<Line> = vec![Line::from(Span::styled(
        format!(
            "在途链路 {}  | 字段: chain_id  instance  method  span  running",
            chains.len()
        ),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))];
    let visible_h = top.height.saturating_sub(3) as usize;
    let max_scroll = chains.len().saturating_sub(visible_h.max(1));
    let cscroll = state.chains_scroll.min(max_scroll);
    for c in chains.iter().skip(cscroll).take(visible_h.max(1)) {
        // ChainStatus 仅有这些字段；无开始时间则不编造 duration
        chain_lines.push(Line::from(format!(
            "{:<18}  {:<12}  {:<8}  span={}  {}  t={}  id={}",
            truncate(&c.chain_id, 18),
            truncate(&c.instance, 12),
            truncate(c.method.as_deref().unwrap_or("-"), 8),
            c.span,
            if c.running { "run" } else { "idle" },
            c.started_at
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into()),
            truncate(c.span_id.as_deref().unwrap_or("-"), 10),
        )));
    }
    if chains.is_empty() {
        chain_lines.push(Line::from("(无在途链路 — 等待 Host invoke)"));
    } else if cscroll > 0 || chains.len() > visible_h {
        chain_lines.push(Line::from(format!("… scroll {cscroll}/{} …", max_scroll)));
    }
    let chain_border = if state.focus == Focus::Chains {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    f.render_widget(
        Paragraph::new(chain_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(chain_border)
                    .title(chain_title),
            )
            .wrap(Wrap { trim: false }),
        top,
    );

    let evs = app.recent_events();
    let ev_title = if state.focus == Focus::Events {
        "▶ monitor · events (实时流)"
    } else {
        "monitor · events (实时流)"
    };
    let visible_e = bottom.height.saturating_sub(3) as usize;
    let max_escroll = evs.len().saturating_sub(visible_e.max(1));
    let escroll = state.events_scroll.min(max_escroll);
    // 最新在后：从末尾往前取窗口，再按时间正序画
    let end = evs.len().saturating_sub(escroll);
    let start = end.saturating_sub(visible_e.max(1));
    let mut ev_lines: Vec<Line> = Vec::new();
    if evs.is_empty() {
        ev_lines.push(Line::from("(尚无事件 — demo 模式会自动泵入)"));
    } else {
        for ev in &evs[start..end] {
            let kind_style = match ev.kind.as_str() {
                "invoke" => Style::default().fg(Color::Green),
                "lifecycle" => Style::default().fg(Color::Magenta),
                "host" => Style::default().fg(Color::Blue),
                _ => Style::default().fg(Color::Gray),
            };
            let phase = ev.phase.as_deref().unwrap_or("-");
            let phase_style = match phase {
                "error" => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                "before" => Style::default().fg(Color::Yellow),
                "after" => Style::default().fg(Color::Green),
                "load" | "unload" => Style::default().fg(Color::Blue),
                _ => Style::default(),
            };
            let chain = ev
                .chain_id
                .as_deref()
                .map(|c| truncate(c, 10))
                .unwrap_or_else(|| "-".into());
            let span = ev.span.map(|s| s.to_string()).unwrap_or_else(|| "-".into());
            let err = ev
                .error
                .as_deref()
                .map(|e| format!(" err={}", truncate(e, 24)))
                .unwrap_or_default();
            ev_lines.push(Line::from(vec![
                Span::raw(format!("{} +{:<6} ", fmt_clock(ev.at), fmt_age(ev.at))),
                Span::styled(format!("[{:<9}]", ev.kind), kind_style),
                Span::styled(format!(" {phase:<7}"), phase_style),
                Span::raw(format!(
                    " {inst:<14} {meth:<10} chain={chain} span={span}{err}",
                    inst = truncate(ev.instance.as_deref().unwrap_or("-"), 14),
                    meth = truncate(ev.method.as_deref().unwrap_or("-"), 10),
                )),
            ]));
        }
        if escroll > 0 {
            ev_lines.insert(
                0,
                Line::from(Span::styled(
                    format!("↑ 更旧 · 已上滚 {escroll} 条 · End 回到最新"),
                    Style::default().fg(Color::DarkGray),
                )),
            );
        }
    }
    let ev_border = if state.focus == Focus::Events {
        Style::default().fg(Color::Green)
    } else {
        Style::default()
    };
    f.render_widget(
        Paragraph::new(ev_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(ev_border)
                .title(ev_title),
        ),
        bottom,
    );
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// Demo 事件泵：周期性 `emit` 合成监测事件，让无完整业务 Host 也能看到实时滚动。
/// 返回 stop 开关；置 true 后泵退出。
pub fn start_demo_pump(interval_ms: u64) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);
    std::thread::spawn(move || {
        let mut i = 0u64;
        let kinds = ["invoke", "lifecycle", "host"];
        let phases = [
            ("invoke", "before"),
            ("invoke", "after"),
            ("lifecycle", "Ready"),
            ("host", "load"),
            ("host", "unload"),
            ("invoke", "error"),
        ];
        while !stop2.load(Ordering::SeqCst) {
            let (kind, phase) = phases[i as usize % phases.len()];
            let _ = kinds; // 文档性：仅用 phases 对
            let span_id = format!("demo-span-{i}");
            let mut ev = ability_support::monitor::MonitorEvent::new(kind, now_ms())
                .phase(phase)
                .instance(format!("demo-inst-{}", i % 3))
                .method(if kind == "invoke" { "run" } else { "-" })
                .chain(format!("demo-chain-{}", i % 5), i % 3)
                .span_id(&span_id)
                .ability_id(format!("demo-ability-{}", i % 3))
                .started_at(now_ms());
            if i % 3 != 0 && kind == "invoke" {
                ev = ev.parent_span_id(format!("demo-span-{}", i.saturating_sub(1)));
            }
            if phase == "after" || phase == "error" {
                ev = ev.ended_at(now_ms());
            }
            if phase == "error" {
                ev = ev.error("demo simulated error");
            }
            if kind == "lifecycle" {
                ev = ev.from_state("Ready").to_state(phase);
            }
            ability_support::monitor::emit(ev);
            i += 1;
            std::thread::sleep(Duration::from_millis(interval_ms.max(50)));
        }
    });
    stop
}

#[cfg(test)]
mod tests {
    use super::*;
    use ability_core::AbilityHost;
    use ratatui::backend::TestBackend;

    use crate::{MonitorConfig, attach};

    #[test]
    fn tui_draw_smoke_testbackend() {
        let host = AbilityHost::new();
        let app = attach(
            &host,
            MonitorConfig {
                log_path: String::new(),
                ..MonitorConfig::default()
            },
        );
        ability_support::monitor::emit(
            ability_support::monitor::MonitorEvent::new("invoke", now_ms())
                .phase("before")
                .instance("echo")
                .method("run")
                .chain("c1", 0),
        );
        ability_support::monitor::emit(
            ability_support::monitor::MonitorEvent::new("host", now_ms())
                .phase("load")
                .instance("echo"),
        );
        let backend = TestBackend::new(100, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = ViewState::new(true, 200);
        terminal.draw(|f| draw(f, &app, &state)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let flat: String = buf
            .content()
            .iter()
            .map(|c| c.symbol().to_string())
            .collect();
        assert!(
            flat.contains("DEMO") || flat.contains("LIVE") || flat.contains("monitor"),
            "should render header: {flat}"
        );
        assert!(
            flat.contains("chains") || flat.contains("在途"),
            "should render chains panel: {flat}"
        );
        assert!(
            flat.contains("events") || flat.contains("invoke"),
            "should render events panel: {flat}"
        );
        // 极小尺寸不崩
        let backend2 = TestBackend::new(5, 2);
        let mut t2 = Terminal::new(backend2).unwrap();
        t2.draw(|f| draw(f, &app, &state)).unwrap();
        app.shutdown();
    }

    #[test]
    fn scroll_and_focus_helpers() {
        let mut s = ViewState::new(false, 100);
        assert_eq!(s.focus, Focus::Events);
        s.focus = Focus::Chains;
        scroll(&mut s, 3);
        assert_eq!(s.chains_scroll, 3);
        scroll(&mut s, -1);
        assert_eq!(s.chains_scroll, 2);
        s.focus = Focus::Events;
        scroll(&mut s, 5);
        assert_eq!(s.events_scroll, 5);
    }
}
