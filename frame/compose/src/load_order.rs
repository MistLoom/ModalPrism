//! 装配运算：依赖图、拓扑序、启用建议、卸载影响。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use ability_support::load_ability_package;
use frame_core::AbilityExpose;
use serde::Serialize;
use serde_json::Value;

use crate::catalog::WorkspaceCatalog;

#[derive(Clone, Debug)]
pub struct DepEdge {
    pub from: String,
    pub to: String,
    pub optional: bool,
}

#[derive(Clone, Debug, Default)]
pub struct DepGraph {
    pub edges: Vec<DepEdge>,
    pub deps: HashMap<String, Vec<(String, bool)>>,
    pub dependents: HashMap<String, Vec<String>>,
}

pub fn build_dep_graph(catalog: &WorkspaceCatalog) -> DepGraph {
    let mut graph = DepGraph::default();
    for entry in catalog.sources() {
        let Ok(pkg) = load_ability_package(&entry.root) else {
            continue;
        };
        for dep in &pkg.decl.depends {
            let v = serde_json::to_value(dep).unwrap_or(Value::Null);
            let (to, optional) = dep_from_value(&v);
            if to.is_empty() {
                continue;
            }
            graph.edges.push(DepEdge {
                from: entry.id.clone(),
                to: to.clone(),
                optional,
            });
            graph
                .deps
                .entry(entry.id.clone())
                .or_default()
                .push((to.clone(), optional));
            if !optional {
                graph
                    .dependents
                    .entry(to)
                    .or_default()
                    .push(entry.id.clone());
            }
        }
    }
    graph
}

pub fn required_closure(graph: &DepGraph, targets: &[String]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut q = VecDeque::new();
    for t in targets {
        q.push_back(t.clone());
    }
    while let Some(id) = q.pop_front() {
        if !out.insert(id.clone()) {
            continue;
        }
        if let Some(deps) = graph.deps.get(&id) {
            for (dep, optional) in deps {
                if !optional {
                    q.push_back(dep.clone());
                }
            }
        }
    }
    out
}

/// 拓扑排序：依赖在前；同层按 id 字典序。
pub fn topo_sort(catalog: &WorkspaceCatalog, ids: &BTreeSet<String>) -> Vec<String> {
    let graph = build_dep_graph(catalog);
    let mut indegree: HashMap<String, usize> = ids.iter().map(|id| (id.clone(), 0)).collect();
    for id in ids {
        if let Some(deps) = graph.deps.get(id) {
            for (dep, optional) in deps {
                if !optional && ids.contains(dep) {
                    *indegree.entry(id.clone()).or_default() += 1;
                }
            }
        }
    }

    let mut q: Vec<String> = indegree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(k, _)| k.clone())
        .collect();
    q.sort();

    let mut sorted = Vec::new();
    while let Some(id) = q.first().cloned() {
        q.remove(0);
        sorted.push(id.clone());
        for other in ids {
            if other == &id {
                continue;
            }
            if let Some(deps) = graph.deps.get(other) {
                for (dep, optional) in deps {
                    if !optional && dep == &id {
                        let e = indegree.get_mut(other).expect("indeg");
                        *e = e.saturating_sub(1);
                        if *e == 0 && !sorted.contains(other) && !q.contains(other) {
                            q.push(other.clone());
                            q.sort();
                        }
                    }
                }
            }
        }
    }

    if sorted.len() < ids.len() {
        let mut rest: Vec<String> = ids
            .iter()
            .filter(|id| !sorted.contains(id))
            .cloned()
            .collect();
        rest.sort();
        sorted.extend(rest);
    }
    sorted
}

#[derive(Serialize)]
pub struct EnableSuggest {
    pub targets: Vec<String>,
    pub to_enable: Vec<String>,
    pub order: Vec<String>,
    pub missing: Vec<String>,
    pub already_enabled: Vec<String>,
}

pub fn suggest_enable(
    catalog: &WorkspaceCatalog,
    targets: &[String],
    expose: &AbilityExpose,
) -> EnableSuggest {
    let graph = build_dep_graph(catalog);
    let closure = required_closure(&graph, targets);
    let order = topo_sort(catalog, &closure);
    let mut missing = Vec::new();
    let mut already = Vec::new();
    for id in &order {
        if expose.resolve(id).enabled {
            already.push(id.clone());
        } else {
            missing.push(id.clone());
        }
    }
    EnableSuggest {
        targets: targets.to_vec(),
        to_enable: order.clone(),
        order,
        missing,
        already_enabled: already,
    }
}

#[derive(Serialize)]
pub struct UnloadImpact {
    pub instance: String,
    pub dependents: Vec<String>,
    pub loaded_dependents: Vec<String>,
    pub enabled_dependents: Vec<String>,
    pub safe: bool,
}

pub fn unload_impact(
    catalog: &WorkspaceCatalog,
    instance: &str,
    loaded: &HashSet<String>,
    expose: &AbilityExpose,
) -> UnloadImpact {
    let graph = build_dep_graph(catalog);
    let mut dependents: BTreeSet<String> = BTreeSet::new();
    collect_dependents(&graph, instance, &mut dependents);

    let dep_vec: Vec<String> = dependents.into_iter().collect();
    let loaded_dep: Vec<String> = dep_vec
        .iter()
        .filter(|d| loaded.contains(*d))
        .cloned()
        .collect();
    let enabled_dep: Vec<String> = dep_vec
        .iter()
        .filter(|d| expose.resolve(d).enabled)
        .cloned()
        .collect();

    UnloadImpact {
        instance: instance.to_string(),
        dependents: dep_vec,
        loaded_dependents: loaded_dep.clone(),
        enabled_dependents: enabled_dep,
        safe: loaded_dep.is_empty(),
    }
}

fn collect_dependents(graph: &DepGraph, id: &str, out: &mut BTreeSet<String>) {
    if let Some(list) = graph.dependents.get(id) {
        for d in list {
            if out.insert(d.clone()) {
                collect_dependents(graph, d, out);
            }
        }
    }
}

pub fn expose_to_yaml(expose: &AbilityExpose) -> Result<String, String> {
    serde_yaml::to_string(expose).map_err(|e| e.to_string())
}

pub fn write_expose_file(path: &std::path::Path, expose: &AbilityExpose) -> Result<(), String> {
    let yaml = expose_to_yaml(expose)?;
    std::fs::write(path, yaml).map_err(|e| e.to_string())
}

pub fn dep_from_value(dep: &Value) -> (String, bool) {
    if let Some(s) = dep.as_str() {
        return (s.to_string(), false);
    }
    let id = dep
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let optional = dep
        .get("optional")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    (id, optional)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shout_needs_echo() {
        let catalog = WorkspaceCatalog::discover();
        let graph = build_dep_graph(&catalog);
        let s = suggest_enable(&catalog, &["app.shout".into()], &AbilityExpose::open_all());
        assert!(s.to_enable.contains(&"algo.echo".to_string()));
        assert!(s.to_enable.contains(&"app.shout".to_string()));
        let echo_idx = s.order.iter().position(|x| x == "algo.echo").unwrap();
        let shout_idx = s.order.iter().position(|x| x == "app.shout").unwrap();
        assert!(echo_idx < shout_idx);
        let _ = graph;
    }
}
