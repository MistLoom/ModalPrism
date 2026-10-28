//! Claude Mod 形态与 compat 门禁。

use std::path::PathBuf;

use ability_plugin_host::{
    ClaudeModShape, UNSUPPORTED_HOOKS_MODULES, classify_claude_mod_root,
    ensure_claude_mod_path_allowed, should_skip_vendor_claude_plugin,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn hooks_command_fixture_ok() {
    let root = fixture("token-weather");
    assert_eq!(
        classify_claude_mod_root(&root).expect("classify"),
        ClaudeModShape::HooksCommand
    );
    ensure_claude_mod_path_allowed(&root).expect("allowed");
    assert!(!should_skip_vendor_claude_plugin(&root).expect("skip"));
}

#[test]
fn hooks_modules_rejected_on_explicit_path() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hooks_module");
    assert_eq!(
        classify_claude_mod_root(&root).expect("classify"),
        ClaudeModShape::HooksModuleTs
    );
    let err = ensure_claude_mod_path_allowed(&root).expect_err("reject");
    assert!(err.to_string().contains(UNSUPPORTED_HOOKS_MODULES));
}

#[test]
fn hooks_modules_skipped_for_vendor_expand() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hooks_module");
    assert!(should_skip_vendor_claude_plugin(&root).expect("skip"));
}
