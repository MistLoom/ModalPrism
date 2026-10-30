//! 工具面契约：一处新增、四面同步。
//!
//! 同一张动词表必须同时成立：
//! 1. `surface::TOOLS`（唯一事实源）；
//! 2. `ability/tool/ability.yml`（能力声明，CLI / fun / http / mcp 的协议投影依据）；
//! 3. `GET /tools` 的目录输出（控制台与外部集成读的那份）；
//! 4. MCP 面投影（只读面 + 写白名单的并集）。
//!
//! 任一处新增动词而忘了另一处，这个测试就会失败。

use std::path::Path;

#[test]
fn tool_verbs_match_the_ability_declaration() {
    let yml = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("ability/tool/ability.yml");
    let decl = ability_support::load_ability_decl(&yml).expect("tool 的 ability.yml 必须可解析");
    assert_eq!(decl.id, "tool");
    let mut declared: Vec<String> = decl.methods.iter().map(|item| item.id.clone()).collect();
    declared.sort();
    let mut table: Vec<String> = surface::tool_ids().iter().map(|id| id.to_string()).collect();
    table.sort();
    assert_eq!(
        declared, table,
        "ability.yml 的方法集必须与 surface::TOOLS 一致"
    );
    // 协议投影：CLI / fun / http / mcp 三面都要能装载这个能力。
    for protocol in ["fun", "http", "mcp"] {
        assert!(
            decl.protocols.iter().any(|item| item.dir_name() == protocol),
            "tool 能力必须声明 {protocol} 协议"
        );
    }
}

#[test]
fn tool_verbs_match_the_http_catalog() {
    let catalog = domain_api::tools::catalog();
    let items = catalog["items"].as_array().expect("目录是数组");
    let mut verbs: Vec<String> = items
        .iter()
        .filter_map(|item| item["verb"].as_str().map(str::to_string))
        .collect();
    verbs.sort();
    let mut table: Vec<String> = surface::tool_ids().iter().map(|id| id.to_string()).collect();
    table.sort();
    assert_eq!(verbs, table, "GET /tools 的动词集必须与表面表一致");
    assert_eq!(catalog["total"].as_u64(), Some(table.len() as u64));
    // 读写标记同源：决定 MCP 面的白名单归属。
    for item in items {
        let verb = item["verb"].as_str().unwrap_or_default();
        let spec = surface::tool(verb).expect("目录里的动词必须在表里");
        assert_eq!(
            item["write"].as_bool(),
            Some(spec.write),
            "{verb} 的读写标记与表不一致"
        );
    }
}

#[test]
fn tool_verbs_match_the_mcp_projection() {
    for id in surface::tool_ids() {
        let exposed = surface::exposes(surface::AGENT_MCP, "tool", id);
        let writable = surface::mcp_write_allowed("tool", id);
        assert!(
            exposed || writable,
            "MCP 面缺少工具动词 {id}：只读面与写白名单至少要开放一个"
        );
        let spec = surface::tool(id).expect("表里有这个动词");
        assert!(
            spec.write != exposed,
            "{id} 的读写归属与 MCP 面不一致（写动词不得进只读面）"
        );
    }
    // 表里没有的动词，MCP 面也不得放行。
    for (id, method) in [("tool", "commit"), ("tool", "delete"), ("tool", "purge")] {
        assert!(!surface::exposes(surface::AGENT_MCP, id, method));
        assert!(!surface::mcp_write_allowed(id, method));
    }
}
