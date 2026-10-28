//! Rebuild a call-chain graph from monitor events.
//!
//! Edges are created **only** when both `span_id` and `parent_span_id` are present.
//! Depth/`span` alone is never used to invent parent links.

use ability_support::monitor::MonitorEvent;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One graph node keyed by `span_id`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphNode {
    pub span_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ability_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Directed edge parent -> child (only when parent_span_id was explicit).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphEdge {
    pub parent_span_id: String,
    pub span_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ChainGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// Build a chain graph from a slice of monitor events.
pub fn build_chain_graph(events: &[MonitorEvent]) -> ChainGraph {
    let mut nodes: BTreeMap<String, GraphNode> = BTreeMap::new();
    let mut edges: BTreeSet<(String, String)> = BTreeSet::new();

    for ev in events {
        let Some(span_id) = ev.span_id.as_ref() else {
            continue;
        };
        let entry = nodes.entry(span_id.clone()).or_insert_with(|| GraphNode {
            span_id: span_id.clone(),
            chain_id: ev.chain_id.clone(),
            instance: ev.instance.clone(),
            method: ev.method.clone(),
            ability_id: ev.ability_id.clone(),
            span: ev.span,
            started_at: None,
            ended_at: None,
            error: None,
        });
        if entry.chain_id.is_none() {
            entry.chain_id = ev.chain_id.clone();
        }
        if entry.instance.is_none() {
            entry.instance = ev.instance.clone();
        }
        if entry.method.is_none() {
            entry.method = ev.method.clone();
        }
        if entry.ability_id.is_none() {
            entry.ability_id = ev.ability_id.clone();
        }
        if entry.span.is_none() {
            entry.span = ev.span;
        }
        if let Some(s) = ev.started_at {
            entry.started_at = Some(entry.started_at.map_or(s, |o| o.min(s)));
        } else if ev.phase.as_deref() == Some("before") {
            entry.started_at = Some(entry.started_at.map_or(ev.at, |o| o.min(ev.at)));
        }
        if let Some(e) = ev.ended_at {
            entry.ended_at = Some(entry.ended_at.map_or(e, |o| o.max(e)));
        } else if matches!(ev.phase.as_deref(), Some("after") | Some("error")) {
            entry.ended_at = Some(entry.ended_at.map_or(ev.at, |o| o.max(ev.at)));
        }
        if let Some(err) = &ev.error {
            entry.error = Some(err.clone());
        }

        if let Some(parent) = &ev.parent_span_id {
            edges.insert((parent.clone(), span_id.clone()));
        }
    }

    ChainGraph {
        nodes: nodes.into_values().collect(),
        edges: edges
            .into_iter()
            .map(|(parent_span_id, span_id)| GraphEdge {
                parent_span_id,
                span_id,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ability_support::monitor::MonitorEvent;

    #[test]
    fn graph_uses_explicit_parent_only() {
        let evs = vec![
            MonitorEvent::new("invoke", 1)
                .phase("before")
                .instance("parent")
                .method("run")
                .chain("c1", 0)
                .span_id("s0")
                .started_at(1),
            MonitorEvent::new("invoke", 2)
                .phase("before")
                .instance("child")
                .method("run")
                .chain("c1", 1)
                .span_id("s1")
                .parent_span_id("s0")
                .started_at(2),
            MonitorEvent::new("invoke", 3)
                .phase("after")
                .instance("child")
                .method("run")
                .chain("c1", 1)
                .span_id("s1")
                .parent_span_id("s0")
                .ended_at(3),
            // orphan with span_id but no parent — node only, no invented edge
            MonitorEvent::new("invoke", 4)
                .phase("before")
                .span_id("orphan")
                .chain("c2", 0),
        ];
        let g = build_chain_graph(&evs);
        assert_eq!(g.nodes.len(), 3);
        assert_eq!(g.edges.len(), 1);
        assert_eq!(g.edges[0].parent_span_id, "s0");
        assert_eq!(g.edges[0].span_id, "s1");
        let child = g.nodes.iter().find(|n| n.span_id == "s1").unwrap();
        assert_eq!(child.started_at, Some(2));
        assert_eq!(child.ended_at, Some(3));
    }
}
