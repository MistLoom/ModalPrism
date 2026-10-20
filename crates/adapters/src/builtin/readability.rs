//! 从 HTML 抽出标题层级和段落。

use async_trait::async_trait;
use scraper::{ElementRef, Html, Node, Selector};
use serde_json::Value;
use strata_model::{FacetKind, RenderLevel};

use crate::error::AdapterError;
use crate::model::{
    AdapterInput, AdapterMeta, AdapterRole, ContentOutput, DraftBody, FacetDraft, Feedback,
    InputSelector, OutputDecl, Quality,
};
use crate::textutil::{absolute_http, collapse_ws, locator_text};
use crate::traits::ContentAdapter;

const SKIP: &[&str] = &[
    "script", "style", "noscript", "nav", "footer", "header", "form", "iframe", "svg",
];

pub struct Readability {
    meta: AdapterMeta,
}

impl Default for Readability {
    fn default() -> Self {
        Self::new()
    }
}

impl Readability {
    pub fn new() -> Self {
        Self {
            meta: AdapterMeta {
                id: "adapter/readability-md".into(),
                version: "1".into(),
                role: AdapterRole::Content,
                consumes: vec![InputSelector {
                    kind: Some("raw".into()),
                    variant: None,
                    part_role: Some("document".into()),
                    media_type: Some("text/html".into()),
                }],
                produces: vec![
                    OutputDecl::Facet {
                        kind: "md".into(),
                        variant: Some("readable".into()),
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
impl ContentAdapter for Readability {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    async fn adapt(
        &self,
        input: &AdapterInput,
        _spec: &Value,
    ) -> Result<ContentOutput, AdapterError> {
        let html = String::from_utf8_lossy(&input.view.bytes);
        let document = Html::parse_document(&html);
        let root = select_root(&document);
        let base = locator_text(&input.locator);
        let mut md = String::new();
        let mut text = String::new();
        if let Some(root) = root {
            walk(root, base, &mut md, &mut text);
        }
        let md = md.trim().to_string();
        let text = collapse_ws(&text);
        let feedback = feedback_for(input, &text);
        Ok(ContentOutput {
            facets: vec![
                FacetDraft {
                    kind: FacetKind::Md,
                    variant: Some("readable".into()),
                    body: DraftBody::Text(md),
                    media_type: Some("text/markdown".into()),
                    derived_from: Vec::new(),
                },
                FacetDraft {
                    kind: FacetKind::Text,
                    variant: None,
                    body: DraftBody::Text(text),
                    media_type: Some("text/plain".into()),
                    derived_from: Vec::new(),
                },
            ],
            links: Vec::new(),
            feedback,
        })
    }
}

fn feedback_for(input: &AdapterInput, text: &str) -> Option<Feedback> {
    let blocked = input.detection.as_ref().is_some_and(|detection| {
        detection
            .traits
            .iter()
            .any(|trait_name| trait_name == "shell-page" || trait_name == "challenge-page")
    });
    if blocked {
        return Some(Feedback {
            needs_render: Some(RenderLevel::L2),
            quality: Quality::Blocked,
            missing: Vec::new(),
        });
    }
    if text.is_empty() {
        return Some(Feedback {
            needs_render: Some(RenderLevel::L2),
            quality: Quality::Empty,
            missing: vec!["body".into()],
        });
    }
    if text.chars().count() < 80 {
        return Some(Feedback {
            needs_render: Some(RenderLevel::L2),
            quality: Quality::Thin,
            missing: Vec::new(),
        });
    }
    None
}

fn select_root(document: &Html) -> Option<ElementRef<'_>> {
    for query in ["article", "main", "[role=main]", "body"] {
        let Ok(selector) = Selector::parse(query) else {
            continue;
        };
        if let Some(element) = document.select(&selector).next() {
            return Some(element);
        }
    }
    None
}

fn walk(element: ElementRef<'_>, base: &str, md: &mut String, text: &mut String) {
    let name = element.value().name();
    if SKIP.contains(&name) {
        return;
    }
    if let Some(level) = heading_level(name) {
        let inline = inline_text(element, base, text);
        if inline.is_empty() {
            return;
        }
        separate(md);
        md.push_str(&"#".repeat(level));
        md.push(' ');
        md.push_str(&inline);
        md.push_str("\n\n");
        return;
    }
    if name == "p" || name == "li" || name == "blockquote" {
        let inline = inline_text(element, base, text);
        if inline.is_empty() {
            return;
        }
        separate(md);
        md.push_str(&inline);
        md.push_str("\n\n");
        return;
    }
    for child in element.children() {
        if let Some(child) = ElementRef::wrap(child) {
            walk(child, base, md, text);
        }
    }
}

fn inline_text(element: ElementRef<'_>, base: &str, plain: &mut String) -> String {
    let mut out = String::new();
    write_inline(element, base, &mut out, plain);
    collapse_ws(&out)
}

fn write_inline(element: ElementRef<'_>, base: &str, out: &mut String, plain: &mut String) {
    if SKIP.contains(&element.value().name()) {
        return;
    }
    for child in element.children() {
        match child.value() {
            Node::Text(text) => {
                let piece = collapse_ws(text);
                if piece.is_empty() {
                    continue;
                }
                push_piece(out, &piece);
                push_piece(plain, &piece);
            }
            Node::Element(_) => {
                let Some(child) = ElementRef::wrap(child) else {
                    continue;
                };
                if child.value().name() == "a" {
                    let label = {
                        let mut label = String::new();
                        write_inline(child, base, &mut label, plain);
                        collapse_ws(&label)
                    };
                    if let Some(href) = child
                        .attr("href")
                        .and_then(|href| absolute_http(base, href))
                    {
                        if !label.is_empty() {
                            push_piece(out, &format!("[{label}]({href})"));
                        }
                    } else if !label.is_empty() {
                        push_piece(out, &label);
                    }
                } else if heading_level(child.value().name()).is_none() {
                    write_inline(child, base, out, plain);
                }
            }
            _ => {}
        }
    }
}

fn push_piece(out: &mut String, piece: &str) {
    if piece.is_empty() {
        return;
    }
    if !out.is_empty() && !out.ends_with(' ') && !piece.starts_with(' ') {
        out.push(' ');
    }
    out.push_str(piece);
}

fn separate(md: &mut String) {
    if md.is_empty() || md.ends_with("\n\n") {
        return;
    }
    if md.ends_with('\n') {
        md.push('\n');
    } else {
        md.push_str("\n\n");
    }
}

fn heading_level(name: &str) -> Option<usize> {
    match name {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}
