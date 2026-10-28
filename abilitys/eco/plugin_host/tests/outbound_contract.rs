//! 出站契约：声明面与 `is_mapped_outbound`、典型 Mod fixture 出站一致。

use ability_plugin_host::{is_mapped_outbound, mod_outbound_surfaces};

#[test]
fn declared_surfaces_are_mappable() {
    for (ns, method) in mod_outbound_surfaces() {
        assert!(is_mapped_outbound(ns, method), "{ns}.{method}");
    }
}

#[test]
fn ui_namespace_declared_methods() {
    assert!(is_mapped_outbound("ui", "invalidate"));
    assert!(is_mapped_outbound("ui", "render"));
    assert!(!is_mapped_outbound("ui", "custom_panel"));
}

#[test]
fn unknown_host_surface_not_mapped() {
    assert!(!is_mapped_outbound("network", "fetch"));
}

/// 与 `tests/fixtures` 里 Mod 脚本实际出现的 `$.ns.method` 对齐。
#[test]
fn compat_fixture_outbound_calls_are_mapped() {
    for (ns, method) in [("session", "usage"), ("session", "cwd"), ("agents", "get")] {
        assert!(is_mapped_outbound(ns, method), "{ns}.{method}");
    }
}
