//! prompt 经 SkillExt 注入 skill，不引用对方实现类型。

use ability_core::AbilityHost;
use ability_support::{InstanceId, LoadSpec, MethodId, SourceId};
use serde_json::json;

#[tokio::test]
async fn build_without_skill() {
    let host = AbilityHost::new();
    host.install_str("prompt", ai_prompt::provider())
        .await
        .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("prompt").unwrap(),
        InstanceId::new("prompt").unwrap(),
    ))
    .await
    .unwrap();

    let resp = host
        .into_gateway()
        .invoke(frame_core::InvokeRequest::new(
            InstanceId::new("prompt").unwrap(),
            MethodId::new("build").unwrap(),
            json!({ "system": "你是助手", "user": "hello" }),
        ))
        .await
        .unwrap();
    let msgs = resp.data.as_array().unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[1]["role"], "user");
}

#[tokio::test]
async fn build_injects_skill_via_client() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("demo");
    std::fs::create_dir(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: demo\ndescription: desc\n---\n\ndo it carefully\n",
    )
    .unwrap();

    let host = AbilityHost::new();
    host.install_str("skill", ai_skill::provider())
        .await
        .unwrap();
    host.install_str("prompt", ai_prompt::provider())
        .await
        .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("skill").unwrap(),
        InstanceId::new("skill").unwrap(),
    ))
    .await
    .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("prompt").unwrap(),
        InstanceId::new("prompt").unwrap(),
    ))
    .await
    .unwrap();

    let path = skill_dir.to_string_lossy().into_owned();
    let resp = host
        .into_gateway()
        .invoke(frame_core::InvokeRequest::new(
            InstanceId::new("prompt").unwrap(),
            MethodId::new("build").unwrap(),
            json!({ "skill_path": path, "user": "go" }),
        ))
        .await
        .unwrap();
    let msgs = resp.data.as_array().unwrap();
    assert_eq!(msgs.len(), 2);
    let system = msgs[0]["content"].as_str().unwrap_or_default();
    assert!(system.contains("demo"), "{system}");
    assert!(system.contains("do it carefully"), "{system}");
    assert_eq!(msgs[1]["role"], "user");
}
