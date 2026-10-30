//! 信源分类的行为测试。
//!
//! 分类是用户自己的管理口径，与闭集的 `sourceType` 正交：创建、批量归属、
//! 计数回显、删除后成员退回未分组，都走公共 API。

use std::fs;
use std::path::Path;
use std::time::Duration;

use ability_support::{InstanceId, MethodId};
use assembly::Binds;
use config::builtin_presets;
use frame_core::InvokeRequest;
use serde_json::{Value, json};

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn config_dir(dir: &Path) {
    for preset in builtin_presets() {
        write(dir, &format!("presets/{}", preset.file_name), preset.source);
    }
    write(
        dir,
        "system.yaml",
        "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\nstrata:\n  gateway:\n    blob_backend: dir\n",
    );
}

fn binds() -> Binds {
    Binds {
        http: "127.0.0.1:0".parse().unwrap(),
        proxy: "127.0.0.1:0".parse().unwrap(),
        push: "127.0.0.1:0".parse().unwrap(),
    }
}

async fn wait_http(base: &str) {
    for _ in 0..100 {
        if reqwest::get(format!("{base}/api/v1/system/status"))
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("领域 API 未就绪");
}

async fn operation(http: &reqwest::Client, base: &str, id: &str) -> Value {
    for _ in 0..200 {
        let value: Value = http
            .get(format!("{base}/api/v1/operations/{id}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if matches!(
            value["status"].as_str().unwrap_or_default(),
            "succeeded" | "partiallySucceeded" | "failed" | "cancelled"
        ) {
            return value;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("操作 {id} 未结束");
}

async fn accepted(http: &reqwest::Client, base: &str, response: reqwest::Response) -> Value {
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 202, "异步入口必须返回 202: {text}");
    let value: Value = serde_json::from_str(&text).unwrap();
    let id = value["operationId"].as_str().unwrap().to_string();
    operation(http, base, &id).await
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

async fn ops_call(
    running: &modalprism_collector::Running,
    instance: &str,
    method: &str,
    args: Value,
) -> Value {
    let request = InvokeRequest::new(
        InstanceId::new(instance).unwrap(),
        MethodId::new(method).unwrap(),
        args,
    );
    running
        .gateway()
        .invoke(request)
        .await
        .map(|response| response.data)
        .unwrap_or_else(|error| panic!("{instance}.{method}: {error}"))
}

/// 用模板建一个访问器和两个信源：测试只关心分类，不关心采集形状。
async fn seed_sources(running: &modalprism_collector::Running) {
    let templates = ops_call(running, "catalog_admin", "templates", json!({})).await;
    let accessor = templates
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["slug"] == json!("accessor-http"))
        })
        .expect("缺少 accessor-http 模板")
        .clone();
    ops_call(
        running,
        "catalog_admin",
        "create",
        json!({"kind": "accessor", "slug": "http2", "doc": accessor["doc"]}),
    )
    .await;
    for slug in ["demo-a", "demo-b"] {
        ops_call(
            running,
            "catalog_admin",
            "create",
            json!({
                "kind": "source",
                "slug": slug,
                "doc": {
                    "name": format!("示例信源 {slug}"),
                    "access": {
                        "accessor": "accessor/http2",
                        "locator": {"url": format!("https://example.com/{slug}")}
                    },
                    "pipeline": [],
                    "schedule": {"interval": "6h"},
                    "admission": {}
                }
            }),
        )
        .await;
    }
}

async fn source_row(http: &reqwest::Client, base: &str, id: &str) -> Value {
    let page: Value = http
        .get(format!("{base}/api/v1/catalog/sources?scope=all"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    page["items"]
        .as_array()
        .and_then(|items| items.iter().find(|row| row["id"] == json!(id)))
        .cloned()
        .unwrap_or_else(|| panic!("缺少信源 {id}"))
}

async fn group_row(http: &reqwest::Client, base: &str, id: &str) -> Value {
    let groups: Value = http
        .get(format!("{base}/api/v1/catalog/sourceGroups"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    groups["items"]
        .as_array()
        .and_then(|items| items.iter().find(|row| row["id"] == json!(id)))
        .cloned()
        .unwrap_or_else(|| panic!("缺少分类 {id}"))
}

#[tokio::test]
async fn factory_reset_keeps_builtin_groups_and_filing() {
    // 复现「首次安装时冻结的旧基线」：旧配置里的内置信源没有分类归属。
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    write(
        config.path(),
        "accessors/rss.yaml",
        "id: accessor/rss\nkind: accessor\nname: rss\nimplementation: builtin:rss\nlimits:\n  timeout: 45s\n  concurrency: 4\n  retry:\n    attempts: 2\n    backoff: exponential\n    base: 1s\n    max: 30s\negress:\n  profile: direct\n",
    );
    write(
        config.path(),
        "sources/google-ai-blog.yaml",
        "id: source/google-ai-blog\nkind: source\nsourceType: feed\nname: Google AI Blog\naccess:\n  accessor: accessor/rss\n  locator:\n    url: https://example.com/feed\npipeline: []\nschedule:\n  interval: 4h\nadmission: {}\n",
    );
    // 自带示例计划：默认关闭，启动不会因它采集。
    write(
        config.path(),
        "plans/example.yaml",
        "id: plan/example\nkind: plan\nname: 示例计划\nenabled: false\nproject: proj_default\ntargets:\n  - source/google-ai-blog\ntrigger:\n  interval: 4h\n  kind: recurring\n",
    );
    let running = modalprism_collector::boot(data.path(), config.path(), binds())
        .await
        .unwrap();
    let base = format!("http://{}", running.http());
    wait_http(&base).await;
    let http = reqwest::Client::new();

    // 启动即补齐：旧配置里内置信源一个归属都没有，按内置清单补上。
    assert_eq!(
        source_row(&http, &base, "google-ai-blog").await["groupId"],
        json!("grp_builtin_ai_labs")
    );
    // 遗留节律只被摘除、不补计划；计划页只剩默认关闭的示例计划，不存在「没有计划却在采集」。
    let plans: Value = http
        .get(format!("{base}/api/v1/collection/plans"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let plan_items = plans["items"].as_array().unwrap();
    assert_eq!(plan_items.len(), 1, "{plans}");
    assert_eq!(plan_items[0]["enabled"], json!(false), "{plans}");
    assert_eq!(plan_items[0]["status"], json!("paused"), "{plans}");
    assert!(
        !fs::read_to_string(config.path().join("sources/google-ai-blog.yaml"))
            .unwrap()
            .contains("schedule:")
    );
    running.shutdown().await.unwrap();

    // 模拟旧版本留下的状态：live 配置去掉 groupId，出厂基线是同一份旧副本且没有版本标记。
    let path = config.path().join("sources/google-ai-blog.yaml");
    let old: String = fs::read_to_string(&path)
        .unwrap()
        .lines()
        .filter(|line| !line.starts_with("groupId:"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&path, &old).unwrap();
    let factory = data.path().join("factory");
    let _ = fs::remove_dir_all(factory.join("config"));
    copy_tree(config.path(), &factory.join("config"));
    let _ = fs::remove_file(factory.join("revision"));

    // 再启动：基线过期被作废并按当前配置重建，内置信源重新补齐归属。
    let running = modalprism_collector::boot(data.path(), config.path(), binds())
        .await
        .unwrap();
    let base = format!("http://{}", running.http());
    wait_http(&base).await;
    let http = reqwest::Client::new();
    assert_eq!(
        source_row(&http, &base, "google-ai-blog").await["groupId"],
        json!("grp_builtin_ai_labs")
    );

    // 恢复出厂：基线已是当前版本，恢复后分类与归属都还在。
    let preview_response = http
        .post(format!("{base}/api/v1/maintenance/factoryResetPreviews"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(preview_response.status(), 201);
    let preview_etag = preview_response.headers()["etag"]
        .to_str()
        .unwrap()
        .to_string();
    let preview: Value = preview_response.json().await.unwrap();
    let response = http
        .post(format!("{base}/api/v1/maintenance/factoryResets"))
        .header("if-match", preview_etag)
        .json(&json!({
            "previewId": preview["id"],
            "confirmationToken": preview["confirmationToken"],
        }))
        .send()
        .await
        .unwrap();
    let finished = accepted(&http, &base, response).await;
    assert_eq!(finished["status"], "succeeded");
    assert_eq!(
        source_row(&http, &base, "google-ai-blog").await["groupId"],
        json!("grp_builtin_ai_labs")
    );
    let groups: Value = http
        .get(format!("{base}/api/v1/catalog/sourceGroups"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(groups["items"].as_array().unwrap().len(), 7, "{groups}");
    // 恢复出厂后计划仍在且默认关闭：不会无计划采集，也不会无提示启用。
    let plans: Value = http
        .get(format!("{base}/api/v1/collection/plans"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let plan_items = plans["items"].as_array().unwrap();
    assert_eq!(plan_items.len(), 1, "{plans}");
    assert_eq!(plan_items[0]["enabled"], json!(false), "{plans}");
    assert!(data.path().join("factory/revision").is_file());

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn source_groups_scope_sources_without_touching_type() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let running = modalprism_collector::boot(data.path(), config.path(), binds())
        .await
        .unwrap();
    let base = format!("http://{}", running.http());
    wait_http(&base).await;
    seed_sources(&running).await;
    let http = reqwest::Client::new();

    // 内置分类随启动种子化：七个领域分类固定在前，且不可删除。
    let groups: Value = http
        .get(format!("{base}/api/v1/catalog/sourceGroups"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let items = groups["items"].as_array().unwrap();
    assert_eq!(items.len(), 7, "{groups}");
    assert!(items.iter().all(|row| row["builtin"] == json!(true)));
    assert_eq!(items[0]["id"], json!("grp_builtin_ai_labs"));
    assert_eq!(items[6]["id"], json!("grp_builtin_access"));
    let blocked = http
        .delete(format!(
            "{base}/api/v1/catalog/sourceGroups/grp_builtin_ai_labs"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), 409);

    // 建档：分类只是名字与描述，不参与信源类型判定。
    let response = http
        .post(format!("{base}/api/v1/catalog/sourceGroups"))
        .json(&json!({"name": "AI 跟踪", "description": "长期观察的模型厂商"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let group: Value = response.json().await.unwrap();
    let group_id = group["id"].as_str().unwrap().to_string();
    assert!(group_id.starts_with("grp_"), "{group_id}");

    // 同项目内重名直接拒绝，避免导航栏出现两个同名分类。
    let duplicate = http
        .post(format!("{base}/api/v1/catalog/sourceGroups"))
        .json(&json!({"name": "AI 跟踪"}))
        .send()
        .await
        .unwrap();
    assert_eq!(duplicate.status(), 409);

    // 批量归属：成功项落盘，不存在的信源逐条跳过。
    let response = http
        .post(format!("{base}/api/v1/catalog/sourceGroups/bindings"))
        .json(&json!({"groupId": group_id, "sourceIds": ["demo-a", "demo-b", "ghost"]}))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 200, "{text}");
    let assigned: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(assigned["updated"], json!(["demo-a", "demo-b"]));
    assert_eq!(assigned["skipped"][0]["id"], json!("ghost"));

    // 信源行回显分类，分类列表带成员计数。
    assert_eq!(
        source_row(&http, &base, "demo-a").await["groupId"],
        json!(group_id)
    );
    assert_eq!(group_row(&http, &base, &group_id).await["count"], json!(2));

    // 重命名保留成员关系。
    let renamed: Value = http
        .patch(format!("{base}/api/v1/catalog/sourceGroups/{group_id}"))
        .json(&json!({"name": "AI 观察"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(renamed["name"], json!("AI 观察"));
    assert_eq!(
        source_row(&http, &base, "demo-b").await["groupId"],
        json!(group_id)
    );

    // 空选择不写盘，直接给出可读的校验错误。
    let empty = http
        .post(format!("{base}/api/v1/catalog/sourceGroups/bindings"))
        .json(&json!({"groupId": group_id, "sourceIds": []}))
        .send()
        .await
        .unwrap();
    assert_eq!(empty.status(), 400);

    // 删除分类：成员退回未分组，信源本身保留。
    let deleted = http
        .delete(format!("{base}/api/v1/catalog/sourceGroups/{group_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), 204);
    assert!(source_row(&http, &base, "demo-a").await["groupId"].is_null());
    let groups: Value = http
        .get(format!("{base}/api/v1/catalog/sourceGroups"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(groups["items"].as_array().unwrap().len(), 7);

    // 内置分类同样可收成员：移入后行回显与计数都成立。
    let assigned: Value = http
        .post(format!("{base}/api/v1/catalog/sourceGroups/bindings"))
        .json(&json!({"groupId": "grp_builtin_ai_media", "sourceIds": ["demo-a"]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(assigned["updated"], json!(["demo-a"]));
    assert_eq!(
        source_row(&http, &base, "demo-a").await["groupId"],
        json!("grp_builtin_ai_media")
    );
    assert_eq!(
        group_row(&http, &base, "grp_builtin_ai_media").await["count"],
        json!(1)
    );
}
