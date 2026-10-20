//! 阶段 3 的退出标准。

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use config::{
    ConfigStore, EnvSource, LoadOptions, builtin_presets, diff, explain, overlay_system_policy,
    validate, watch,
};
use serde_json::json;

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn copy_presets(dir: &Path) {
    for preset in builtin_presets() {
        write(dir, &format!("presets/{}", preset.file_name), preset.source);
    }
}

fn system_yaml(policy_extra: &str) -> String {
    format!(
        "kind: system\nstrata:\n  staging:\n    dir: ./var/staging\n    ttl: 24h\npolicy:\n  preset: standard\n{policy_extra}"
    )
}

fn accessor_yaml() -> &'static str {
    r#"
id: accessor/http
kind: accessor
name: HTTP
implementation: builtin:http
limits:
  timeout: 30s
  concurrency: 4
  retry:
    attempts: 3
    backoff: exponential
    base: 1s
    max: 30s
egress:
  profile: direct
"#
}

fn source_yaml(policy: &str) -> String {
    format!(
        r#"
id: source/demo
kind: source
name: Demo
{policy}
access:
  accessor: accessor/http
  locator:
    url: https://example.com/a
pipeline: []
schedule:
  adaptive: true
  floor: 15m
  ceiling: 2h
  expectedPeriod: 7d
admission: {{}}
"#
    )
}

fn scaffold(dir: &Path) {
    copy_presets(dir);
    write(dir, "system.yaml", &system_yaml(""));
    write(dir, "accessors/http.yaml", accessor_yaml());
    write(dir, "sources/demo.yaml", &source_yaml(""));
}

fn options() -> LoadOptions {
    LoadOptions::default()
}

#[test]
fn preset_is_baseline_and_system_scope_follows_global_preset() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path());
    write(
        dir.path(),
        "profiles/gentle-crawl.yaml",
        r#"
id: profile/gentle-crawl
kind: profile
name: 温和抓取
egress:
  profile: per-domain-1rps
policy:
  render:
    max_level: L2
"#,
    );
    write(
        dir.path(),
        "sources/official/example-blog.yaml",
        r#"
id: source/example-blog
kind: source
name: Example
extends: [profile/gentle-crawl]
policy:
  preset: archive
  raw:
    retain: 180d
  media:
    kinds: [image]
access:
  accessor: accessor/http
  locator:
    url: https://example.com/blog/
pipeline: []
schedule:
  adaptive: true
  floor: 15m
  ceiling: 2h
  expectedPeriod: 7d
admission: {}
"#,
    );
    let snapshot = validate(dir.path(), &options()).unwrap();
    let source = snapshot.modules.get("source/example-blog").unwrap();
    assert_eq!(source.pointer("/policy/raw/retain"), Some(&json!("180d")));
    assert_eq!(source.pointer("/policy/raw/store"), Some(&json!("all")));
    assert_eq!(
        source.pointer("/policy/render/max_level"),
        Some(&json!("L2"))
    );
    assert_eq!(
        source.pointer("/policy/media/kinds"),
        Some(&json!(["image"]))
    );
    assert!(source.pointer("/policy/versions/snapshot_retain").is_none());
    assert_eq!(
        snapshot.system.pointer("/policy/versions/snapshot_retain"),
        Some(&json!("3d"))
    );

    let fields = explain(&snapshot, "source/example-blog").unwrap();
    let retain = fields
        .iter()
        .find(|field| field.path == "policy.raw.retain")
        .unwrap();
    assert_eq!(retain.value, json!("180d"));
    assert!(retain.file.ends_with("example-blog.yaml"));
    let line = fs::read_to_string(dir.path().join(&retain.file)).unwrap();
    assert!(
        line.lines()
            .nth(retain.line as usize - 1)
            .unwrap()
            .contains("180d")
    );

    let level = fields
        .iter()
        .find(|field| field.path == "policy.render.max_level")
        .unwrap();
    assert_eq!(level.value, json!("L2"));
    assert!(level.file.contains("gentle-crawl.yaml"));

    let store = fields
        .iter()
        .find(|field| field.path == "policy.raw.store")
        .unwrap();
    assert_eq!(store.value, json!("all"));
    assert_eq!(store.preset_id.as_deref(), Some("preset/standard"));
    assert_eq!(store.via_preset.as_deref(), Some("preset/archive"));

    let snap = fields
        .iter()
        .find(|field| field.path == "policy.versions.snapshot_retain")
        .unwrap();
    assert!(snap.system_scope);
    assert_eq!(snap.value, json!("3d"));
    assert!(snap.file.contains("standard.yaml"));
    assert!(snap.via_preset.is_none());
}

#[test]
fn invalid_reload_keeps_the_previous_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path());
    let store = ConfigStore::open(dir.path(), options()).unwrap();
    assert_eq!(store.snapshot().generation, 1);

    write(dir.path(), "sources/demo.yaml", &source_yaml("foo: 1\n"));
    let error = store.reload().unwrap_err();
    let text = error.to_string();
    assert!(text.contains("未知字段"), "{text}");
    assert_eq!(store.snapshot().generation, 1);
    assert!(store.snapshot().modules.contains_key("source/demo"));

    write(dir.path(), "sources/demo.yaml", &source_yaml(""));
    write(
        dir.path(),
        "sources/demo.yaml",
        &source_yaml("").replace("accessor/http", "accessor/missing"),
    );
    let error = store.reload().unwrap_err();
    assert!(error.to_string().contains("引用不存在"), "{}", error);
    assert_eq!(store.snapshot().generation, 1);

    write(
        dir.path(),
        "system.yaml",
        &system_yaml("  locks: [robots.mode]\n  robots:\n    mode: obey-with-exemptions\n"),
    );
    write(
        dir.path(),
        "sources/demo.yaml",
        &source_yaml("policy:\n  robots:\n    mode: ignore\n    robots_reason: because\n"),
    );
    let error = store.reload().unwrap_err();
    assert!(error.to_string().contains("锁定冲突"), "{}", error);
    assert_eq!(store.snapshot().generation, 1);
    assert!(store.snapshot().system.pointer("/policy/locks").is_none());
}

#[test]
fn keys_below_scope_fail() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path());
    write(
        dir.path(),
        "sources/demo.yaml",
        &source_yaml("policy:\n  versions:\n    snapshot_retain: 1d\n"),
    );
    let error = validate(dir.path(), &options()).unwrap_err();
    assert!(error.to_string().contains("低于作用域"), "{}", error);

    write(dir.path(), "sources/demo.yaml", &source_yaml(""));
    write(
        dir.path(),
        "profiles/locked.yaml",
        r#"
id: profile/locked
kind: profile
name: Locked
policy:
  raw:
    redact_headers: [cookie]
"#,
    );
    let error = validate(dir.path(), &options()).unwrap_err();
    assert!(error.to_string().contains("低于作用域"), "{}", error);
}

#[test]
fn profile_source_keys_do_not_land_on_accessors() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path());
    write(
        dir.path(),
        "profiles/gentle-crawl.yaml",
        r#"
id: profile/gentle-crawl
kind: profile
name: 温和抓取
policy:
  render:
    max_level: L2
"#,
    );
    let mut accessor = accessor_yaml().to_string();
    accessor.push_str("extends: [profile/gentle-crawl]\n");
    write(dir.path(), "accessors/http.yaml", &accessor);
    let snapshot = validate(dir.path(), &options()).unwrap();
    let accessor = snapshot.modules.get("accessor/http").unwrap();
    assert!(accessor.pointer("/policy/render/max_level").is_none());

    write(
        dir.path(),
        "accessors/http.yaml",
        &format!(
            "policy:\n  render:\n    max_level: L1\n{accessor_body}",
            accessor_body = accessor_yaml()
        ),
    );
    let error = validate(dir.path(), &options()).unwrap_err();
    assert!(error.to_string().contains("低于作用域"), "{}", error);
}

#[test]
fn restart_fields_stay_on_the_opening_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path());
    let store = ConfigStore::open(dir.path(), options()).unwrap();
    write(
        dir.path(),
        "system.yaml",
        &system_yaml("").replace("./var/staging", "./var/other"),
    );
    write(
        dir.path(),
        "sources/demo.yaml",
        &source_yaml("policy:\n  raw:\n    retain: 9d\n"),
    );
    store.reload().unwrap();
    assert_eq!(store.snapshot().generation, 2);
    assert_eq!(
        store.snapshot().system.pointer("/strata/staging/dir"),
        Some(&json!("./var/other"))
    );
    assert_eq!(
        store.applied().system.pointer("/strata/staging/dir"),
        Some(&json!("./var/staging"))
    );
    assert_eq!(
        store.applied().system.pointer("/strata/staging/ttl"),
        Some(&json!("24h"))
    );
    assert_eq!(
        store.applied().modules["source/demo"].pointer("/policy/raw/retain"),
        Some(&json!("9d"))
    );
    assert!(
        store
            .pending_restart()
            .iter()
            .any(|change| change.path == "system.strata.staging.dir")
    );
}

#[test]
fn list_ops_null_delete_and_diff() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path());
    write(
        dir.path(),
        "sources/demo.yaml",
        &source_yaml(
            "policy:\n  preset: archive\n  capture:\n    renditions:\n      $append: [semantic_tree_text]\n  media:\n    kinds: null\n",
        ),
    );
    let snapshot = validate(dir.path(), &options()).unwrap();
    let source = &snapshot.modules["source/demo"];
    assert_eq!(
        source.pointer("/policy/capture/renditions"),
        Some(&json!(["markdown", "semantic_tree_text"]))
    );
    assert!(source.pointer("/policy/media/kinds").is_none());
    assert_eq!(
        source.pointer("/policy/media/mode"),
        Some(&json!("download"))
    );

    write(dir.path(), "sources/demo.yaml", &source_yaml(""));
    let other = validate(dir.path(), &options()).unwrap();
    let changes = diff(&other, &snapshot);
    assert!(
        changes
            .iter()
            .any(|change| change.path.contains("renditions"))
    );
}

#[test]
fn duplicate_robots_env_and_schema_errors() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path());
    write(dir.path(), "sources/again.yaml", &source_yaml(""));
    let error = validate(dir.path(), &options()).unwrap_err();
    assert!(error.to_string().contains("重复的 id"), "{}", error);

    write(
        dir.path(),
        "sources/again.yaml",
        &source_yaml("").replace("source/demo", "source/again"),
    );
    write(
        dir.path(),
        "sources/again.yaml",
        r#"
id: source/again
kind: source
name: Again
policy:
  robots:
    mode: ignore
access:
  accessor: accessor/http
  locator:
    url: https://example.com/b
pipeline: []
schedule:
  interval: 1h
admission: {}
"#,
    );
    let error = validate(dir.path(), &options()).unwrap_err();
    assert!(error.to_string().contains("robots_reason"), "{}", error);

    write(
        dir.path(),
        "sources/again.yaml",
        r#"
id: source/again
kind: source
name: Again
access:
  accessor: accessor/http
  locator:
    url: ${env:BLOG}
pipeline: []
schedule:
  interval: 1h
admission: {}
labels:
  note: ${ref:accessor/http#spec.note}
"#,
    );
    write(
        dir.path(),
        "accessors/http.yaml",
        &format!(
            "spec:\n  note: ${{ref:source/again#labels.note}}\n{}",
            accessor_yaml()
        ),
    );
    let error = validate(dir.path(), &options()).unwrap_err();
    assert!(
        error.to_string().contains("成环") || error.to_string().contains("环境变量不存在"),
        "{}",
        error
    );

    let env = LoadOptions {
        env: EnvSource::Fixed([("BLOG".into(), "https://example.com/env".into())].into()),
        ..LoadOptions::default()
    };
    let error = validate(dir.path(), &env).unwrap_err();
    assert!(error.to_string().contains("成环"), "{}", error);

    write(
        dir.path(),
        "sources/again.yaml",
        &source_yaml("").replace("source/demo", "source/again"),
    );
    write(dir.path(), "accessors/http.yaml", accessor_yaml());
    write(
        dir.path(),
        "sources/again.yaml",
        &source_yaml("")
            .replace("source/demo", "source/again")
            .replace("url: https://example.com/a", "url: ${env:BLOG}"),
    );
    let snapshot = validate(dir.path(), &env).unwrap();
    assert_eq!(
        snapshot.modules["source/again"].pointer("/access/locator/url"),
        Some(&json!("https://example.com/env"))
    );

    write(
        dir.path(),
        "sources/again.yaml",
        &source_yaml("policy:\n  raw:\n    store: 1\n").replace("source/demo", "source/again"),
    );
    let error = validate(dir.path(), &options()).unwrap_err();
    assert!(error.to_string().contains("schema"), "{}", error);
}

#[test]
fn watch_debounces_and_keeps_snapshot_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path());
    let store = Arc::new(ConfigStore::open(dir.path(), options()).unwrap());
    let _watch = watch(Arc::clone(&store), Duration::from_millis(200)).unwrap();
    write(
        dir.path(),
        "sources/demo.yaml",
        &source_yaml("policy:\n  raw:\n    retain: 2d\n"),
    );
    write(
        dir.path(),
        "sources/demo.yaml",
        &source_yaml("policy:\n  raw:\n    retain: 4d\n"),
    );
    assert_eq!(store.snapshot().generation, 1);
    assert!(wait_until(Duration::from_secs(2), || store
        .snapshot()
        .generation
        == 2));
    assert_eq!(
        store.snapshot().modules["source/demo"].pointer("/policy/raw/retain"),
        Some(&json!("4d"))
    );
    write(dir.path(), "sources/demo.yaml", &source_yaml("foo: 1\n"));
    assert!(wait_until(Duration::from_secs(2), || {
        store.last_error().is_some()
    }));
    assert_eq!(store.snapshot().generation, 2);
}

fn wait_until(limit: Duration, mut ready: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if ready() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    ready()
}

#[test]
fn secret_stays_a_placeholder_in_snapshot_explain_and_diff() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path());
    write(
        dir.path(),
        "sources/demo.yaml",
        &source_yaml("").replace("name: Demo", "name: ${secret:blog/token}"),
    );
    let options = LoadOptions {
        secrets: [("blog/token".into(), "s3cret-value".into())].into(),
        ..LoadOptions::default()
    };
    let snapshot = validate(dir.path(), &options).unwrap();
    let name = snapshot.modules["source/demo"]
        .get("name")
        .cloned()
        .unwrap();
    assert_eq!(name, json!("${secret:blog/token}"));
    let rendered = serde_json::to_string(&snapshot.modules).unwrap();
    assert!(!rendered.contains("s3cret-value"));
    let fields = explain(&snapshot, "source/demo").unwrap();
    assert!(fields.iter().any(|field| field.value == name));
    assert!(!format!("{fields:?}").contains("s3cret-value"));
    let again = validate(dir.path(), &options).unwrap();
    let changes = diff(&snapshot, &again);
    assert!(changes.is_empty());
}

#[test]
fn effective_policy_includes_system_redact_headers() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path());
    let snapshot = validate(dir.path(), &options()).unwrap();
    let module = &snapshot.modules["source/demo"];
    assert!(module.pointer("/policy/raw/redact_headers").is_none());
    let policy = overlay_system_policy(&snapshot.system, module.get("policy").unwrap());
    assert_eq!(
        policy.pointer("/raw/redact_headers"),
        Some(&json!([
            "authorization",
            "cookie",
            "set-cookie",
            "proxy-authorization"
        ]))
    );
}

#[test]
fn debounce_constant_is_two_seconds() {
    assert_eq!(config::DEFAULT_DEBOUNCE, Duration::from_secs(2));
}

#[test]
fn disabled_module_keeps_an_unset_env_reference() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path());
    write(
        dir.path(),
        "sources/keyed.yaml",
        r#"
id: source/keyed
kind: source
name: Keyed
enabled: false
access:
  accessor: accessor/http
  locator:
    url: https://example.com/keyed
  spec:
    token: ${env:MISSING_PAID_KEY}
pipeline: []
schedule:
  interval: 1h
admission: {}
"#,
    );
    let snapshot = validate(dir.path(), &options()).unwrap();
    assert_eq!(
        snapshot.modules["source/keyed"].pointer("/access/spec/token"),
        Some(&json!("${env:MISSING_PAID_KEY}"))
    );
    write(
        dir.path(),
        "sources/keyed.yaml",
        r#"
id: source/keyed
kind: source
name: Keyed
enabled: true
access:
  accessor: accessor/http
  locator:
    url: https://example.com/keyed
  spec:
    token: ${env:MISSING_PAID_KEY}
pipeline: []
schedule:
  interval: 1h
admission: {}
"#,
    );
    let error = validate(dir.path(), &options()).unwrap_err();
    assert!(error.to_string().contains("环境变量不存在"), "{error}");
}
