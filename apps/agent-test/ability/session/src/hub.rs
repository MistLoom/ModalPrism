use std::sync::Arc;

use crate::error::SessionError;
use crate::model::{
    AppendOutcome, ContextPolicy, ContextView, Conversation, ConversationBundle, ConversationId,
    ConversationPatch, ConversationStatus, CreateSpec, ItemDraft, ItemId, ItemPatch, SessionItem,
    TagOp, TagScope, TagStat, now_ms,
};
use crate::query::{ConversationQuery, ItemQuery, Page};
use crate::render::{RenderCache, RenderLoader, RenderPolicy};
use crate::run::{Run, RunId, RunPatch, RunQuery, RunSpec, RunStatus};
use crate::snapshot::{ImportMode, SNAPSHOT_FORMAT, SessionSnapshot};
use crate::store::{ConversationStore, MemoryStore};

/// Hub 装配参数。
#[derive(Clone, Debug)]
pub struct HubConfig {
    pub render: RenderPolicy,
    pub cache_capacity: usize,
}

impl Default for HubConfig {
    fn default() -> Self {
        Self {
            render: RenderPolicy::default(),
            cache_capacity: 256,
        }
    }
}

pub(crate) struct HubInner {
    pub(crate) store: Arc<dyn ConversationStore>,
    pub(crate) render_cache: RenderCache,
    pub(crate) render_policy: RenderPolicy,
}

/// 会话管道唯一入口。存储与渲染经此装配；能力/用户事件走 Host，不在 Hub 另建管道。
#[derive(Clone)]
pub struct SessionHub {
    pub(crate) inner: Arc<HubInner>,
}

impl SessionHub {
    pub fn memory() -> Self {
        Self::memory_with(HubConfig::default())
    }

    pub fn memory_with(config: HubConfig) -> Self {
        Self::new(Arc::new(MemoryStore::new()), config)
    }

    pub fn with_store(store: Arc<dyn ConversationStore>) -> Self {
        Self::new(store, HubConfig::default())
    }

    pub fn new(store: Arc<dyn ConversationStore>, config: HubConfig) -> Self {
        Self {
            inner: Arc::new(HubInner {
                store,
                render_cache: RenderCache::new(config.cache_capacity),
                render_policy: config.render,
            }),
        }
    }

    pub fn render(&self) -> RenderLoader {
        RenderLoader::new(
            self.inner.store.clone(),
            self.inner.render_cache.clone(),
            self.inner.render_policy.clone(),
        )
    }

    pub async fn create_conversation(
        &self,
        spec: CreateSpec,
    ) -> Result<Conversation, SessionError> {
        let now = now_ms();
        let conv = Conversation {
            id: ConversationId::new(),
            title: spec.title.filter(|s| !s.is_empty()),
            status: ConversationStatus::Active,
            tags: spec.tags,
            created_at: now,
            updated_at: now,
            item_count: 0,
            revision: 1,
            parent_id: spec.parent_id,
            parent_item_id: spec.parent_item_id,
            compact_until: None,
            refs: spec.refs,
            metrics: crate::ext::MetricMap::new(),
            ext: spec.ext,
        };
        self.inner.store.upsert_conversation(&conv).await?;
        Ok(conv)
    }

    pub async fn get(&self, id: &ConversationId) -> Result<Conversation, SessionError> {
        self.inner
            .store
            .get_conversation(id)
            .await?
            .ok_or_else(|| SessionError::not_found(id))
    }

    pub async fn patch(
        &self,
        id: &ConversationId,
        patch: ConversationPatch,
    ) -> Result<Conversation, SessionError> {
        let conv = self.inner.store.patch_conversation(id, &patch).await?;
        self.inner.render_cache.invalidate(id).await;
        Ok(conv)
    }

    pub async fn delete(&self, id: &ConversationId) -> Result<(), SessionError> {
        let n = self.inner.store.delete_conversations(&[id.clone()]).await?;
        if n == 0 {
            return Err(SessionError::not_found(id));
        }
        self.inner.render_cache.invalidate(id).await;
        Ok(())
    }

    pub async fn delete_many(&self, ids: &[ConversationId]) -> Result<u64, SessionError> {
        let existing = self
            .inner
            .store
            .query_conversations(&ConversationQuery {
                ids: Some(ids.to_vec()),
                limit: None,
                ..ConversationQuery::default()
            })
            .await?
            .items
            .into_iter()
            .map(|c| c.id)
            .collect::<Vec<_>>();
        let n = self.inner.store.delete_conversations(&existing).await?;
        for id in &existing {
            self.inner.render_cache.invalidate(id).await;
        }
        Ok(n)
    }

    pub async fn query(
        &self,
        mut query: ConversationQuery,
    ) -> Result<Page<Conversation>, SessionError> {
        if query.limit.is_some() {
            query.limit = query.clamped_limit();
        }
        self.inner.store.query_conversations(&query).await
    }

    pub async fn append(
        &self,
        id: &ConversationId,
        drafts: &[ItemDraft],
    ) -> Result<AppendOutcome, SessionError> {
        let out = self.inner.store.append_items(id, drafts).await?;
        if out.created == 0 {
            return Ok(out);
        }
        self.inner.render_cache.invalidate(id).await;
        Ok(out)
    }

    pub async fn items(
        &self,
        id: &ConversationId,
        mut query: ItemQuery,
    ) -> Result<Page<SessionItem>, SessionError> {
        if query.limit.is_some() {
            query.limit = query.clamped_limit();
        }
        self.inner.store.query_items(id, &query).await
    }

    pub async fn delete_items(
        &self,
        id: &ConversationId,
        item_ids: &[ItemId],
    ) -> Result<u64, SessionError> {
        let n = self.inner.store.delete_items(id, item_ids).await?;
        if n > 0 {
            self.inner.render_cache.invalidate(id).await;
        }
        Ok(n)
    }

    pub async fn protocol_messages(
        &self,
        id: &ConversationId,
    ) -> Result<Vec<ai_protocol::Message>, SessionError> {
        Ok(self
            .protocol_context(id, &ContextPolicy::default())
            .await?
            .messages)
    }

    pub async fn protocol_context(
        &self,
        id: &ConversationId,
        policy: &ContextPolicy,
    ) -> Result<ContextView, SessionError> {
        let page = self.inner.store.query_items(id, &ItemQuery::all()).await?;
        let visible: Vec<&SessionItem> = page.items.iter().filter(|i| i.protocol_visible).collect();
        let mut selected: Vec<&SessionItem> = if let Some(max) = policy.max_items {
            let max = max as usize;
            let pinned: Vec<&SessionItem> = if policy.include_pinned {
                visible.iter().copied().filter(|i| i.pinned).collect()
            } else {
                Vec::new()
            };
            let rest: Vec<&SessionItem> = visible
                .iter()
                .copied()
                .filter(|i| !(policy.include_pinned && i.pinned))
                .collect();
            let start = rest.len().saturating_sub(max);
            let mut out = rest[start..].to_vec();
            for p in pinned {
                if !out.iter().any(|i| i.id == p.id) {
                    out.push(p);
                }
            }
            out
        } else {
            visible
        };
        selected.sort_by_key(|i| i.seq);
        let omitted = page
            .items
            .iter()
            .filter(|i| i.protocol_visible)
            .count()
            .saturating_sub(selected.len()) as u64;
        let item_ids = selected.iter().map(|i| i.id.clone()).collect();
        let messages = selected
            .iter()
            .filter_map(|i| i.as_protocol_message().cloned())
            .collect();
        Ok(ContextView {
            messages,
            item_ids,
            omitted,
        })
    }

    pub async fn fork(
        &self,
        id: &ConversationId,
        until_seq: Option<u64>,
    ) -> Result<Conversation, SessionError> {
        let out = self.inner.store.fork(id, until_seq).await?;
        Ok(out.conversation)
    }

    pub async fn fork_at(
        &self,
        id: &ConversationId,
        item_id: &ItemId,
    ) -> Result<Conversation, SessionError> {
        let page = self.inner.store.query_items(id, &ItemQuery::all()).await?;
        let item = page
            .items
            .iter()
            .find(|i| &i.id == item_id)
            .ok_or_else(|| SessionError::not_found(item_id))?;
        let seq = item.seq;
        let child = self.fork(id, Some(seq)).await?;
        self.patch(
            &child.id,
            ConversationPatch {
                parent_item_id: Some(item_id.clone()),
                ..ConversationPatch::default()
            },
        )
        .await
    }

    pub async fn patch_item(
        &self,
        id: &ConversationId,
        item_id: &ItemId,
        patch: ItemPatch,
    ) -> Result<SessionItem, SessionError> {
        let item = self.inner.store.patch_item(id, item_id, &patch).await?;
        self.inner.render_cache.invalidate(id).await;
        Ok(item)
    }

    pub async fn compact(
        &self,
        id: &ConversationId,
        until_seq: u64,
        marker: ItemDraft,
    ) -> Result<AppendOutcome, SessionError> {
        let page = self.inner.store.query_items(id, &ItemQuery::all()).await?;
        for item in &page.items {
            if item.seq <= until_seq && item.protocol_visible && !item.pinned {
                self.inner
                    .store
                    .patch_item(
                        id,
                        &item.id,
                        &ItemPatch {
                            protocol_visible: Some(false),
                            ..ItemPatch::default()
                        },
                    )
                    .await?;
            }
        }
        self.patch(
            id,
            ConversationPatch {
                compact_until: Some(until_seq),
                ..ConversationPatch::default()
            },
        )
        .await?;
        let out = self.append(id, &[marker]).await?;
        Ok(out)
    }

    pub async fn begin_run(&self, id: &ConversationId, spec: RunSpec) -> Result<Run, SessionError> {
        let _ = self.get(id).await?;
        if let Some(open) = self.inner.store.open_run(id).await? {
            return Err(SessionError::conflict(format!(
                "会话已有开放回合: {}",
                open.id
            )));
        }
        if !matches!(spec.status, RunStatus::Pending | RunStatus::Active) {
            return Err(SessionError::invalid(
                "开启回合状态只能是 pending 或 active",
            ));
        }
        let run = Run::new(id.clone(), &spec);
        self.inner.store.upsert_run(&run).await?;
        Ok(run)
    }

    pub async fn get_run(&self, run_id: &RunId) -> Result<Run, SessionError> {
        self.inner
            .store
            .get_run(run_id)
            .await?
            .ok_or_else(|| SessionError::not_found(run_id))
    }

    pub async fn open_run(&self, id: &ConversationId) -> Result<Option<Run>, SessionError> {
        self.inner.store.open_run(id).await
    }

    pub async fn query_runs(&self, query: RunQuery) -> Result<Page<Run>, SessionError> {
        self.inner.store.query_runs(&query).await
    }

    pub async fn patch_run(&self, run_id: &RunId, patch: RunPatch) -> Result<Run, SessionError> {
        let run = self.inner.store.patch_run(run_id, &patch).await?;
        Ok(run)
    }

    pub async fn export(&self, id: &ConversationId) -> Result<SessionSnapshot, SessionError> {
        let conv = self.get(id).await?;
        let items = self.items(id, ItemQuery::all()).await?.items;
        let runs = self
            .inner
            .store
            .query_runs(&RunQuery {
                conversation_id: Some(id.clone()),
                ..RunQuery::default()
            })
            .await?
            .items;
        Ok(SessionSnapshot::new(conv, items, runs))
    }

    pub async fn import(
        &self,
        snap: SessionSnapshot,
        mode: ImportMode,
    ) -> Result<Conversation, SessionError> {
        if snap.format > SNAPSHOT_FORMAT {
            return Err(SessionError::invalid(format!(
                "快照格式不兼容: {}",
                snap.format
            )));
        }
        match mode {
            ImportMode::Restore => {
                self.inner.store.restore_snapshot(&snap).await?;
                Ok(snap.conversation)
            }
            ImportMode::Copy => {
                let spec = CreateSpec {
                    title: snap.conversation.title.clone(),
                    tags: snap.conversation.tags.clone(),
                    parent_id: Some(snap.conversation.id.clone()),
                    parent_item_id: snap.conversation.parent_item_id.clone(),
                    refs: snap.conversation.refs.clone(),
                    ext: snap.conversation.ext.clone(),
                };
                let conv = self.create_conversation(spec).await?;
                let drafts: Vec<ItemDraft> = snap
                    .items
                    .into_iter()
                    .map(|item| ItemDraft {
                        body: item.body,
                        tags: item.tags,
                        protocol_visible: item.protocol_visible,
                        pinned: item.pinned,
                        group_id: item.group_id,
                        client_id: None,
                        parent_item: None,
                        refs: item.refs,
                        metrics: item.metrics,
                        ext: item.ext,
                    })
                    .collect();
                if !drafts.is_empty() {
                    self.append(&conv.id, &drafts).await?;
                }
                if let Some(until) = snap.conversation.compact_until {
                    self.patch(
                        &conv.id,
                        ConversationPatch {
                            compact_until: Some(until),
                            ..ConversationPatch::default()
                        },
                    )
                    .await?;
                }
                self.get(&conv.id).await
            }
        }
    }

    pub async fn list_tags(&self, scope: TagScope) -> Result<Vec<TagStat>, SessionError> {
        self.inner.store.list_tags(scope).await
    }

    pub async fn rename_tag(&self, from: &str, to: &str) -> Result<u64, SessionError> {
        let from = crate::model::normalize_tag(from)?;
        let to_norm = crate::model::normalize_tag(to)?;
        if from == to_norm {
            return Ok(0);
        }
        let page = self
            .inner
            .store
            .query_conversations(&ConversationQuery {
                tags: Some(crate::query::TagFilter::all([&from])),
                limit: None,
                ..ConversationQuery::default()
            })
            .await?;
        let mut n = 0u64;
        for conv in page.items {
            self.patch(
                &conv.id,
                ConversationPatch {
                    tag_op: Some(TagOp::Remove(vec![from.clone()])),
                    ..ConversationPatch::default()
                },
            )
            .await?;
            self.patch(
                &conv.id,
                ConversationPatch {
                    tag_op: Some(TagOp::Add(vec![to_norm.clone()])),
                    ..ConversationPatch::default()
                },
            )
            .await?;
            n += 1;
        }
        Ok(n)
    }

    pub async fn read_many(
        &self,
        ids: &[ConversationId],
        item_query: ItemQuery,
    ) -> Result<Vec<ConversationBundle>, SessionError> {
        let page = self
            .inner
            .store
            .query_conversations(&ConversationQuery {
                ids: Some(ids.to_vec()),
                limit: None,
                ..ConversationQuery::default()
            })
            .await?;
        let mut out = Vec::with_capacity(page.items.len());
        for conv in page.items {
            let items = self.items(&conv.id, item_query.clone()).await?.items;
            out.push(ConversationBundle {
                conversation: conv,
                items,
            });
        }
        Ok(out)
    }

    pub(crate) async fn matching_ids(
        &self,
        query: &ConversationQuery,
    ) -> Result<Vec<ConversationId>, SessionError> {
        let page = self.inner.store.query_conversations(query).await?;
        Ok(page.items.into_iter().map(|c| c.id).collect())
    }
}
