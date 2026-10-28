//! Claude Mod 源码中常见的 `$.ns.method` 出站面（静态契约）。

/// 已映射到 [`dispatch_host_api`] 的出站调用。
pub fn mod_outbound_surfaces() -> &'static [(&'static str, &'static str)] {
    &[
        ("fs", "read"),
        ("fs", "exists"),
        ("tools", "execute"),
        ("tools", "register"),
        ("process", "run"),
        ("session", "usage"),
        ("session", "cwd"),
        ("session", "get"),
        ("session", "append_user"),
        ("session", "append_assistant"),
        ("llm", "complete"),
        ("ui", "invalidate"),
        ("ui", "resolve"),
        ("agents", "get"),
        ("clock", "now"),
        ("clock", "sleep"),
        ("command", "register"),
    ]
}

pub fn is_mapped_outbound(ns: &str, method: &str) -> bool {
    if ns == "ui" {
        return matches!(
            method,
            "invalidate" | "resolve" | "render" | "show" | "hide"
        );
    }
    mod_outbound_surfaces()
        .iter()
        .any(|(n, m)| *n == ns && *m == method)
}
