//! 公共 API 契约：`/api/v1` 的每个地址都在真实网关上跑通，返回形状与错误码固定。
//!
//! 这里挂的是 `ops::app`——与 `modalprism-collector` 实际对外的是同一个合成应用，
//! 只把监听端口换成临时端口。测的是路由与投影本身，不为测试另写一份处理逻辑。

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use assembly::Binds;
use config::builtin_presets;
use serde_json::{Value, json};
use strata_model::{ApplyOpts, Body, ChangeSet, FacetKind, LineageInput, Op, Producer, Target};

const ARTICLE_URL: &str = "https://example.com/api-test";

/// ULID 全零：格式合法、必然不存在，用来验 404。
const ABSENT_RID: &str = "r_00000000000000000000000000";

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

/// 在本机临时端口上跑公共面，返回基址。
async fn serve(collector: &modalprism_collector::Running) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = modalprism_collector::ops::app(collector.gateway());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

async fn get(http: &reqwest::Client, url: &str) -> Value {
    let response = http.get(url).send().await.unwrap();
    let status = response.status();
    let body: Value = response.json().await.unwrap();
    assert!(status.is_success(), "{url} 应成功：{status} {body}");
    body
}

async fn post(http: &reqwest::Client, url: &str, body: Value) -> reqwest::Response {
    http.post(url).json(&body).send().await.unwrap()
}

async fn failure(response: reqwest::Response) -> (u16, Value) {
    let status = response.status().as_u16();
    let body = response.json().await.unwrap();
    (status, body)
}

/// 发布一篇带 `struct:article` 的文章，返回它的 rid。
async fn publish_article(strata: &strata_client::Strata) -> String {
    strata
        .apply(
            ChangeSet {
                producer: Producer {
                    module: "api-test".into(),
                    version: "1".into(),
                    config_generation: 1,
                },
                idempotency_key: None,
                ack: None,
                ops: vec![
                    Op::Ensure {
                        identity: format!("url:{ARTICLE_URL}"),
                        ns: None,
                        labels: BTreeMap::new(),
                        r#as: "article".into(),
                    },
                    Op::PutFacet {
                        target: Target::Alias("article".into()),
                        kind: FacetKind::Struct,
                        variant: Some("article".into()),
                        body: Body::Struct {
                            value: json!({
                                "title": "示例文章",
                                "url": ARTICLE_URL,
                                "publishedAt": "2026-01-01T00:00:00Z",
                            }),
                        },
                        media_type: Some("application/json".into()),
                        lineage: LineageInput {
                            inputs: Vec::new(),
                            params_hash: "params".into(),
                        },
                    },
                    // 正文进了全文索引，`q` 才有东西可检。默认分词器按空白切词。
                    Op::PutFacet {
                        target: Target::Alias("article".into()),
                        kind: FacetKind::Text,
                        variant: None,
                        body: Body::Text {
                            text: "sample article body".into(),
                        },
                        media_type: Some("text/plain".into()),
                        lineage: LineageInput {
                            inputs: Vec::new(),
                            params_hash: "params".into(),
                        },
                    },
                ],
            },
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    strata.flush().await.unwrap();
    strata
        .resolve(&format!("url:{ARTICLE_URL}"))
        .await
        .unwrap()
        .unwrap()
        .to_string()
}

/// 七个读地址：形状固定、读到的是真数据；两个写地址各有回报。
#[tokio::test]
async fn public_api_serves_the_documented_shapes() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let collector = modalprism_collector::boot(data.path(), config.path(), binds())
        .await
        .unwrap();
    let rid = publish_article(&collector.services().strata()).await;
    let base = serve(&collector).await;
    let http = reqwest::Client::new();

    let health = get(&http, &format!("{base}/api/v1/health")).await;
    assert_eq!(health["status"], "ok");
    assert_eq!(health["service"], "modalprism");
    assert!(
        health["generation"].is_number(),
        "健康面要带配置代际：{health}"
    );

    let page = get(&http, &format!("{base}/api/v1/articles?limit=5")).await;
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "列表只应含刚发布的一篇：{page}");
    assert_eq!(items[0]["rid"], rid.as_str());
    assert_eq!(items[0]["title"], "示例文章");

    let one = get(&http, &format!("{base}/api/v1/articles/{rid}")).await;
    assert_eq!(one["rid"], rid.as_str());
    assert_eq!(one["struct"]["title"], "示例文章");
    assert!(
        one["readable"].is_null() && one["cover"].is_null(),
        "缺 md / 封面时投影给 null，而不是省略字段：{one}"
    );

    let sources = get(&http, &format!("{base}/api/v1/sources")).await;
    for key in ["sources", "accessors", "adapters"] {
        assert!(sources[key].is_array(), "sources 少 {key}：{sources}");
    }

    let workflows = get(&http, &format!("{base}/api/v1/workflows")).await;
    assert!(
        workflows["workflows"].is_array() && workflows["models"].is_array(),
        "工作流目录：{workflows}"
    );

    let runs = get(&http, &format!("{base}/api/v1/runs?limit=3")).await;
    assert!(runs["items"].is_array(), "运行记录：{runs}");

    // 检索与按序号分页互斥，也不能带 hasFacets：命中后按结构面筛成文章。
    let searched = get(&http, &format!("{base}/api/v1/articles?q=sample")).await;
    let hits = searched["items"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "全文检索应命中刚发布的正文：{searched}");
    assert_eq!(hits[0]["rid"], rid.as_str());

    let nothing = get(&http, &format!("{base}/api/v1/articles?q=absentterm")).await;
    assert!(
        nothing["items"].as_array().unwrap().is_empty(),
        "检索不到时给空数组：{nothing}"
    );

    // 单次上限 50：再大也只按上限取，公共面不会被当成导出接口。
    let capped = get(&http, &format!("{base}/api/v1/articles?limit=999")).await;
    assert!(capped["items"].as_array().unwrap().len() <= 50);

    let tick = post(&http, &format!("{base}/api/v1/process"), json!({}))
        .await
        .json::<Value>()
        .await
        .unwrap();
    assert!(
        tick["scanned"].is_number(),
        "加工推进要回报扫描进度：{tick}"
    );

    collector.shutdown().await.unwrap();
}

/// 失败一律是 `{"error": {"code", "message"}}`，配合稳定的状态码。
#[tokio::test]
async fn public_api_maps_failures_to_fixed_shapes() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let collector = modalprism_collector::boot(data.path(), config.path(), binds())
        .await
        .unwrap();
    let rid = publish_article(&collector.services().strata()).await;
    let base = serve(&collector).await;
    let http = reqwest::Client::new();

    let missing = http
        .get(format!("{base}/api/v1/articles/{ABSENT_RID}"))
        .send()
        .await
        .unwrap();
    let (status, body) = failure(missing).await;
    assert_eq!(status, 404);
    assert_eq!(body["error"]["code"], "not_found");
    assert!(body["error"]["message"].is_string());

    let no_rid = post(
        &http,
        &format!("{base}/api/v1/workflows/anything/run"),
        json!({}),
    )
    .await;
    let (status, body) = failure(no_rid).await;
    assert_eq!(status, 400);
    assert_eq!(body["error"]["code"], "invalid_input");

    // 未知工作流：拒绝，而不是悄悄什么都不做。
    let unknown = post(
        &http,
        &format!("{base}/api/v1/workflows/anything/run"),
        json!({ "rid": rid }),
    )
    .await;
    let (status, body) = failure(unknown).await;
    assert_eq!(status, 400);
    assert_eq!(body["error"]["code"], "invalid_input");

    collector.shutdown().await.unwrap();
}
