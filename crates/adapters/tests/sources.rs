//! 每个已声明信源的夹具映射，以及列表与 Feed 的通用字段。

use std::path::PathBuf;

use bytes::Bytes;
use config::LoadOptions;
use serde_json::{Value, json};
use strata_model::{Body, FacetKind, Locator, Op, PartId, PartRole, Target, TaskReason, Timestamp};

use adapters::{
    DetectorChoice, PartBytes, Registry, RunRequest, empty_identities, parse_pipeline, run,
    source_view,
};

fn repo_config() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config")
}

fn producer() -> strata_model::Producer {
    strata_model::Producer {
        module: "runner".into(),
        version: "1".into(),
        config_generation: 1,
    }
}

fn titles(ops: &[Op]) -> Vec<String> {
    ops.iter()
        .filter_map(|op| match op {
            Op::PutFacet {
                kind: FacetKind::Struct,
                variant: Some(variant),
                body: Body::Struct { value },
                ..
            } if variant == "discovery" => value
                .pointer("/hints/title")
                .and_then(Value::as_str)
                .map(str::to_string),
            _ => None,
        })
        .collect()
}

fn discovery_keys(ops: &[Op]) -> Vec<String> {
    ops.iter()
        .filter_map(|op| match op {
            Op::PutFacet {
                kind: FacetKind::Struct,
                variant: Some(variant),
                body: Body::Struct { value },
                ..
            } if variant == "discovery" => {
                value
                    .get("identity")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
            }
            _ => None,
        })
        .flatten()
        .collect()
}

fn has_text(ops: &[Op], needle: &str) -> bool {
    ops.iter().any(|op| match op {
        Op::PutFacet {
            kind: FacetKind::Text,
            body: Body::Text { text },
            target: Target::Alias(_),
            ..
        } => text.contains(needle),
        _ => false,
    })
}

async fn run_bytes(
    module: &Value,
    accessor: &str,
    source_id: &str,
    media: &str,
    body: &str,
    locator: Locator,
) -> adapters::RunOutput {
    let pipeline = module.get("pipeline").cloned().unwrap_or(Value::Null);
    let admission =
        adapters::Admission::from_value(module.get("admission").unwrap_or(&Value::Null)).unwrap();
    let request = RunRequest {
        parts: vec![PartBytes {
            part_id: PartId::new("p1").unwrap(),
            role: PartRole::Document,
            declared: media.into(),
            charset: Some("utf-8".into()),
            bytes: Bytes::from(body.to_string()),
            facet: None,
        }],
        pipeline: parse_pipeline(&pipeline).unwrap(),
        reason: TaskReason::Schedule,
        source: source_view(source_id, accessor).unwrap(),
        resource: None,
        origin: None,
        locator,
        detector: DetectorChoice::Basic,
        admission,
        now: Timestamp::parse("2026-10-20T00:00:00Z").unwrap(),
        initial: true,
        producer: producer(),
        specs: Default::default(),
        identities: empty_identities(),
    };
    run(&Registry::builtins(), request).await.unwrap()
}

fn locator_of(module: &Value) -> Locator {
    let locator = module.pointer("/access/locator").unwrap();
    if let Some(url) = locator.get("url").and_then(Value::as_str) {
        return Locator::Url { url: url.into() };
    }
    if let Some(endpoint) = locator.get("endpoint").and_then(Value::as_str) {
        return Locator::ApiEndpoint {
            endpoint: endpoint.into(),
        };
    }
    if let Some(channel) = locator.get("channel").and_then(Value::as_str) {
        return Locator::PushChannel {
            channel: channel.into(),
        };
    }
    panic!("定位符");
}

fn fixture(id: &str, name: &str) -> (&'static str, String) {
    match id {
        "source/html-listing" => (
            "text/html",
            r#"<div class="news-list"><div class="news-item"><h2><a href="https://example.com/posts/first">第一条示例新闻</a></h2><time datetime="2026-10-01T09:00:00+08:00">10 月 1 日</time></div><div class="news-item"><h2><a href="/news">列表自身</a></h2></div></div>"#.into(),
        ),
        "source/json-listing" => (
            "application/json",
            r#"{"data":{"items":[{"id":"first","title":"第一条示例新闻","summary":"摘要","published_at":"2026-10-01T09:00:00+08:00"}]}}"#.into(),
        ),
        "source/status-search" => (
            "application/json",
            r#"{"tweets":[{"id_str":"100","full_text":"Hello &amp; world","tweet_created_at":"Wed Oct 01 01:00:00 +0000 2026","user":{"screen_name":"example"}}]}"#.into(),
        ),
        "source/wechat-account" => (
            "application/json",
            r#"{"data":[{"title":"公众号标题","url":"https://mp.weixin.qq.com/s/abc","post_time":1759276800,"digest":"摘要"}]}"#.into(),
        ),
        "source/inbound-items" => (
            "application/json",
            r#"{"items":[{"title":"推送标题","url":"https://example.com/pushed","publishedAt":"2026-10-01T00:00:00Z","summary":"推送摘要"}]}"#.into(),
        ),
        _ => (
            "application/rss+xml",
            format!(
                r#"<?xml version="1.0"?><rss version="2.0"><channel><item><title>{}</title><link>https://example.com/{slug}</link><guid>item-{slug}</guid><pubDate>Wed, 01 Oct 2026 00:00:00 +0000</pubDate><description>摘要</description></item></channel></rss>"#,
                name.replace('&', "&amp;"),
                slug = id.trim_start_matches("source/")
            ),
        ),
    }
}

#[tokio::test]
async fn every_declared_source_maps_its_fixture() {
    let snapshot = config::validate(&repo_config(), &LoadOptions::default()).expect("配置");
    let mut seen = 0;
    for (id, module) in &snapshot.modules {
        if module.get("kind").and_then(Value::as_str) != Some("source") {
            continue;
        }
        seen += 1;
        let name = module.get("name").and_then(Value::as_str).unwrap();
        let accessor = module
            .pointer("/access/accessor")
            .and_then(Value::as_str)
            .unwrap();
        let (media, body) = fixture(id, name);
        let output = run_bytes(module, accessor, id, media, &body, locator_of(module)).await;
        let found = titles(&output.changeset.ops);
        assert!(!found.is_empty(), "{id} 没有映射出条目");
        match id.as_str() {
            "source/html-listing" | "source/json-listing" => {
                assert_eq!(found, ["第一条示例新闻"]);
            }
            "source/status-search" => {
                assert_eq!(found, ["Hello & world"]);
                assert!(
                    discovery_keys(&output.changeset.ops)
                        .iter()
                        .any(|key| key == "ext:100")
                );
                assert!(has_text(&output.changeset.ops, "Hello & world"));
                assert!(output.follows.is_empty());
            }
            "source/wechat-account" => assert_eq!(found, ["公众号标题"]),
            "source/inbound-items" => assert_eq!(found, ["推送标题"]),
            _ => assert_eq!(found, [name]),
        }
    }
    assert_eq!(seen, 23);
}

#[tokio::test]
async fn feed_filters_categories_and_can_keep_summary_as_body() {
    let registry = Registry::builtins();
    let feed = r#"<?xml version="1.0"?><rss><channel>
        <item><title>Keep</title><link>https://example.com/keep</link><category>Research</category><description>&lt;p&gt;Body&lt;/p&gt;</description></item>
        <item><title>Drop</title><link>https://example.com/drop</link><category>Ads</category><description>no</description></item>
        <item><title>Video</title><link>https://www.youtube.com/watch?v=abc</link><category>Research</category><description>clip</description></item>
        </channel></rss>"#;
    let pipeline = json!([{
        "run": ["adapter/feed-items"],
        "spec": {"adapter/feed-items": {"allowCategories": ["research"], "denyCategories": ["ads"], "summaryIsBody": true}}
    }]);
    let mut request = bare(feed, "application/rss+xml", pipeline);
    request.locator = Locator::Url {
        url: "https://example.com/feed.xml".into(),
    };
    let output = run(&registry, request).await.unwrap();
    assert_eq!(titles(&output.changeset.ops), ["Keep", "Video"]);
    assert!(has_text(&output.changeset.ops, "Body"));
    assert!(!has_text(&output.changeset.ops, "clip"));
    assert!(output.follows.is_empty());
}

#[tokio::test]
async fn list_modes_read_markdown_and_changelogs() {
    let registry = Registry::builtins();
    let markdown = "# Notes\n\n[第一条长标题](https://example.com/posts/a)\n[Read](https://example.com/posts/a)\n";
    let pipeline = json!([{
        "run": ["adapter/html-list"],
        "spec": {"adapter/html-list": {"parseMode": "markdown", "minTitle": 4}}
    }]);
    let output = run(&registry, bare(markdown, "text/markdown", pipeline))
        .await
        .unwrap();
    assert_eq!(titles(&output.changeset.ops), ["第一条长标题"]);

    let changelog = r#"<article><h2 id="day">2026-10-01</h2><h3 id="ship">发布说明</h3></article>"#;
    let pipeline = json!([{
        "run": ["adapter/html-list"],
        "spec": {"adapter/html-list": {"parseMode": "docusaurus_changelog", "publishedAtUtcOffset": "+08:00"}}
    }]);
    let mut request = bare(changelog, "text/html", pipeline);
    request.locator = Locator::Url {
        url: "https://example.com/changelog".into(),
    };
    let output = run(&registry, request).await.unwrap();
    assert_eq!(titles(&output.changeset.ops), ["发布说明"]);
    let ids = discovery_keys(&output.changeset.ops);
    assert!(ids.iter().any(|key| key.contains("#ship")), "{ids:?}");

    let intercom = r#"<article><h2 id="day">2026-10-01</h2><p><strong>节标题</strong></p><p>本节正文</p></article>"#;
    let pipeline = json!([{
        "run": ["adapter/html-list"],
        "spec": {"adapter/html-list": {"parseMode": "intercom_changelog"}}
    }]);
    let mut request = bare(intercom, "text/html", pipeline);
    request.locator = Locator::Url {
        url: "https://example.com/changes".into(),
    };
    let output = run(&registry, request).await.unwrap();
    assert_eq!(titles(&output.changeset.ops), ["节标题"]);
    assert!(has_text(&output.changeset.ops, "本节正文"));
}

#[tokio::test]
async fn json_list_reads_units_and_templates() {
    let registry = Registry::builtins();
    let body = r#"[{"id":"20261001","name":"数字日期","slug":"a/b"}]"#;
    let pipeline = json!([{
        "run": ["adapter/json-list"],
        "spec": {"adapter/json-list": {
            "itemsPath": "",
            "urlTemplate": "https://example.com/posts/{raw:slug}",
            "titlePaths": ["missing", "name"],
            "publishedAtPath": "id",
            "publishedAtUnit": "yyyymmdd",
            "externalIdPath": "id"
        }}
    }]);
    let output = run(&registry, bare(body, "application/json", pipeline))
        .await
        .unwrap();
    assert_eq!(titles(&output.changeset.ops), ["数字日期"]);
    let keys = discovery_keys(&output.changeset.ops);
    assert!(
        keys.iter()
            .any(|key| key == "url:https://example.com/posts/a/b"),
        "{keys:?}"
    );
    assert!(keys.iter().any(|key| key == "ext:20261001"), "{keys:?}");
}

fn bare(body: &str, media: &str, pipeline: Value) -> RunRequest {
    RunRequest {
        parts: vec![PartBytes {
            part_id: PartId::new("p1").unwrap(),
            role: PartRole::Document,
            declared: media.into(),
            charset: Some("utf-8".into()),
            bytes: Bytes::from(body.to_string()),
            facet: None,
        }],
        pipeline: parse_pipeline(&pipeline).unwrap(),
        reason: TaskReason::Schedule,
        source: source_view("source/demo", "accessor/http").unwrap(),
        resource: None,
        origin: None,
        locator: Locator::Url {
            url: "https://example.com/list".into(),
        },
        detector: DetectorChoice::Basic,
        admission: Default::default(),
        now: Timestamp::parse("2026-10-20T00:00:00Z").unwrap(),
        initial: false,
        producer: producer(),
        specs: Default::default(),
        identities: empty_identities(),
    }
}
