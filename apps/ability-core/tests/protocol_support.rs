use ability_core::AbilityHost;
use frame_core::AccessKind;

#[test]
fn protocol_support_returns_registered_kinds() {
    let host = AbilityHost::new();
    host.set_protocol_support(
        "demo",
        vec![AccessKind::Http, AccessKind::Cli],
    );
    let kinds = host.protocol_support("demo");
    assert_eq!(kinds, vec![AccessKind::Http, AccessKind::Cli]);
}

#[test]
fn protocol_support_missing_instance_is_empty() {
    let host = AbilityHost::new();
    assert!(host.protocol_support("missing").is_empty());
}