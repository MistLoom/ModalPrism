use console_api::normalize_rel_path;

#[test]
fn allows_whitelist() {
    assert!(normalize_rel_path("sources/a.yaml").is_ok());
    assert!(normalize_rel_path("presets/minimal.yaml").is_ok());
    assert!(normalize_rel_path("system.yaml").is_ok());
}

#[test]
fn rejects_escape() {
    assert!(normalize_rel_path("../system.yaml").is_err());
    assert!(normalize_rel_path("sources/../../etc/passwd").is_err());
    assert!(normalize_rel_path("/etc/passwd").is_err());
}

#[test]
fn rejects_unknown_root() {
    assert!(normalize_rel_path("secrets/x.yaml").is_err());
}
