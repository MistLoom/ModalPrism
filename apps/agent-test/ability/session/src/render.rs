use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::error::SessionError;
use crate::model::{Conversation, ConversationId, ItemBody, ItemId, ItemKind, SessionItem};
use crate::query::{ItemQuery, TagFilter};
use crate::store::ConversationStore;

/// 渲染窗口。只描述数据范围，不规定 UI 滚动实现。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderWindow {
    Latest { limit: u32 },
    After { seq: u64, limit: u32 },
    Range { from_seq: u64, to_seq: u64 },
}

impl RenderWindow {
    pub fn fingerprint(&self) -> String {
        match self {
            Self::Latest { limit } => format!("latest:{limit}"),
            Self::After { seq, limit } => format!("after:{seq}:{limit}"),
            Self::Range { from_seq, to_seq } => format!("range:{from_seq}:{to_seq}"),
        }
    }

    pub fn to_item_query(&self, kinds: Option<&[ItemKind]>, tags: Option<&TagFilter>) -> ItemQuery {
        let (after_seq, before_seq, limit, reverse) = match self {
            Self::Latest { limit } => (None, None, Some(*limit), true),
            Self::After { seq, limit } => (Some(*seq), None, Some(*limit), false),
            Self::Range { from_seq, to_seq } => {
                (Some(from_seq.saturating_sub(1)), Some(*to_seq), None, false)
            }
        };
        ItemQuery {
            tags: tags.cloned(),
            kinds: kinds.map(|k| k.to_vec()),
            after_seq,
            before_seq,
            limit,
            reverse,
            ..ItemQuery::default()
        }
    }
}

/// 视图规格。`view` 由调用方定义，本层只作缓存键，不解释含义。
#[derive(Clone, Debug)]
pub struct RenderViewSpec {
    pub view: String,
    pub window: RenderWindow,
    pub kinds: Option<Vec<ItemKind>>,
    pub tags: Option<TagFilter>,
}

impl RenderViewSpec {
    pub fn default_latest(limit: u32) -> Self {
        Self {
            view: "default".into(),
            window: RenderWindow::Latest { limit },
            kinds: None,
            tags: None,
        }
    }

    fn cache_key(&self, conversation_id: &ConversationId, revision: u64) -> String {
        format!(
            "{}:{}:{}:{}:{}",
            conversation_id,
            revision,
            self.view,
            self.window.fingerprint(),
            kinds_key(self.kinds.as_deref())
        )
    }
}

fn kinds_key(kinds: Option<&[ItemKind]>) -> String {
    match kinds {
        None => "*".into(),
        Some(ks) => ks.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(","),
    }
}

/// 渲染游标（按 seq，与存储序号对齐）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderCursor {
    pub seq: u64,
}

/// 正文引用：短内容内联，长内容延迟加载。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RenderBody {
    Inline { text: String },
    Deferred { item_id: ItemId, bytes_hint: u64 },
}

/// 渲染节点。字段均为数据，不含组件/样式约定。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderNode {
    pub id: ItemId,
    pub seq: u64,
    pub kind: ItemKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    pub body: RenderBody,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext_kind: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
}

/// 一次渲染快照。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderSnapshot {
    pub conversation_id: ConversationId,
    pub revision: u64,
    pub view: String,
    pub nodes: Vec<RenderNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<RenderCursor>,
    pub truncated: bool,
}

/// 内联/摘要阈值。
#[derive(Clone, Debug)]
pub struct RenderPolicy {
    pub excerpt_chars: usize,
    pub inline_chars: usize,
}

impl Default for RenderPolicy {
    fn default() -> Self {
        Self {
            excerpt_chars: 240,
            inline_chars: 4096,
        }
    }
}

struct CacheInner {
    cap: usize,
    map: HashMap<String, RenderSnapshot>,
    order: VecDeque<String>,
}

/// 渲染快照缓存（按会话 revision 失效）。
#[derive(Clone)]
pub struct RenderCache {
    inner: Arc<RwLock<CacheInner>>,
}

impl RenderCache {
    pub fn new(cap: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(CacheInner {
                cap: cap.max(1),
                map: HashMap::new(),
                order: VecDeque::new(),
            })),
        }
    }

    pub async fn get(&self, key: &str) -> Option<RenderSnapshot> {
        self.inner.read().await.map.get(key).cloned()
    }

    pub async fn put(&self, key: String, snapshot: RenderSnapshot) {
        let mut inner = self.inner.write().await;
        if inner.map.contains_key(&key) {
            inner.map.insert(key, snapshot);
            return;
        }
        while inner.order.len() >= inner.cap {
            if let Some(old) = inner.order.pop_front() {
                inner.map.remove(&old);
            }
        }
        inner.order.push_back(key.clone());
        inner.map.insert(key, snapshot);
    }

    pub async fn invalidate(&self, conversation_id: &ConversationId) {
        let prefix = format!("{conversation_id}:");
        let mut inner = self.inner.write().await;
        inner.order.retain(|k| !k.starts_with(&prefix));
        inner.map.retain(|k, _| !k.starts_with(&prefix));
    }
}

/// 渲染加载器：快照缓存 + 正文延迟读取。经 Hub 装配。
pub struct RenderLoader {
    store: Arc<dyn ConversationStore>,
    cache: RenderCache,
    policy: RenderPolicy,
}

impl RenderLoader {
    pub(crate) fn new(
        store: Arc<dyn ConversationStore>,
        cache: RenderCache,
        policy: RenderPolicy,
    ) -> Self {
        Self {
            store,
            cache,
            policy,
        }
    }

    pub async fn load(
        &self,
        id: &ConversationId,
        spec: &RenderViewSpec,
    ) -> Result<RenderSnapshot, SessionError> {
        let conv = self
            .store
            .get_conversation(id)
            .await?
            .ok_or_else(|| SessionError::not_found(id))?;
        let key = spec.cache_key(id, conv.revision);
        if let Some(hit) = self.cache.get(&key).await {
            return Ok(hit);
        }
        let query = spec
            .window
            .to_item_query(spec.kinds.as_deref(), spec.tags.as_ref());
        let page = self.store.query_items(id, &query).await?;
        let snapshot = project_snapshot(&conv, spec, page.items, page.next, &self.policy);
        self.cache.put(key, snapshot.clone()).await;
        Ok(snapshot)
    }

    pub async fn load_body(
        &self,
        id: &ConversationId,
        item_id: &ItemId,
    ) -> Result<String, SessionError> {
        let page = self.store.query_items(id, &ItemQuery::all()).await?;
        let item = page
            .items
            .into_iter()
            .find(|i| &i.id == item_id)
            .ok_or_else(|| SessionError::not_found(item_id))?;
        Ok(item.body.as_text())
    }
}

fn project_snapshot(
    conv: &Conversation,
    spec: &RenderViewSpec,
    mut items: Vec<SessionItem>,
    next_raw: Option<String>,
    policy: &RenderPolicy,
) -> RenderSnapshot {
    if matches!(spec.window, RenderWindow::Latest { .. }) {
        items.reverse();
    }
    let truncated = next_raw.is_some();
    let next = next_raw
        .and_then(|s| s.parse::<u64>().ok())
        .map(|seq| RenderCursor { seq });
    let nodes = items
        .iter()
        .map(|item| project_node(item, policy))
        .collect();
    RenderSnapshot {
        conversation_id: conv.id.clone(),
        revision: conv.revision,
        view: spec.view.clone(),
        nodes,
        next,
        truncated,
    }
}

fn project_node(item: &SessionItem, policy: &RenderPolicy) -> RenderNode {
    let text = item.body.as_text();
    let excerpt = if text.chars().count() > policy.excerpt_chars {
        Some(text.chars().take(policy.excerpt_chars).collect())
    } else if text.is_empty() {
        None
    } else {
        Some(text.clone())
    };
    let body = if text.chars().count() > policy.inline_chars {
        RenderBody::Deferred {
            item_id: item.id.clone(),
            bytes_hint: text.len() as u64,
        }
    } else {
        RenderBody::Inline { text }
    };
    let media = match &item.body {
        ItemBody::Artifact { media, .. } => Some(media.clone()),
        _ => None,
    };
    RenderNode {
        id: item.id.clone(),
        seq: item.seq,
        kind: item.kind(),
        title: item.body.title_hint(),
        excerpt,
        body,
        tags: item.tags.as_slice_owned(),
        media,
        group_id: item.group_id.clone(),
        ext_kind: item.body.ext_kind().map(str::to_string),
        pinned: item.pinned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ConversationStatus, ItemDraft, TagSet};
    use crate::store::{ConversationStore, MemoryStore};

    fn conv() -> Conversation {
        Conversation {
            id: ConversationId::new(),
            title: Some("t".into()),
            status: ConversationStatus::Active,
            tags: TagSet::new(),
            created_at: 1,
            updated_at: 1,
            item_count: 0,
            revision: 1,
            parent_id: None,
            parent_item_id: None,
            compact_until: None,
            refs: crate::ext::RefMap::new(),
            metrics: crate::ext::MetricMap::new(),
            ext: serde_json::Map::new(),
        }
    }

    #[tokio::test]
    async fn cache_hit_until_revision_changes() {
        let store = Arc::new(MemoryStore::new());
        let c = conv();
        store.upsert_conversation(&c).await.unwrap();
        store
            .append_items(&c.id, &[ItemDraft::user("hello")])
            .await
            .unwrap();
        let loader = RenderLoader::new(store.clone(), RenderCache::new(8), RenderPolicy::default());
        let spec = RenderViewSpec::default_latest(10);
        let a = loader.load(&c.id, &spec).await.unwrap();
        let b = loader.load(&c.id, &spec).await.unwrap();
        assert_eq!(a, b);
        store
            .append_items(&c.id, &[ItemDraft::assistant("hi")])
            .await
            .unwrap();
        loader.cache.invalidate(&c.id).await;
        let d = loader.load(&c.id, &spec).await.unwrap();
        assert_eq!(d.nodes.len(), 2);
        assert_ne!(d.revision, a.revision);
    }

    #[tokio::test]
    async fn long_body_is_deferred() {
        let store = Arc::new(MemoryStore::new());
        let c = conv();
        store.upsert_conversation(&c).await.unwrap();
        let long = "x".repeat(80);
        store
            .append_items(&c.id, &[ItemDraft::user(long.clone())])
            .await
            .unwrap();
        let policy = RenderPolicy {
            excerpt_chars: 10,
            inline_chars: 20,
        };
        let loader = RenderLoader::new(store, RenderCache::new(4), policy);
        let snap = loader
            .load(&c.id, &RenderViewSpec::default_latest(5))
            .await
            .unwrap();
        assert!(matches!(snap.nodes[0].body, RenderBody::Deferred { .. }));
        let body = loader.load_body(&c.id, &snap.nodes[0].id).await.unwrap();
        assert_eq!(body, long);
    }
}
