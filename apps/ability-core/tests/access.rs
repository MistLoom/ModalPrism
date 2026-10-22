use ability_core::{AbilityGrant, AbilityHost};
use ability_support::{EventClass, EventFilter, EventMode, InstanceId};

fn grant(app: &str, open: bool) -> AbilityGrant {
    AbilityGrant {
        app: app.into(),
        open,
    }
}

#[test]
fn port_call_respects_open_and_owner() {
    let host = AbilityHost::new();
    host.set_ability_grant("read_file", grant("runtime", true));
    host.set_ability_grant("claude_mod", grant("web-agent", false));
    host.set_entry_app("web-agent");
    assert!(host.authorize_port("read_file").is_ok());
    assert!(host.authorize_port("claude_mod").is_ok());
    host.set_entry_app("runtime");
    assert!(host.authorize_port("read_file").is_ok());
    assert!(host.authorize_port("claude_mod").is_err());
}

#[test]
fn cross_app_call_requires_open() {
    let host = AbilityHost::new();
    host.set_ability_grant("read_file", grant("runtime", true));
    host.set_ability_grant("session", grant("agent-test", true));
    host.set_ability_grant("claude_mod", grant("web-agent", false));
    assert!(host.authorize_call("read_file", "session").is_ok());
    assert!(host.authorize_call("read_file", "claude_mod").is_err());
    assert!(host.authorize_call("claude_mod", "claude_mod").is_ok());
}

#[test]
fn common_ability_is_controllable_by_the_host_app() {
    let host = AbilityHost::new();
    host.set_ability_grant("read_file", grant("common", true));
    host.set_ability_grant("claude_mod", grant("web-agent", false));
    host.set_entry_app("web-agent");
    let filter = EventFilter {
        class: Some(EventClass::Ability),
        instance: Some(InstanceId::new("read_file").unwrap()),
        name: None,
    };
    assert!(host.authorize_port("read_file").is_ok());
    assert!(host.authorize_call("claude_mod", "read_file").is_ok());
    assert!(
        host.authorize_event("claude_mod", &filter, EventMode::Middleware)
            .is_ok()
    );
}

#[test]
fn intercept_stays_with_owner() {
    let host = AbilityHost::new();
    host.set_ability_grant("session", grant("agent-test", true));
    host.set_ability_grant("claude_mod", grant("web-agent", false));
    let filter = EventFilter {
        class: Some(EventClass::Ability),
        instance: Some(InstanceId::new("session").unwrap()),
        name: None,
    };
    assert!(
        host.authorize_event("claude_mod", &filter, EventMode::Message)
            .is_ok()
    );
    assert!(
        host.authorize_event("claude_mod", &filter, EventMode::Middleware)
            .is_err()
    );
    assert!(
        host.authorize_event("session", &filter, EventMode::Middleware)
            .is_ok()
    );
}
