//! 对照 guest-ctx-api.json 校验 TS / Python Guest SDK 表面 API。

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Deserialize)]
struct GuestCtxApiSpec {
    methods: Vec<MethodEntry>,
    forbidden_on_guest: Vec<String>,
}

#[derive(Deserialize)]
struct MethodEntry {
    method: String,
    python: Option<String>,
    internal: Option<bool>,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

#[test]
fn guest_sdk_covers_ctx_api_spec() {
    let spec_path = repo_root().join("frame/support/spec/guest-ctx-api.json");
    let spec: GuestCtxApiSpec =
        serde_json::from_str(&std::fs::read_to_string(&spec_path).expect("read spec"))
            .expect("parse");

    let ts_guest =
        std::fs::read_to_string(repo_root().join("SDK/ability-sdk-ts/lib/guest.js"))
            .expect("read guest.js");
    let py_guest = std::fs::read_to_string(
        repo_root().join("SDK/ability-sdk-py/src/leaf_ability_sdk/guest.py"),
    )
    .expect("read guest.py");

    for entry in &spec.methods {
        if entry.internal == Some(true) {
            continue;
        }
        let py_name = entry.python.as_deref().unwrap_or(&entry.method);
        assert!(
            ts_guest.contains(&format!("{}(", entry.method))
                || ts_guest.contains(&format!("async {}(", entry.method)),
            "TS guest 缺少方法 {}",
            entry.method
        );
        assert!(
            py_guest.contains(&format!("def {py_name}(")),
            "Python guest 缺少方法 {py_name}"
        );
    }

    for forbidden in &spec.forbidden_on_guest {
        let snake = if forbidden.contains('_') {
            forbidden.clone()
        } else {
            let mut s = String::new();
            for (i, c) in forbidden.chars().enumerate() {
                if c.is_uppercase() && i > 0 {
                    s.push('_');
                }
                s.push(c.to_ascii_lowercase());
            }
            s
        };
        assert!(
            !ts_guest.contains(&format!("op: \"{snake}\""))
                && !py_guest.contains(&format!("\"op\": \"{snake}\"")),
            "Guest 不得转发 CtxOp {snake}"
        );
    }
}
