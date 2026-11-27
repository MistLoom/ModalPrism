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
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../config")
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
        resource_alias: None,
        origin: None,
        locator,
        detector: DetectorChoice::Basic,
        admission,
        now: Timestamp::parse("2026-10-20T00:00:00Z").unwrap(),
        initial: true,
        producer: producer(),
        specs: Default::default(),
        identities: empty_identities(),
        prism: None,
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
        "source/deepseek-updates" => (
            "text/html",
            format!(
                r#"<article><h2 id="day">时间: 2026-10-17</h2><h3 id="ship">{name}</h3></article>"#
            ),
        ),
        "source/mimo-blog" => (
            "application/json",
            format!(
                r#"{{"items":[{{"url":"https://mimo.xiaomi.com/blog/post","title":"{name}","summary":"摘要","publishedAt":"2026-09-27"}}]}}"#
            ),
        ),
        "source/crates-io-index" => (
            "application/json",
            format!(
                r#"{{"crates":[{{"name":"{name}","repository":"https://github.com/example/{slug}"}}]}}"#,
                slug = id.trim_start_matches("source/").replace('-', "_")
            ),
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
            "source/status-search" => {
                assert_eq!(found, ["Hello & world"]);
                assert!(
                    discovery_keys(&output.changeset.ops)
                        .iter()
                        .any(|key| key == "ext:id:100")
                );
                assert!(has_text(&output.changeset.ops, "Hello & world"));
                assert!(output.follows.is_empty());
            }
            "source/wechat-account" => assert_eq!(found, ["公众号标题"]),
            "source/inbound-items" => assert_eq!(found, ["推送标题"]),
            _ => assert_eq!(found, [name]),
        }
    }
    // 声明数随信源清单变动：拿程序侧的内置信源清单对齐，添信源时不会漏改这里。
    assert_eq!(
        seen,
        config::source_type::BUILTIN_SOURCES.len(),
        "跑出条目的信源数应与内置信源清单一致"
    );
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
async fn feed_decodes_a_second_layer_of_entities() {
    let registry = Registry::builtins();
    let feed = r#"<?xml version="1.0"?><rss><channel><item>
        <title>Army&amp;#039;s unit</title>
        <link>https://example.com/army</link>
        <description>Army&amp;#039;s note</description>
        </item></channel></rss>"#;
    let pipeline = json!([{ "run": ["adapter/feed-items"] }]);
    let output = run(&registry, bare(feed, "application/rss+xml", pipeline))
        .await
        .unwrap();
    assert_eq!(titles(&output.changeset.ops), ["Army's unit"]);
    let summary = output.changeset.ops.iter().find_map(|op| match op {
        Op::PutFacet {
            kind: FacetKind::Struct,
            body: Body::Struct { value },
            ..
        } => value
            .pointer("/hints/summary")
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    });
    assert_eq!(summary.as_deref(), Some("Army's note"));
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
    assert!(keys.iter().any(|key| key == "ext:id:20261001"), "{keys:?}");
}

#[tokio::test]
async fn json_list_reads_sparql_bindings() {
    let registry = Registry::builtins();
    let body = r#"{"results":{"bindings":[{"item":{"value":"http://www.wikidata.org/entity/Q1"},"itemLabel":{"value":"AI winter 1974–1980"},"qid":{"value":"Q1"},"start":{"value":"1974-01-01T00:00:00Z"}}]}}"#;
    let pipeline = json!([{
        "run": ["adapter/json-list"],
        "spec": {"adapter/json-list": {
            "items": "/results/bindings",
            "url": "item.value",
            "title": "itemLabel.value",
            "publishedAt": "start.value",
            "externalIdPath": "qid.value"
        }}
    }]);
    let output = run(
        &registry,
        bare(body, "application/sparql-results+json", pipeline),
    )
    .await
    .unwrap();
    assert_eq!(titles(&output.changeset.ops), ["AI winter 1974–1980"]);
    let keys = discovery_keys(&output.changeset.ops);
    assert!(
        keys.iter().any(|key| key == "ext:id:Q1"),
        "{keys:?}"
    );
}

#[tokio::test]
async fn wikitable_keeps_rowspan_year_and_longer_link() {
    let registry = Registry::builtins();
    let html = r#"<table class="wikitable"><tr><th>Date</th><th>Development</th></tr>
<tr><th rowspan="2">1956</th><td><a href="/wiki/John_McCarthy">John McCarthy</a> convened the <a href="/wiki/Dartmouth_workshop">Dartmouth workshop</a> to study artificial intelligence.</td></tr>
<tr><td><a href="/wiki/Logic_Theorist">Logic Theorist</a> was presented as a reasoning program.</td></tr>
</table>"#;
    let pipeline = json!([{
        "run": ["adapter/html-list"],
        "spec": {"adapter/html-list": {"parseMode": "wikitable"}}
    }]);
    let mut request = bare(html, "text/html", pipeline);
    request.locator = Locator::Url {
        url: "https://en.wikipedia.org/wiki/Timeline_of_artificial_intelligence".into(),
    };
    let output = run(&registry, request).await.unwrap();
    assert_eq!(
        titles(&output.changeset.ops),
        ["Dartmouth workshop", "Logic Theorist"]
    );
}

#[tokio::test]
async fn json_list_copies_extra_paths_and_literals() {
    let registry = Registry::builtins();
    let body = r#"[{"id":"Q1","name":"达特茅斯"}]"#;
    let pipeline = json!([{
        "run": ["adapter/json-list"],
        "spec": {"adapter/json-list": {
            "itemsPath": "",
            "urlTemplate": "https://www.wikidata.org/wiki/{raw:id}",
            "title": "name",
            "extra": {"official_id": "id", "type_key": "const:event"}
        }}
    }]);
    let output = run(&registry, bare(body, "application/json", pipeline))
        .await
        .unwrap();
    let extra = output.changeset.ops.iter().find_map(|op| match op {
        Op::PutFacet {
            kind: FacetKind::Struct,
            body: Body::Struct { value },
            ..
        } => value.pointer("/hints/extra").and_then(Value::as_object),
        _ => None,
    });
    let extra = extra.expect("extra");
    assert_eq!(extra.get("official_id").and_then(Value::as_str), Some("Q1"));
    assert_eq!(extra.get("type_key").and_then(Value::as_str), Some("event"));
}

#[tokio::test]
async fn json_list_skips_absent_paths_and_non_scalars() {
    let registry = Registry::builtins();
    let body = r#"[{"id":"1","name":"条目","vendor":"acme","score":42,"meta":{"nested":"x"}}]"#;
    let pipeline = json!([{
        "run": ["adapter/json-list"],
        "spec": {"adapter/json-list": {
            "itemsPath": "",
            "urlTemplate": "https://example.com/{raw:id}",
            "title": "name",
            "extra": {"owner": "vendor", "absent": "score.missing", "nested": "meta"}
        }}
    }]);
    let output = run(&registry, bare(body, "application/json", pipeline))
        .await
        .unwrap();
    let extra = output
        .changeset
        .ops
        .iter()
        .find_map(|op| match op {
            Op::PutFacet {
                kind: FacetKind::Struct,
                body: Body::Struct { value },
                ..
            } => value
                .pointer("/hints/extra")
                .and_then(Value::as_object)
                .cloned(),
            _ => None,
        })
        .expect("extra");
    // 只写入能取到标量的键：路径不存在与非标量子树都不投影。
    assert_eq!(extra.len(), 1, "{extra:?}");
    assert_eq!(extra["owner"], json!("acme"));
}

#[tokio::test]
async fn json_list_without_extra_keeps_the_discovery_shape() {
    let registry = Registry::builtins();
    let body = r#"[{"id":"1","name":"条目"}]"#;
    let pipeline = json!([{
        "run": ["adapter/json-list"],
        "spec": {"adapter/json-list": {
            "itemsPath": "",
            "urlTemplate": "https://example.com/{raw:id}",
            "title": "name"
        }}
    }]);
    let output = run(&registry, bare(body, "application/json", pipeline))
        .await
        .unwrap();
    // 未声明 extra 时不产生该键，既有载荷字节保持稳定。
    let has_extra = output.changeset.ops.iter().any(|op| match op {
        Op::PutFacet {
            kind: FacetKind::Struct,
            body: Body::Struct { value },
            ..
        } => value.pointer("/hints/extra").is_some(),
        _ => false,
    });
    assert!(!has_extra);
}

#[tokio::test]
async fn table_records_splits_each_row_into_a_discovery() {
    let registry = Registry::builtins();
    let body = r#"<table id="records">
      <thead><tr><th>Name</th><th>Value</th><th>Unit</th><th>When</th><th>Link</th></tr></thead>
      <tbody>
        <tr><td>Alpha</td><td>1.5</td><td>unit</td><td>2026-01-02T03:04:05Z</td><td><a href="/a">src</a></td></tr>
        <tr><td>Beta</td><td>2</td><td>unit</td><td>2026-01-03T00:00:00Z</td><td><a href="https://example.com/b">src</a></td></tr>
      </tbody>
    </table>"#;
    let pipeline = json!([{
        "run": ["adapter/table-records"],
        "spec": {"adapter/table-records": {
            "table": "table#records",
            "columns": {
                "name": "Name",
                "amount": "Value",
                "unit": "Unit",
                "observed_at": "When",
                "source_url": "Link"
            },
            "link": "source_url",
            "title": "name",
            "publishedAt": "observed_at"
        }}
    }]);
    let output = run(&registry, bare(body, "text/html", pipeline))
        .await
        .unwrap();
    assert_eq!(titles(&output.changeset.ops), ["Alpha", "Beta"]);
    let hints: Vec<Value> = output
        .changeset
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::PutFacet {
                kind: FacetKind::Struct,
                body: Body::Struct { value },
                ..
            } => value.get("hints").cloned(),
            _ => None,
        })
        .collect();
    assert_eq!(hints.len(), 2, "{hints:?}");
    assert_eq!(hints[0]["extra"]["amount"], json!("1.5"));
    assert_eq!(hints[0]["extra"]["unit"], json!("unit"));
    assert_eq!(
        hints[0]["extra"]["observed_at"],
        json!("2026-01-02T03:04:05Z")
    );
    assert_eq!(
        hints[0]["extra"]["source_url"],
        json!("https://example.com/a")
    );
    assert_eq!(
        hints[0].get("publishedAt").and_then(Value::as_str),
        Some("2026-01-02T03:04:05Z")
    );
    let keys = discovery_keys(&output.changeset.ops);
    assert!(
        keys.iter().any(|key| key == "url:https://example.com/a"),
        "{keys:?}"
    );
    assert!(
        keys.iter().any(|key| key == "url:https://example.com/b"),
        "{keys:?}"
    );
}

#[tokio::test]
async fn json_list_truncates_long_extra_values() {
    let registry = Registry::builtins();
    let long = "x".repeat(900);
    let body = format!(r#"[{{"id":"Q1","name":"条目","note":"{long}"}}]"#);
    let pipeline = json!([{
        "run": ["adapter/json-list"],
        "spec": {"adapter/json-list": {
            "itemsPath": "",
            "urlTemplate": "https://example.com/{raw:id}",
            "title": "name",
            "extra": {"note": "note"}
        }}
    }]);
    let output = run(&registry, bare(&body, "application/json", pipeline))
        .await
        .unwrap();
    let note = output
        .changeset
        .ops
        .iter()
        .find_map(|op| match op {
            Op::PutFacet {
                kind: FacetKind::Struct,
                body: Body::Struct { value },
                ..
            } => value.pointer("/hints/extra/note").and_then(Value::as_str),
            _ => None,
        })
        .expect("note");
    assert_eq!(note.chars().count(), 512);
}

#[tokio::test]
async fn json_list_rejects_reserved_or_malformed_extra() {
    let registry = Registry::builtins();
    let reserved = json!([{
        "run": ["adapter/json-list"],
        "spec": {"adapter/json-list": {
            "itemsPath": "",
            "urlTemplate": "https://example.com/{raw:id}",
            "title": "name",
            "extra": {"title": "t"}
        }}
    }]);
    // title 已有固定槽位，再经 extra 重复会让下游无从取值。
    assert!(run(&registry, bare("[]", "application/json", reserved))
        .await
        .is_err());
    let blank = json!([{
        "run": ["adapter/json-list"],
        "spec": {"adapter/json-list": {
            "itemsPath": "",
            "urlTemplate": "https://example.com/{raw:id}",
            "title": "name",
            "extra": {"note": "  "}
        }}
    }]);
    assert!(run(&registry, bare("[]", "application/json", blank))
        .await
        .is_err());
}

#[tokio::test]
async fn changelog_reads_a_fullwidth_date_heading() {
    let registry = Registry::builtins();
    let html = r#"<article><h2 id="day">时间：2026-10-17</h2><h3 id="ship">发布</h3></article>"#;
    let pipeline = json!([{
        "run": ["adapter/html-list"],
        "spec": {"adapter/html-list": {"parseMode": "docusaurus_changelog"}}
    }]);
    let output = run(&registry, bare(html, "text/html", pipeline))
        .await
        .unwrap();
    assert_eq!(titles(&output.changeset.ops), ["发布"]);
}

#[tokio::test]
async fn html_list_reads_a_configured_date_pattern() {
    let registry = Registry::builtins();
    let html = r#"<article class="card"><a href="https://example.com/posts/a">标题</a><span>发布于 2026-09-27</span></article>"#;
    let pipeline = json!([{
        "run": ["adapter/html-list"],
        "spec": {"adapter/html-list": {
            "item": "article.card",
            "link": "a",
            "title": "a",
            "publishedAtRegex": "发布于 (\\d{4}-\\d{2}-\\d{2})",
            "detail": {"maxFetches": 2, "fields": ["publishedAt"]}
        }}
    }]);
    let output = run(&registry, bare(html, "text/html", pipeline))
        .await
        .unwrap();
    assert_eq!(titles(&output.changeset.ops), ["标题"]);
    assert!(output.follows.is_empty());
    let stamp = output.changeset.ops.iter().find_map(|op| match op {
        Op::PutFacet {
            kind: FacetKind::Struct,
            variant: Some(variant),
            body: Body::Struct { value },
            ..
        } if variant == "discovery" => value
            .pointer("/hints/publishedAt")
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    });
    assert_eq!(stamp.as_deref(), Some("2026-09-27T00:00:00Z"));
}

#[tokio::test]
async fn missing_detail_field_requests_one_follow() {
    let registry = Registry::builtins();
    let body = r#"{"items":[{"url":"https://example.com/a","title":"甲"},{"url":"https://example.com/b","title":"乙","publishedAt":"2026-10-01T00:00:00Z"}]}"#;
    let pipeline = json!([{
        "run": ["adapter/json-list"],
        "spec": {"adapter/json-list": {
            "items": "/items",
            "url": "url",
            "title": "title",
            "publishedAt": "publishedAt",
            "detail": {"maxFetches": 4, "fields": ["publishedAt"]}
        }}
    }]);
    let output = run(&registry, bare(body, "application/json", pipeline))
        .await
        .unwrap();
    assert_eq!(output.follows.len(), 1);
    assert_eq!(output.follows[0].accessor, "accessor/http");
    assert_eq!(output.follows[0].needs, ["publishedAt"]);
}

#[tokio::test]
async fn page_fields_reads_json_ld_published_date() {
    let registry = Registry::builtins();
    let html = r#"<script type="application/ld+json">{"@type":"BlogPosting","datePublished":"2026-09-27T08:00:00+08:00"}</script>"#;
    let pipeline = json!([{
        "for": "follow",
        "run": ["adapter/page-fields"],
        "spec": {"adapter/page-fields": {
            "publishedAt": "time.missing",
            "publishedAtUtcOffset": "+08:00"
        }}
    }]);
    let mut request = bare(html, "text/html", pipeline);
    request.reason = TaskReason::Follow;
    request.resource = Some(strata_model::Rid::parse("r_01JAF3K8Z2X7Q9M4T6V0B5N1CE").unwrap());
    let output = run(&registry, request).await.unwrap();
    let page = output.changeset.ops.iter().find_map(|op| match op {
        Op::PutFacet {
            kind: FacetKind::Struct,
            variant: Some(variant),
            body: Body::Struct { value },
            ..
        } if variant == "page" => Some(value),
        _ => None,
    });
    assert_eq!(
        page.unwrap().get("publishedAt").and_then(Value::as_str),
        Some("2026-09-27T08:00:00+08:00")
    );
}

#[tokio::test]
async fn mimo_shell_detail_does_not_emit_struct_page() {
    let registry = Registry::builtins();
    let html = r#"<!doctype html><html><head><script defer src="/static/js/index.js"></script></head><body><div id="root"></div></body></html>"#;
    let pipeline = json!([{
        "for": "follow",
        "run": ["adapter/page-fields"],
        "spec": {"adapter/page-fields": {
            "publishedAt": "time.hero-date",
            "publishedAtAttr": "datetime",
            "title": "h1.hero-title",
            "publishedAtUtcOffset": "+08:00"
        }}
    }]);
    let mut request = bare(html, "text/html", pipeline);
    request.reason = TaskReason::Follow;
    request.resource = Some(strata_model::Rid::parse("r_01JAF3K8Z2X7Q9M4T6V0B5N1CE").unwrap());
    let output = run(&registry, request).await.unwrap();
    assert!(
        !output
            .changeset
            .ops
            .iter()
            .any(|op| matches!(op, Op::PutFacet { variant: Some(v), .. } if v == "page"))
    );
}

#[tokio::test]
async fn page_fields_reads_the_detail_date() {
    let registry = Registry::builtins();
    let html = r#"<time class="hero-date" datetime="2026-09-27">September 27, 2026</time><h1 class="hero-title">文章标题</h1><p class="hero-subtitle">一句摘要</p>"#;
    let pipeline = json!([{
        "for": "follow",
        "run": ["adapter/page-fields"],
        "spec": {"adapter/page-fields": {
            "publishedAt": "time.hero-date",
            "publishedAtAttr": "datetime",
            "title": "h1.hero-title",
            "summary": "p.hero-subtitle"
        }}
    }]);
    let mut request = bare(html, "text/html", pipeline);
    request.reason = TaskReason::Follow;
    request.resource = Some(strata_model::Rid::parse("r_01JAF3K8Z2X7Q9M4T6V0B5N1CE").unwrap());
    let output = run(&registry, request).await.unwrap();
    let page = output.changeset.ops.iter().find_map(|op| match op {
        Op::PutFacet {
            kind: FacetKind::Struct,
            variant: Some(variant),
            body: Body::Struct { value },
            ..
        } if variant == "page" => Some(value),
        _ => None,
    });
    let page = page.unwrap();
    assert_eq!(
        page.get("publishedAt").and_then(Value::as_str),
        Some("2026-09-27T00:00:00Z")
    );
    assert_eq!(page.get("title").and_then(Value::as_str), Some("文章标题"));
    assert_eq!(
        page.get("summary").and_then(Value::as_str),
        Some("一句摘要")
    );
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
        resource_alias: None,
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
        prism: None,
    }
}
