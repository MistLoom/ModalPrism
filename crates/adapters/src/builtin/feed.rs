//! RSS 2.0 与 Atom 1.0。

use async_trait::async_trait;
use quick_xml::Reader;
use quick_xml::events::Event;
use serde_json::Value;
use strata_model::{FacetKind, Locator};

use crate::error::AdapterError;
use crate::model::{
    AdapterInput, AdapterMeta, AdapterRole, ContentOutput, DiscoverOutput, Discovery, DraftBody,
    FacetDraft, Follow, InputSelector, OutputDecl,
};
use crate::textutil::{
    absolute_http, guid_identity, locator_text, parse_timestamp, strip_tags, url_identity,
};
use crate::traits::{ContentAdapter, SearchAdapter};

pub struct FeedItems {
    meta: AdapterMeta,
}

impl Default for FeedItems {
    fn default() -> Self {
        Self::new()
    }
}

impl FeedItems {
    pub fn new() -> Self {
        Self {
            meta: AdapterMeta {
                id: "adapter/feed-items".into(),
                version: "1".into(),
                role: AdapterRole::Search,
                consumes: vec![InputSelector {
                    kind: Some("raw".into()),
                    variant: None,
                    part_role: Some("document".into()),
                    media_type: None,
                }],
                produces: vec![OutputDecl::Discovery],
                deterministic: true,
            },
        }
    }
}

pub struct FeedFulltext {
    meta: AdapterMeta,
}

impl Default for FeedFulltext {
    fn default() -> Self {
        Self::new()
    }
}

impl FeedFulltext {
    pub fn new() -> Self {
        Self {
            meta: AdapterMeta {
                id: "adapter/feed-fulltext".into(),
                version: "1".into(),
                role: AdapterRole::Content,
                consumes: vec![InputSelector {
                    kind: Some("raw".into()),
                    variant: None,
                    part_role: Some("document".into()),
                    media_type: None,
                }],
                produces: vec![
                    OutputDecl::Facet {
                        kind: "md".into(),
                        variant: None,
                    },
                    OutputDecl::Facet {
                        kind: "text".into(),
                        variant: None,
                    },
                ],
                deterministic: true,
            },
        }
    }
}

#[async_trait]
impl SearchAdapter for FeedItems {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    async fn discover(
        &self,
        input: &AdapterInput,
        _spec: &Value,
    ) -> Result<DiscoverOutput, AdapterError> {
        let feed = parse_feed(&input.view.bytes)?;
        let base = locator_text(&input.locator);
        let mut items = Vec::new();
        for entry in &feed.entries {
            let Some(discovery) = discovery_from(entry, base, input) else {
                continue;
            };
            items.push(discovery);
        }
        for (index, item) in items.iter_mut().enumerate() {
            item.hints.rank = Some((index + 1) as u32);
        }
        let feedback = if items.is_empty() {
            Some(crate::model::Feedback {
                needs_render: None,
                quality: crate::model::Quality::Empty,
                missing: vec!["item".into()],
            })
        } else {
            None
        };
        Ok(DiscoverOutput { items, feedback })
    }
}

#[async_trait]
impl ContentAdapter for FeedFulltext {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    async fn adapt(
        &self,
        input: &AdapterInput,
        _spec: &Value,
    ) -> Result<ContentOutput, AdapterError> {
        let feed = parse_feed(&input.view.bytes)?;
        if feed.title.is_none() && feed.entries.is_empty() {
            return Ok(ContentOutput {
                facets: Vec::new(),
                links: Vec::new(),
                feedback: Some(crate::model::Feedback {
                    needs_render: None,
                    quality: crate::model::Quality::Empty,
                    missing: vec!["item".into()],
                }),
            });
        }
        let mut md = String::new();
        let mut text = String::new();
        if let Some(title) = &feed.title {
            md.push_str("# ");
            md.push_str(title.trim());
            md.push_str("\n\n");
            text.push_str(title.trim());
        }
        for entry in &feed.entries {
            let body = entry
                .content
                .as_deref()
                .or(entry.summary.as_deref())
                .map(strip_tags)
                .unwrap_or_default();
            if let Some(title) = &entry.title {
                if !md.is_empty() && !md.ends_with("\n\n") {
                    md.push_str("\n\n");
                }
                md.push_str("## ");
                md.push_str(title.trim());
                md.push_str("\n\n");
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(title.trim());
            }
            if !body.is_empty() {
                md.push_str(&body);
                md.push_str("\n\n");
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(&body);
            }
        }
        Ok(ContentOutput {
            facets: vec![
                text_facet(FacetKind::Md, None, md.trim().to_string(), "text/markdown"),
                text_facet(FacetKind::Text, None, text, "text/plain"),
            ],
            links: Vec::new(),
            feedback: None,
        })
    }
}

fn discovery_from(entry: &Entry, base: &str, input: &AdapterInput) -> Option<Discovery> {
    let href = entry.link.as_deref()?;
    let absolute = absolute_http(base, href)?;
    let mut identity = vec![url_identity(&absolute)?];
    if let Some(guid) = entry.guid.as_deref().and_then(guid_identity) {
        if guid != identity[0] {
            identity.push(guid);
        }
    }
    let summary = entry
        .summary
        .as_deref()
        .map(strip_tags)
        .filter(|text| !text.is_empty());
    let hints = crate::model::DiscoveryHints {
        title: entry.title.clone(),
        published_at: entry.published.as_deref().and_then(parse_timestamp),
        author: entry.author.clone(),
        summary,
        rank: None,
        extra: Default::default(),
    };
    let (materialize, follow) = if let Some(content) = &entry.content {
        (
            vec![text_facet(
                FacetKind::Html,
                None,
                content.clone(),
                "text/html",
            )],
            Follow::No,
        )
    } else {
        (
            Vec::new(),
            Follow::Yes {
                accessor: input.source.accessor.clone(),
                render: input.source.render,
                priority: 0,
            },
        )
    };
    Some(Discovery {
        locator: Locator::Url { url: absolute },
        identity,
        hints,
        span: None,
        follow,
        materialize,
    })
}

fn text_facet(
    kind: FacetKind,
    variant: Option<String>,
    text: String,
    media_type: &str,
) -> FacetDraft {
    FacetDraft {
        kind,
        variant,
        body: DraftBody::Text(text),
        media_type: Some(media_type.to_string()),
        derived_from: Vec::new(),
    }
}

struct FeedDoc {
    title: Option<String>,
    entries: Vec<Entry>,
}

struct Entry {
    title: Option<String>,
    link: Option<String>,
    guid: Option<String>,
    published: Option<String>,
    author: Option<String>,
    summary: Option<String>,
    content: Option<String>,
}

fn parse_feed(bytes: &[u8]) -> Result<FeedDoc, AdapterError> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let text = String::from_utf8_lossy(bytes);
    let mut reader = Reader::from_str(&text);
    let mut buf = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut feed = FeedDoc {
        title: None,
        entries: Vec::new(),
    };
    let mut current: Option<Entry> = None;
    let mut field: Option<String> = None;
    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|error| AdapterError::permanent(error.to_string()))?;
        match event {
            Event::Start(event) => {
                let name = local_name(event.local_name().as_ref());
                if name == "item" || name == "entry" {
                    current = Some(empty_entry());
                } else if name == "link" {
                    if let Some(entry) = current.as_mut() {
                        capture_link(entry, &event);
                    }
                    if current.is_some() {
                        field = Some(name.clone());
                    }
                } else if is_field(&name) && (current.is_some() || name == "title") {
                    field = Some(name.clone());
                }
                stack.push(name);
            }
            Event::Empty(event) => {
                let name = local_name(event.local_name().as_ref());
                if name == "link" {
                    if let Some(entry) = current.as_mut() {
                        capture_link(entry, &event);
                    }
                }
            }
            Event::Text(event) => {
                assign_text(
                    &mut feed,
                    current.as_mut(),
                    field.as_deref(),
                    &stack,
                    &event_text(&event)?,
                )?;
            }
            Event::CData(event) => {
                let text = String::from_utf8_lossy(&event).into_owned();
                assign_text(&mut feed, current.as_mut(), field.as_deref(), &stack, &text)?;
            }
            Event::End(_) => {
                if let Some(name) = stack.pop() {
                    if name == "item" || name == "entry" {
                        if let Some(entry) = current.take() {
                            feed.entries.push(entry);
                        }
                    }
                    if field.as_deref() == Some(name.as_str()) {
                        field = None;
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(feed)
}

fn assign_text(
    feed: &mut FeedDoc,
    entry: Option<&mut Entry>,
    field: Option<&str>,
    stack: &[String],
    text: &str,
) -> Result<(), AdapterError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    let Some(field) = field else {
        return Ok(());
    };
    let in_entry = stack.iter().any(|name| name == "item" || name == "entry");
    if !in_entry {
        if field == "title" && feed.title.is_none() {
            feed.title = Some(text.to_string());
        }
        return Ok(());
    }
    let Some(entry) = entry else {
        return Ok(());
    };
    if field == "name" && !stack.iter().any(|name| name == "author") {
        return Ok(());
    }
    if field == "published" || field == "pubdate" {
        entry.published = Some(text.to_string());
        return Ok(());
    }
    if field == "updated" {
        if entry.published.is_none() {
            entry.published = Some(text.to_string());
        }
        return Ok(());
    }
    let slot = match field {
        "title" => &mut entry.title,
        "link" => &mut entry.link,
        "guid" | "id" => &mut entry.guid,
        "author" | "creator" | "name" => &mut entry.author,
        "description" | "summary" => &mut entry.summary,
        "encoded" | "content" => &mut entry.content,
        _ => return Ok(()),
    };
    if slot.is_none() {
        *slot = Some(text.to_string());
    }
    Ok(())
}

fn capture_link(entry: &mut Entry, event: &quick_xml::events::BytesStart<'_>) {
    let mut href = None;
    let mut rel = None;
    for attr in event.attributes().flatten() {
        let key = local_name(attr.key.local_name().as_ref());
        let Ok(value) = attr.unescape_value() else {
            continue;
        };
        if key == "href" {
            href = Some(value.into_owned());
        } else if key == "rel" {
            rel = Some(value.into_owned());
        }
    }
    let Some(href) = href else {
        return;
    };
    if entry.link.is_none() || rel.as_deref() == Some("alternate") {
        entry.link = Some(href);
    }
}

fn event_text(event: &quick_xml::events::BytesText<'_>) -> Result<String, AdapterError> {
    event
        .unescape()
        .map(|text| text.into_owned())
        .map_err(|error| AdapterError::permanent(error.to_string()))
}

fn local_name(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw).to_ascii_lowercase()
}

fn is_field(name: &str) -> bool {
    matches!(
        name,
        "title"
            | "link"
            | "guid"
            | "id"
            | "pubdate"
            | "published"
            | "updated"
            | "author"
            | "creator"
            | "name"
            | "description"
            | "summary"
            | "encoded"
            | "content"
    )
}

fn empty_entry() -> Entry {
    Entry {
        title: None,
        link: None,
        guid: None,
        published: None,
        author: None,
        summary: None,
        content: None,
    }
}
