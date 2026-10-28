use ai_protocol::Message;
use serde_json::Value;

use crate::error::SessionError;
use crate::ext::{ExtRef, RefOp};
use crate::hub::SessionHub;
use crate::iter::{ConversationIter, ItemIter};
use crate::model::{
    ContextPolicy, ContextView, Conversation, ConversationBundle, ConversationId,
    ConversationPatch, ConversationStatus, CreateSpec, ItemDraft, ItemId, ItemPatch, TagOp,
    TagScope, TagSet, TagStat,
};
use crate::query::{ConversationQuery, ItemQuery, TagFilter};
use crate::render::{RenderSnapshot, RenderViewSpec};
use crate::run::{Run, RunId, RunPatch, RunSpec, RunStatus};
use crate::snapshot::SessionSnapshot;

impl SessionHub {
    /// 创建会话（语法糖）。
    pub fn create(&self) -> CreateBuilder {
        CreateBuilder {
            hub: self.clone(),
            spec: CreateSpec::default(),
            error: None,
        }
    }

    /// 单会话句柄。
    pub fn session(&self, id: ConversationId) -> SessionHandle {
        SessionHandle {
            hub: self.clone(),
            id,
        }
    }

    /// 批量选择。无过滤条件时必须 `.all()` 才能变更。
    pub fn select(&self) -> SessionSelect {
        SessionSelect {
            hub: self.clone(),
            query: ConversationQuery::default(),
            unbounded: false,
        }
    }

    /// 标签目录。
    pub fn tags(&self) -> TagCatalog {
        TagCatalog { hub: self.clone() }
    }

    /// 全量会话迭代。
    pub fn iter(&self) -> ConversationIter {
        ConversationIter::new(
            self.clone(),
            ConversationQuery {
                limit: Some(50),
                ..ConversationQuery::default()
            },
        )
    }
}

/// 创建构建器。
pub struct CreateBuilder {
    hub: SessionHub,
    spec: CreateSpec,
    error: Option<SessionError>,
}

impl CreateBuilder {
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.spec.title = Some(title.into());
        self
    }

    pub fn tags(mut self, tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        if self.error.is_some() {
            return self;
        }
        match TagSet::from_list(tags) {
            Ok(set) => self.spec.tags = set,
            Err(err) => self.error = Some(err),
        }
        self
    }

    pub fn bind(mut self, slot: impl Into<String>, refer: ExtRef) -> Self {
        if self.error.is_some() {
            return self;
        }
        if let Err(err) = self.spec.refs.bind(slot, refer) {
            self.error = Some(err);
        }
        self
    }

    pub async fn open(self) -> Result<SessionHandle, SessionError> {
        if let Some(err) = self.error {
            return Err(err);
        }
        let conv = self.hub.create_conversation(self.spec).await?;
        Ok(SessionHandle {
            hub: self.hub,
            id: conv.id,
        })
    }
}

/// 单会话操作句柄。
#[derive(Clone)]
pub struct SessionHandle {
    hub: SessionHub,
    id: ConversationId,
}

impl SessionHandle {
    pub fn id(&self) -> &ConversationId {
        &self.id
    }

    pub async fn get(&self) -> Result<Conversation, SessionError> {
        self.hub.get(&self.id).await
    }

    pub async fn set_title(&self, title: impl Into<String>) -> Result<Conversation, SessionError> {
        self.hub
            .patch(
                &self.id,
                ConversationPatch {
                    title: Some(title.into()),
                    ..ConversationPatch::default()
                },
            )
            .await
    }

    pub fn tags(&self) -> TagOps {
        TagOps {
            hub: self.hub.clone(),
            id: self.id.clone(),
        }
    }

    pub fn refs(&self) -> RefOps {
        RefOps {
            hub: self.hub.clone(),
            id: self.id.clone(),
        }
    }

    pub async fn append(
        &self,
        draft: ItemDraft,
    ) -> Result<crate::model::SessionItem, SessionError> {
        let out = self.hub.append(&self.id, &[draft]).await?;
        out.items
            .into_iter()
            .next()
            .ok_or_else(|| SessionError::invalid("追加未返回条目"))
    }

    pub async fn append_many(
        &self,
        drafts: impl IntoIterator<Item = ItemDraft>,
    ) -> Result<Vec<crate::model::SessionItem>, SessionError> {
        let drafts: Vec<_> = drafts.into_iter().collect();
        Ok(self.hub.append(&self.id, &drafts).await?.items)
    }

    pub async fn append_user(
        &self,
        text: impl Into<String>,
    ) -> Result<crate::model::SessionItem, SessionError> {
        self.append(ItemDraft::user(text)).await
    }

    pub async fn append_assistant(
        &self,
        text: impl Into<String>,
    ) -> Result<crate::model::SessionItem, SessionError> {
        self.append(ItemDraft::assistant(text)).await
    }

    pub async fn append_message(
        &self,
        message: Message,
    ) -> Result<crate::model::SessionItem, SessionError> {
        self.append(ItemDraft::message(message)).await
    }

    pub fn items(&self) -> ItemIter {
        ItemIter::new(self.hub.clone(), self.id.clone(), ItemQuery::all())
    }

    pub async fn protocol_messages(&self) -> Result<Vec<Message>, SessionError> {
        self.hub.protocol_messages(&self.id).await
    }

    pub async fn protocol_context(
        &self,
        policy: &ContextPolicy,
    ) -> Result<ContextView, SessionError> {
        self.hub.protocol_context(&self.id, policy).await
    }

    pub async fn archive(&self) -> Result<Conversation, SessionError> {
        self.hub
            .patch(
                &self.id,
                ConversationPatch {
                    status: Some(ConversationStatus::Archived),
                    ..ConversationPatch::default()
                },
            )
            .await
    }

    pub async fn delete(&self) -> Result<(), SessionError> {
        self.hub.delete(&self.id).await
    }

    pub async fn delete_items(&self, item_ids: &[ItemId]) -> Result<u64, SessionError> {
        self.hub.delete_items(&self.id, item_ids).await
    }

    pub async fn fork(&self) -> Result<SessionHandle, SessionError> {
        let conv = self.hub.fork(&self.id, None).await?;
        Ok(SessionHandle {
            hub: self.hub.clone(),
            id: conv.id,
        })
    }

    pub async fn fork_until(&self, seq: u64) -> Result<SessionHandle, SessionError> {
        let conv = self.hub.fork(&self.id, Some(seq)).await?;
        Ok(SessionHandle {
            hub: self.hub.clone(),
            id: conv.id,
        })
    }

    pub async fn fork_at(&self, item_id: &ItemId) -> Result<SessionHandle, SessionError> {
        let conv = self.hub.fork_at(&self.id, item_id).await?;
        Ok(SessionHandle {
            hub: self.hub.clone(),
            id: conv.id,
        })
    }

    pub async fn patch_item(
        &self,
        item_id: &ItemId,
        patch: ItemPatch,
    ) -> Result<crate::model::SessionItem, SessionError> {
        self.hub.patch_item(&self.id, item_id, patch).await
    }

    pub async fn compact(
        &self,
        until_seq: u64,
        marker: ItemDraft,
    ) -> Result<crate::model::SessionItem, SessionError> {
        let out = self.hub.compact(&self.id, until_seq, marker).await?;
        out.items
            .into_iter()
            .next()
            .ok_or_else(|| SessionError::invalid("压缩未返回标记条目"))
    }

    pub fn begin_run(&self) -> RunBuilder {
        RunBuilder {
            hub: self.hub.clone(),
            conversation_id: self.id.clone(),
            spec: RunSpec::default(),
            error: None,
        }
    }

    pub async fn open_run(&self) -> Result<Option<RunHandle>, SessionError> {
        Ok(self.hub.open_run(&self.id).await?.map(|run| RunHandle {
            hub: self.hub.clone(),
            run_id: run.id,
        }))
    }

    pub async fn export(&self) -> Result<SessionSnapshot, SessionError> {
        self.hub.export(&self.id).await
    }

    pub async fn render(&self, spec: &RenderViewSpec) -> Result<RenderSnapshot, SessionError> {
        self.hub.render().load(&self.id, spec).await
    }
}

/// 单会话标签操作。
pub struct TagOps {
    hub: SessionHub,
    id: ConversationId,
}

impl TagOps {
    pub async fn add(
        self,
        tags: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Conversation, SessionError> {
        let tags = TagSet::from_list(tags)?.as_slice_owned();
        self.hub
            .patch(
                &self.id,
                ConversationPatch {
                    tag_op: Some(TagOp::Add(tags)),
                    ..ConversationPatch::default()
                },
            )
            .await
    }

    pub async fn remove(
        self,
        tags: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Conversation, SessionError> {
        let tags = TagSet::from_list(tags)?.as_slice_owned();
        self.hub
            .patch(
                &self.id,
                ConversationPatch {
                    tag_op: Some(TagOp::Remove(tags)),
                    ..ConversationPatch::default()
                },
            )
            .await
    }

    pub async fn set(
        self,
        tags: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Conversation, SessionError> {
        let tags = TagSet::from_list(tags)?.as_slice_owned();
        self.hub
            .patch(
                &self.id,
                ConversationPatch {
                    tag_op: Some(TagOp::Set(tags)),
                    ..ConversationPatch::default()
                },
            )
            .await
    }

    pub async fn list(&self) -> Result<TagSet, SessionError> {
        Ok(self.hub.get(&self.id).await?.tags)
    }
}

/// 会话级动态引用。
pub struct RefOps {
    hub: SessionHub,
    id: ConversationId,
}

impl RefOps {
    pub async fn bind(
        self,
        slot: impl Into<String>,
        refer: ExtRef,
    ) -> Result<Conversation, SessionError> {
        self.hub
            .patch(
                &self.id,
                ConversationPatch {
                    refs: Some(RefOp::Bind {
                        slot: slot.into(),
                        refer,
                    }),
                    ..ConversationPatch::default()
                },
            )
            .await
    }

    pub async fn unbind(self, slot: impl Into<String>) -> Result<Conversation, SessionError> {
        self.hub
            .patch(
                &self.id,
                ConversationPatch {
                    refs: Some(RefOp::Unbind { slot: slot.into() }),
                    ..ConversationPatch::default()
                },
            )
            .await
    }
}

/// 开启回合构建器。
pub struct RunBuilder {
    hub: SessionHub,
    conversation_id: ConversationId,
    spec: RunSpec,
    error: Option<SessionError>,
}

impl RunBuilder {
    pub fn pending(mut self) -> Self {
        self.spec.status = RunStatus::Pending;
        self
    }

    pub fn group(mut self, group_id: impl Into<String>) -> Self {
        self.spec.group_id = Some(group_id.into());
        self
    }

    pub fn bind(mut self, slot: impl Into<String>, refer: ExtRef) -> Self {
        if self.error.is_some() {
            return self;
        }
        if let Err(err) = self.spec.refs.bind(slot, refer) {
            self.error = Some(err);
        }
        self
    }

    pub async fn open(self) -> Result<RunHandle, SessionError> {
        if let Some(err) = self.error {
            return Err(err);
        }
        let run = self.hub.begin_run(&self.conversation_id, self.spec).await?;
        Ok(RunHandle {
            hub: self.hub,
            run_id: run.id,
        })
    }
}

/// 回合句柄。
#[derive(Clone)]
pub struct RunHandle {
    hub: SessionHub,
    run_id: RunId,
}

impl RunHandle {
    pub fn id(&self) -> &RunId {
        &self.run_id
    }

    pub async fn get(&self) -> Result<Run, SessionError> {
        self.hub.get_run(&self.run_id).await
    }

    pub async fn suspend(&self) -> Result<Run, SessionError> {
        self.hub
            .patch_run(
                &self.run_id,
                RunPatch {
                    status: Some(RunStatus::Suspended),
                    ..RunPatch::default()
                },
            )
            .await
    }

    pub async fn resume(&self) -> Result<Run, SessionError> {
        self.hub
            .patch_run(
                &self.run_id,
                RunPatch {
                    status: Some(RunStatus::Active),
                    ..RunPatch::default()
                },
            )
            .await
    }

    pub async fn complete(&self) -> Result<Run, SessionError> {
        self.hub
            .patch_run(
                &self.run_id,
                RunPatch {
                    status: Some(RunStatus::Completed),
                    ..RunPatch::default()
                },
            )
            .await
    }

    pub async fn fail(&self, error: impl Into<String>) -> Result<Run, SessionError> {
        self.hub
            .patch_run(
                &self.run_id,
                RunPatch {
                    status: Some(RunStatus::Failed),
                    error: Some(error.into()),
                    ..RunPatch::default()
                },
            )
            .await
    }

    pub async fn cancel(&self) -> Result<Run, SessionError> {
        self.hub
            .patch_run(
                &self.run_id,
                RunPatch {
                    status: Some(RunStatus::Cancelled),
                    ..RunPatch::default()
                },
            )
            .await
    }

    pub async fn checkpoint(&self, value: Value) -> Result<Run, SessionError> {
        self.hub
            .patch_run(
                &self.run_id,
                RunPatch {
                    checkpoint: Some(value),
                    ..RunPatch::default()
                },
            )
            .await
    }
}

/// 批量选择。
pub struct SessionSelect {
    hub: SessionHub,
    query: ConversationQuery,
    unbounded: bool,
}

impl SessionSelect {
    pub fn ids(mut self, ids: impl IntoIterator<Item = ConversationId>) -> Self {
        self.query.ids = Some(ids.into_iter().collect());
        self
    }

    pub fn tagged(mut self, filter: TagFilter) -> Self {
        self.query.tags = Some(filter);
        self
    }

    pub fn status(mut self, status: ConversationStatus) -> Self {
        self.query.status = Some(status);
        self
    }

    pub fn title_contains(mut self, needle: impl Into<String>) -> Self {
        self.query.title_contains = Some(needle.into());
        self
    }

    pub fn content_contains(mut self, needle: impl Into<String>) -> Self {
        self.query.content_contains = Some(needle.into());
        self
    }

    pub fn created_from(mut self, ts: u64) -> Self {
        self.query.created_from = Some(ts);
        self
    }

    pub fn created_to(mut self, ts: u64) -> Self {
        self.query.created_to = Some(ts);
        self
    }

    /// 允许无过滤条件的全量变更。
    pub fn all(mut self) -> Self {
        self.unbounded = true;
        self
    }

    pub fn iter(self) -> ConversationIter {
        let mut query = self.query;
        if query.limit.is_none() {
            query.limit = Some(50);
        }
        ConversationIter::new(self.hub, query)
    }

    pub async fn list(&self) -> Result<Vec<Conversation>, SessionError> {
        let mut query = self.query.clone();
        if query.limit.is_none() {
            query.limit = Some(50);
        }
        Ok(self.hub.query(query).await?.items)
    }

    pub async fn count(&self) -> Result<u64, SessionError> {
        let query = ConversationQuery {
            limit: None,
            after: None,
            ..self.query.clone()
        };
        Ok(self.hub.query(query).await?.items.len() as u64)
    }

    pub async fn read(
        &self,
        item_query: ItemQuery,
    ) -> Result<Vec<ConversationBundle>, SessionError> {
        let ids = self.resolve_ids().await?;
        self.hub.read_many(&ids, item_query).await
    }

    pub async fn delete(&self) -> Result<u64, SessionError> {
        let ids = self.resolve_ids().await?;
        self.hub.delete_many(&ids).await
    }

    pub async fn archive(&self) -> Result<u64, SessionError> {
        let ids = self.resolve_ids().await?;
        let mut n = 0u64;
        for id in ids {
            self.hub
                .patch(
                    &id,
                    ConversationPatch {
                        status: Some(ConversationStatus::Archived),
                        ..ConversationPatch::default()
                    },
                )
                .await?;
            n += 1;
        }
        Ok(n)
    }

    pub async fn tag(&self, op: TagOp) -> Result<u64, SessionError> {
        let ids = self.resolve_ids().await?;
        let mut n = 0u64;
        for id in ids {
            self.hub
                .patch(
                    &id,
                    ConversationPatch {
                        tag_op: Some(op.clone()),
                        ..ConversationPatch::default()
                    },
                )
                .await?;
            n += 1;
        }
        Ok(n)
    }

    async fn resolve_ids(&self) -> Result<Vec<ConversationId>, SessionError> {
        let has_filter = self.query.has_constraint();
        if !has_filter && !self.unbounded {
            return Err(SessionError::invalid("批量变更需要过滤条件或显式 all()"));
        }
        let query = ConversationQuery {
            limit: None,
            after: None,
            ..self.query.clone()
        };
        self.hub.matching_ids(&query).await
    }
}

/// 标签目录。
pub struct TagCatalog {
    hub: SessionHub,
}

impl TagCatalog {
    pub async fn list(&self) -> Result<Vec<TagStat>, SessionError> {
        self.hub.list_tags(TagScope::Conversation).await
    }

    pub async fn list_item_tags(&self) -> Result<Vec<TagStat>, SessionError> {
        self.hub.list_tags(TagScope::Item).await
    }

    pub fn conversations(&self, filter: TagFilter) -> SessionSelect {
        self.hub.select().tagged(filter)
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<u64, SessionError> {
        self.hub.rename_tag(from, to).await
    }
}
