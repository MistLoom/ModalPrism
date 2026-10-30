//! 文档与路由一致性：`src/external/endpoints.rs` 登记的方法必须与 `src/*.rs` 里
//! 真实注册的 `.route(...)` 完全一致。
//!
//! 外部调用页展示的就是这张表；少一条或多一条都会让调用者在按图索骥时失败，
//! 所以把「加了路由忘了补文档」变成测试失败，而不是运行期才发现。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// 逐个源文件找出 `.route(...)` 注册的 (METHOD, path)。
fn registered_routes() -> BTreeSet<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs(&root, &mut files);
    let mut routes = BTreeSet::new();
    for file in files {
        let text = fs::read_to_string(&file).expect("读取源文件");
        for segment in route_segments(&text) {
            let Some(path) = first_string(&segment) else {
                continue;
            };
            if !path.starts_with('/') {
                continue;
            }
            for verb in ["get", "post", "patch", "put", "delete"] {
                if segment.contains(&format!("{verb}(")) {
                    routes.insert((verb.to_uppercase(), path.clone()));
                }
            }
        }
    }
    routes
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// 取出每个 `.route(` 到配对右括号之间的片段（跳过字符串里的括号）。
fn route_segments(text: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    while let Some(found) = text[index..].find(".route(") {
        let open = index + found + ".route".len();
        let mut depth = 0usize;
        let mut cursor = open;
        let mut in_string = false;
        while cursor < bytes.len() {
            let byte = bytes[cursor];
            if in_string {
                if byte == b'\\' {
                    cursor += 2;
                    continue;
                }
                if byte == b'"' {
                    in_string = false;
                }
            } else if byte == b'"' {
                in_string = true;
            } else if byte == b'(' {
                depth += 1;
            } else if byte == b')' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            cursor += 1;
        }
        if cursor <= text.len() {
            segments.push(text[open..cursor.min(text.len())].to_string());
        }
        index = cursor.max(index + 1);
    }
    segments
}

fn first_string(segment: &str) -> Option<String> {
    let start = segment.find('"')? + 1;
    let mut value = String::new();
    let mut escaped = false;
    for ch in segment[start..].chars() {
        if escaped {
            value.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Some(value),
            other => value.push(other),
        }
    }
    None
}

#[test]
fn documented_endpoints_match_router() {
    let routes = registered_routes();
    let documented: BTreeSet<(String, String)> = domain_api::external::documented()
        .into_iter()
        .map(|(_, method, path)| (method.to_string(), path.to_string()))
        .collect();
    let missing: Vec<_> = routes.difference(&documented).cloned().collect();
    let extra: Vec<_> = documented.difference(&routes).cloned().collect();
    assert!(
        missing.is_empty(),
        "这些路由没有写进外部调用清单（src/external/endpoints.rs）：{missing:?}"
    );
    assert!(
        extra.is_empty(),
        "外部调用清单里有路由表不存在的条目：{extra:?}"
    );
}

/// MCP 工具清单与只读进程的白名单同源：任一侧增删方法，这里都会失败。
#[test]
fn mcp_tools_match_agent_whitelist() {
    let documented: BTreeSet<(String, String)> = domain_api::external::mcp_documented()
        .into_iter()
        .map(|(ability, method)| (ability.to_string(), method.to_string()))
        .collect();
    let mut expected = BTreeSet::new();
    for exposure in surface::AGENT_MCP
        .iter()
        .chain(surface::AGENT_MCP_WRITES.iter())
    {
        for method in exposure.methods {
            expected.insert((exposure.id.to_string(), method.to_string()));
        }
    }
    assert!(!expected.is_empty(), "只读进程没有声明任何工具");
    assert_eq!(
        documented, expected,
        "MCP 工具清单与只读进程的白名单不一致"
    );
}

#[test]
fn every_group_is_declared_and_used() {
    let groups = domain_api::external::group_ids();
    let mut used = BTreeSet::new();
    for (group, _, _) in domain_api::external::documented() {
        assert!(
            groups.contains(&group),
            "接口登记了未声明的分组：{group}"
        );
        used.insert(group);
    }
    for group in &groups {
        assert!(used.contains(group), "分组 {group} 下没有任何接口");
    }
}
