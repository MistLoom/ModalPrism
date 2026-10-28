//! 会话门面联调：CRUD、批量、标签、迭代器、事件与渲染投影。

use ai_protocol::Message;
use serde_json::json;

use crate::ext::ExtRef;
use crate::hub::{HubConfig, SessionHub};
use crate::model::{ContextPolicy, ItemDraft, ItemKind, ItemPatch, TagOp, TagSet};
use crate::query::{ItemQuery, TagFilter};
use crate::render::{RenderBody, RenderPolicy, RenderViewSpec};
use crate::snapshot::ImportMode;

#[tokio::test]
async fn sugar_crud_and_protocol_projection() {
    let hub = SessionHub::memory();
    let session = hub
        .create()
        .title("调研")
        .tags(["kb", "draft"])
        .open()
        .await
        .unwrap();

    session.append_user("hello").await.unwrap();
    session.append_assistant("hi").await.unwrap();
    session.append(ItemDraft::notice("内部备注")).await.unwrap();
    session
        .append(ItemDraft::artifact("图", "text/plain", "body"))
        .await
        .unwrap();

    let conv = session.get().await.unwrap();
    assert_eq!(conv.title.as_deref(), Some("调研"));
    assert!(conv.tags.contains("kb"));
    assert_eq!(conv.item_count, 4);

    let protocol = session.protocol_messages().await.unwrap();
    assert_eq!(protocol.len(), 2);
    assert_eq!(protocol[0], Message::user("hello"));
}

#[tokio::test]
async fn batch_read_delete_and_tag_filter() {
    let hub = SessionHub::memory();
    let a = hub.create().tags(["kb"]).open().await.unwrap();
    let b = hub.create().tags(["kb", "urgent"]).open().await.unwrap();
    let c = hub.create().tags(["misc"]).open().await.unwrap();
    a.append_user("a").await.unwrap();
    b.append_user("b").await.unwrap();

    let listed = hub
        .select()
        .tagged(TagFilter::all(["kb"]))
        .list()
        .await
        .unwrap();
    assert_eq!(listed.len(), 2);

    let any_urgent = hub
        .select()
        .tagged(TagFilter::any(["urgent"]))
        .count()
        .await
        .unwrap();
    assert_eq!(any_urgent, 1);

    let bundles = hub
        .select()
        .tagged(TagFilter::all(["kb"]))
        .read(ItemQuery::all())
        .await
        .unwrap();
    assert_eq!(bundles.len(), 2);

    let deleted = hub.select().ids([c.id().clone()]).delete().await.unwrap();
    assert_eq!(deleted, 1);

    let tagged = hub
        .select()
        .tagged(TagFilter::all(["kb"]))
        .tag(TagOp::Add(vec!["archived".into()]))
        .await
        .unwrap();
    assert_eq!(tagged, 2);
    assert!(a.get().await.unwrap().tags.contains("archived"));
}

#[tokio::test]
async fn batch_requires_filter_or_all() {
    let hub = SessionHub::memory();
    hub.create().open().await.unwrap();
    let err = hub.select().delete().await.unwrap_err();
    assert!(err.to_string().contains("all()"));
    let n = hub.select().all().delete().await.unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn tag_catalog_rename_and_item_filter() {
    let hub = SessionHub::memory();
    let s = hub.create().tags(["old"]).open().await.unwrap();
    s.append(ItemDraft::user("x").with_tags(TagSet::from_list(["tool"]).unwrap()))
        .await
        .unwrap();
    s.append_user("y").await.unwrap();

    let n = hub.tags().rename("old", "new").await.unwrap();
    assert_eq!(n, 1);
    assert!(s.get().await.unwrap().tags.contains("new"));
    assert!(!s.get().await.unwrap().tags.contains("old"));

    let tools = s
        .items()
        .tagged(TagFilter::all(["tool"]))
        .collect()
        .await
        .unwrap();
    assert_eq!(tools.len(), 1);

    let users = s
        .items()
        .kinds(vec![ItemKind::User])
        .collect()
        .await
        .unwrap();
    assert_eq!(users.len(), 2);
}

#[tokio::test]
async fn iterator_and_fork() {
    let hub = SessionHub::memory();
    for i in 0..4 {
        hub.create()
            .title(format!("c{i}"))
            .tags(["x"])
            .open()
            .await
            .unwrap();
    }
    let all = hub
        .select()
        .tagged(TagFilter::all(["x"]))
        .iter()
        .take_n(2)
        .collect()
        .await
        .unwrap();
    assert_eq!(all.len(), 2);

    let src = hub.create().title("src").open().await.unwrap();
    src.append_user("1").await.unwrap();
    src.append_assistant("2").await.unwrap();
    let forked = src.fork().await.unwrap();
    let child = forked.get().await.unwrap();
    assert_eq!(child.parent_id.as_ref(), Some(src.id()));
    assert_eq!(child.item_count, 2);
}

#[tokio::test]
async fn render_cache_and_deferred_body() {
    let hub = SessionHub::memory_with(HubConfig {
        render: RenderPolicy {
            excerpt_chars: 8,
            inline_chars: 16,
        },
        cache_capacity: 8,
    });
    let s = hub.create().title("r").open().await.unwrap();
    let long = "abcdefghijklmnopqrstuvwxyz";
    s.append_user(long).await.unwrap();
    s.append_assistant("ok").await.unwrap();

    let spec = RenderViewSpec::default_latest(10);
    let snap = s.render(&spec).await.unwrap();
    assert_eq!(snap.nodes.len(), 2);
    assert_eq!(snap.nodes[0].kind, ItemKind::User);
    assert!(matches!(snap.nodes[0].body, RenderBody::Deferred { .. }));
    assert!(matches!(snap.nodes[1].body, RenderBody::Inline { .. }));

    let body = hub
        .render()
        .load_body(s.id(), &snap.nodes[0].id)
        .await
        .unwrap();
    assert_eq!(body, long);

    let again = s.render(&spec).await.unwrap();
    assert_eq!(again.revision, snap.revision);

    s.append_user("more").await.unwrap();
    let after = s.render(&spec).await.unwrap();
    assert!(after.revision > snap.revision);
    assert_eq!(after.nodes.len(), 3);
}

#[tokio::test]
async fn run_mutex_checkpoint_and_group() {
    let hub = SessionHub::memory();
    let s = hub.create().open().await.unwrap();
    let run = s.begin_run().group("g1").open().await.unwrap();
    let err = match s.begin_run().open().await {
        Ok(_) => panic!("expected conflict"),
        Err(e) => e,
    };
    assert!(err.to_string().contains("开放回合"));

    run.checkpoint(json!({"step": 2})).await.unwrap();
    s.append(ItemDraft::assistant("...").group("g1").unwrap())
        .await
        .unwrap();
    let grouped = s.items().group("g1").collect().await.unwrap();
    assert_eq!(grouped.len(), 1);

    run.suspend().await.unwrap();
    run.resume().await.unwrap();
    run.complete().await.unwrap();
    assert!(s.open_run().await.unwrap().is_none());
    let again = s.begin_run().open().await.unwrap();
    again.cancel().await.unwrap();
}

#[tokio::test]
async fn item_patch_delta_and_ext_body() {
    let hub = SessionHub::memory();
    let s = hub.create().open().await.unwrap();
    let item = s.append_assistant("hel").await.unwrap();
    s.patch_item(
        &item.id,
        ItemPatch {
            text_append: Some("lo".into()),
            ..ItemPatch::default()
        },
    )
    .await
    .unwrap();
    let msgs = s.protocol_messages().await.unwrap();
    assert_eq!(msgs[0].content_text().unwrap(), "hello");

    let ext = s
        .append(
            ItemDraft::ext("reason", json!("scratch"))
                .unwrap()
                .protocol_visible(false),
        )
        .await
        .unwrap();
    assert_eq!(ext.kind(), ItemKind::Ext);
    let found = s.items().ext_kind("reason").collect().await.unwrap();
    assert_eq!(found.len(), 1);
}

#[tokio::test]
async fn compact_context_and_pin() {
    let hub = SessionHub::memory();
    let s = hub.create().open().await.unwrap();
    s.append_user("a").await.unwrap();
    let pin = s
        .append(ItemDraft::user("keep").pinned(true))
        .await
        .unwrap();
    s.append_user("b").await.unwrap();
    s.append_user("c").await.unwrap();
    s.compact(2, ItemDraft::system("folded")).await.unwrap();

    let full = s.protocol_messages().await.unwrap();
    // pinned user "keep" + unhidden "c" + marker system
    assert!(
        full.iter()
            .any(|m| m.content_text().as_deref() == Some("keep"))
    );
    assert!(
        full.iter()
            .any(|m| m.content_text().as_deref() == Some("c"))
    );
    assert!(
        !full
            .iter()
            .any(|m| m.content_text().as_deref() == Some("a"))
    );
    assert!(pin.pinned);

    let sliced = s
        .protocol_context(&ContextPolicy {
            max_items: Some(1),
            include_pinned: true,
        })
        .await
        .unwrap();
    assert!(sliced.omitted >= 1);
}

#[tokio::test]
async fn fork_until_and_dynamic_refs() {
    let hub = SessionHub::memory();
    let s = hub
        .create()
        .bind("primary", ExtRef::new("skill", "kb").unwrap())
        .open()
        .await
        .unwrap();
    let u = s.append_user("1").await.unwrap();
    s.append_assistant("2").await.unwrap();
    s.refs()
        .bind("file", ExtRef::new("blob", "abc").unwrap())
        .await
        .unwrap();

    let child = s.fork_until(u.seq).await.unwrap();
    assert_eq!(child.get().await.unwrap().item_count, 1);
    assert!(child.get().await.unwrap().refs.get("primary").is_some());

    let from_item = s.fork_at(&u.id).await.unwrap();
    assert_eq!(
        from_item.get().await.unwrap().parent_item_id.as_ref(),
        Some(&u.id)
    );
}

#[tokio::test]
async fn idempotent_search_export_import() {
    let hub = SessionHub::memory();
    let s = hub.create().title("alpha").open().await.unwrap();
    let a = s
        .append(ItemDraft::user("needle-xyz").client_id("c1"))
        .await
        .unwrap();
    let b = s
        .append(ItemDraft::user("needle-xyz").client_id("c1"))
        .await
        .unwrap();
    assert_eq!(a.id, b.id);
    assert_eq!(s.get().await.unwrap().item_count, 1);

    let hit = hub
        .select()
        .content_contains("needle-xyz")
        .list()
        .await
        .unwrap();
    assert_eq!(hit.len(), 1);

    let snap = s.export().await.unwrap();
    let copied = hub.import(snap, ImportMode::Copy).await.unwrap();
    assert_ne!(copied.id, *s.id());
    assert_eq!(copied.item_count, 1);
}
