use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::error::SessionError;
use crate::ext::merge_ext;
use crate::model::{
    AppendOutcome, Conversation, ConversationId, ConversationPatch, ConversationStatus, ItemDraft,
    ItemId, ItemPatch, SessionItem, TagScope, TagSet, TagStat, now_ms,
};
use crate::query::{ConversationQuery, ItemQuery, Page, SortOrder};
use crate::run::{Run, RunId, RunPatch, RunQuery, RunStatus};
use crate::snapshot::SessionSnapshot;

/// 会话存储端口。上层只经 [`crate::SessionHub`] 使用；实现可替换。
/// 新增方法带默认实现（返回 Unsupported），旧实现仍可编译。
#[async_trait]
pub trait ConversationStore: Send + Sync {
    async fn upsert_conversation(&self, conv: &Conversation) -> Result<(), SessionError>;

    async fn get_conversation(
        &self,
        id: &ConversationId,
    ) -> Result<Option<Conversation>, SessionError>;

    async fn patch_conversation(
        &self,
        id: &ConversationId,
        patch: &ConversationPatch,
    ) -> Result<Conversation, SessionError>;

    async fn delete_conversations(&self, ids: &[ConversationId]) -> Result<u64, SessionError>;

    async fn query_conversations(
        &self,
        query: &ConversationQuery,
    ) -> Result<Page<Conversation>, SessionError>;

    async fn append_items(
        &self,
        id: &ConversationId,
        drafts: &[ItemDraft],
    ) -> Result<AppendOutcome, SessionError>;

    async fn query_items(
        &self,
        id: &ConversationId,
        query: &ItemQuery,
    ) -> Result<Page<SessionItem>, SessionError>;

    async fn delete_items(
        &self,
        id: &ConversationId,
        item_ids: &[ItemId],
    ) -> Result<u64, SessionError>;

    async fn list_tags(&self, scope: TagScope) -> Result<Vec<TagStat>, SessionError>;

    async fn fork(
        &self,
        id: &ConversationId,
        until_seq: Option<u64>,
    ) -> Result<AppendOutcome, SessionError>;

    async fn patch_item(
        &self,
        _id: &ConversationId,
        _item_id: &ItemId,
        _patch: &ItemPatch,
    ) -> Result<SessionItem, SessionError> {
        Err(SessionError::unsupported("patch_item"))
    }

    async fn upsert_run(&self, _run: &Run) -> Result<(), SessionError> {
        Err(SessionError::unsupported("upsert_run"))
    }

    async fn get_run(&self, _id: &RunId) -> Result<Option<Run>, SessionError> {
        Err(SessionError::unsupported("get_run"))
    }

    async fn patch_run(&self, _id: &RunId, _patch: &RunPatch) -> Result<Run, SessionError> {
        Err(SessionError::unsupported("patch_run"))
    }

    async fn query_runs(&self, _query: &RunQuery) -> Result<Page<Run>, SessionError> {
        Err(SessionError::unsupported("query_runs"))
    }

    async fn open_run(
        &self,
        _conversation_id: &ConversationId,
    ) -> Result<Option<Run>, SessionError> {
        Err(SessionError::unsupported("open_run"))
    }

    async fn restore_snapshot(&self, _snap: &SessionSnapshot) -> Result<(), SessionError> {
        Err(SessionError::unsupported("restore_snapshot"))
    }
}

struct MemoryInner {
    convs: HashMap<ConversationId, Conversation>,
    items: HashMap<ConversationId, Vec<SessionItem>>,
    runs: HashMap<RunId, Run>,
    runs_by_conv: HashMap<ConversationId, Vec<RunId>>,
}

/// 进程内存储。由 [`crate::SessionHub::memory`] 装配，不作为上层入口。
pub struct MemoryStore {
    inner: Arc<RwLock<MemoryInner>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(MemoryInner {
                convs: HashMap::new(),
                items: HashMap::new(),
                runs: HashMap::new(),
                runs_by_conv: HashMap::new(),
            })),
        }
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

fn sort_convs(items: &mut [Conversation], order: SortOrder) {
    items.sort_by(|a, b| match order {
        SortOrder::UpdatedDesc => b
            .updated_at
            .cmp(&a.updated_at)
            .then_with(|| b.id.to_string().cmp(&a.id.to_string())),
        SortOrder::CreatedDesc => b
            .created_at
            .cmp(&a.created_at)
            .then_with(|| b.id.to_string().cmp(&a.id.to_string())),
        SortOrder::TitleAsc => {
            let ta = a.title.as_deref().unwrap_or("");
            let tb = b.title.as_deref().unwrap_or("");
            ta.cmp(tb)
                .then_with(|| a.id.to_string().cmp(&b.id.to_string()))
        }
    });
}

fn in_range(value: u64, from: Option<u64>, to: Option<u64>) -> bool {
    if let Some(f) = from {
        if value < f {
            return false;
        }
    }
    if let Some(t) = to {
        if value > t {
            return false;
        }
    }
    true
}

fn item_matches(item: &SessionItem, query: &ItemQuery) -> bool {
    if let Some(filter) = &query.tags {
        if !filter.matches(&item.tags) {
            return false;
        }
    }
    if let Some(kinds) = &query.kinds {
        if !kinds.contains(&item.kind()) {
            return false;
        }
    }
    if let Some(g) = &query.group_id {
        if item.group_id.as_deref() != Some(g.as_str()) {
            return false;
        }
    }
    if let Some(k) = &query.ext_kind {
        if item.body.ext_kind() != Some(k.as_str()) {
            return false;
        }
    }
    if let Some(needle) = &query.text_contains {
        if !item.body.as_text().contains(needle) {
            return false;
        }
    }
    if let Some(pinned) = query.pinned {
        if item.pinned != pinned {
            return false;
        }
    }
    if let Some(after) = query.after_seq {
        if query.reverse {
            if item.seq >= after {
                return false;
            }
        } else if item.seq <= after {
            return false;
        }
    }
    if let Some(before) = query.before_seq {
        if item.seq > before {
            return false;
        }
    }
    true
}

fn bump_revision(conv: &mut Conversation) {
    conv.updated_at = now_ms();
    conv.revision = conv.revision.saturating_add(1);
}

#[async_trait]
impl ConversationStore for MemoryStore {
    async fn upsert_conversation(&self, conv: &Conversation) -> Result<(), SessionError> {
        let mut inner = self.inner.write().await;
        inner.convs.insert(conv.id.clone(), conv.clone());
        inner.items.entry(conv.id.clone()).or_default();
        Ok(())
    }

    async fn get_conversation(
        &self,
        id: &ConversationId,
    ) -> Result<Option<Conversation>, SessionError> {
        let inner = self.inner.read().await;
        Ok(inner.convs.get(id).cloned())
    }

    async fn patch_conversation(
        &self,
        id: &ConversationId,
        patch: &ConversationPatch,
    ) -> Result<Conversation, SessionError> {
        let mut inner = self.inner.write().await;
        let conv = inner
            .convs
            .get_mut(id)
            .ok_or_else(|| SessionError::not_found(id))?;
        if let Some(title) = &patch.title {
            conv.title = if title.is_empty() {
                None
            } else {
                Some(title.clone())
            };
        }
        if let Some(status) = patch.status {
            conv.status = status;
        }
        if let Some(op) = &patch.tag_op {
            conv.tags.apply(op)?;
        }
        if let Some(op) = &patch.refs {
            conv.refs.apply(op)?;
        }
        if let Some(m) = &patch.metrics {
            conv.metrics.merge(m);
        }
        if let Some(e) = &patch.ext {
            merge_ext(&mut conv.ext, e);
        }
        if let Some(c) = patch.compact_until {
            conv.compact_until = Some(c);
        }
        if let Some(p) = &patch.parent_item_id {
            conv.parent_item_id = Some(p.clone());
        }
        bump_revision(conv);
        Ok(conv.clone())
    }

    async fn delete_conversations(&self, ids: &[ConversationId]) -> Result<u64, SessionError> {
        let mut inner = self.inner.write().await;
        let mut n = 0u64;
        for id in ids {
            if inner.convs.remove(id).is_some() {
                inner.items.remove(id);
                if let Some(run_ids) = inner.runs_by_conv.remove(id) {
                    for rid in run_ids {
                        inner.runs.remove(&rid);
                    }
                }
                n += 1;
            }
        }
        Ok(n)
    }

    async fn query_conversations(
        &self,
        query: &ConversationQuery,
    ) -> Result<Page<Conversation>, SessionError> {
        let inner = self.inner.read().await;
        let mut matched: Vec<Conversation> = if let Some(ids) = &query.ids {
            ids.iter()
                .filter_map(|id| inner.convs.get(id).cloned())
                .collect()
        } else {
            inner.convs.values().cloned().collect()
        };

        matched.retain(|c| {
            if let Some(status) = query.status {
                if c.status != status {
                    return false;
                }
            }
            if let Some(filter) = &query.tags {
                if !filter.matches(&c.tags) {
                    return false;
                }
            }
            if let Some(needle) = &query.title_contains {
                let title = c.title.as_deref().unwrap_or("");
                if !title.contains(needle) {
                    return false;
                }
            }
            if !in_range(c.created_at, query.created_from, query.created_to) {
                return false;
            }
            if !in_range(c.updated_at, query.updated_from, query.updated_to) {
                return false;
            }
            if let Some(kind) = &query.ref_kind {
                if !c.refs.has_kind(kind) {
                    return false;
                }
            }
            if let Some(slot) = &query.ref_slot {
                if c.refs.get(slot).is_none() {
                    return false;
                }
            }
            if let Some(needle) = &query.content_contains {
                let title_hit = c.title.as_deref().is_some_and(|t| t.contains(needle));
                let item_hit = inner
                    .items
                    .get(&c.id)
                    .into_iter()
                    .flatten()
                    .any(|i| i.body.as_text().contains(needle));
                if !title_hit && !item_hit {
                    return false;
                }
            }
            true
        });

        sort_convs(&mut matched, query.order);

        if let Some(after) = &query.after {
            if let Some(pos) = matched.iter().position(|c| c.id.to_string() == *after) {
                matched = matched.split_off(pos + 1);
            }
        }

        let limit = query.limit.map(|n| n as usize);
        let next = if let Some(lim) = limit {
            if matched.len() > lim {
                let next_cursor = matched.get(lim.saturating_sub(1)).map(|c| c.id.to_string());
                matched.truncate(lim);
                next_cursor
            } else {
                None
            }
        } else {
            None
        };

        Ok(Page {
            items: matched,
            next,
        })
    }

    async fn append_items(
        &self,
        id: &ConversationId,
        drafts: &[ItemDraft],
    ) -> Result<AppendOutcome, SessionError> {
        if drafts.is_empty() {
            return Err(SessionError::invalid("追加条目为空"));
        }
        let mut inner = self.inner.write().await;
        if !inner.convs.contains_key(id) {
            return Err(SessionError::not_found(id));
        }
        let now = now_ms();
        let (appended, created, item_count) = {
            let existing = inner.items.entry(id.clone()).or_default();
            let mut seq = existing.last().map(|i| i.seq).unwrap_or(0);
            let mut appended = Vec::new();
            let mut created = 0usize;
            for draft in drafts {
                if let Some(cid) = &draft.client_id {
                    if let Some(found) = existing
                        .iter()
                        .find(|i| i.client_id.as_deref() == Some(cid))
                    {
                        if found.body != draft.body {
                            return Err(SessionError::conflict(format!("client_id 冲突: {cid}")));
                        }
                        appended.push(found.clone());
                        continue;
                    }
                }
                seq += 1;
                let item = SessionItem::from_draft(draft.clone(), seq, now);
                existing.push(item.clone());
                appended.push(item);
                created += 1;
            }
            let item_count = existing.len() as u64;
            (appended, created, item_count)
        };
        let conv = inner
            .convs
            .get_mut(id)
            .ok_or_else(|| SessionError::not_found(id))?;
        conv.item_count = item_count;
        if created > 0 {
            bump_revision(conv);
        }
        Ok(AppendOutcome {
            conversation: conv.clone(),
            items: appended,
            created,
        })
    }

    async fn query_items(
        &self,
        id: &ConversationId,
        query: &ItemQuery,
    ) -> Result<Page<SessionItem>, SessionError> {
        let inner = self.inner.read().await;
        if !inner.convs.contains_key(id) {
            return Err(SessionError::not_found(id));
        }
        let mut matched: Vec<SessionItem> = inner
            .items
            .get(id)
            .into_iter()
            .flatten()
            .filter(|item| item_matches(item, query))
            .cloned()
            .collect();

        if query.reverse {
            matched.sort_by(|a, b| b.seq.cmp(&a.seq));
        } else {
            matched.sort_by(|a, b| a.seq.cmp(&b.seq));
        }

        let limit = query.limit.map(|n| n as usize);
        let next = if let Some(lim) = limit {
            if matched.len() > lim {
                let next_seq = matched
                    .get(lim.saturating_sub(1))
                    .map(|i| i.seq.to_string());
                matched.truncate(lim);
                next_seq
            } else {
                None
            }
        } else {
            None
        };

        Ok(Page {
            items: matched,
            next,
        })
    }

    async fn delete_items(
        &self,
        id: &ConversationId,
        item_ids: &[ItemId],
    ) -> Result<u64, SessionError> {
        let mut inner = self.inner.write().await;
        if !inner.convs.contains_key(id) {
            return Err(SessionError::not_found(id));
        }
        let (removed, item_count) = {
            let items = inner.items.entry(id.clone()).or_default();
            let before = items.len();
            items.retain(|i| !item_ids.contains(&i.id));
            let item_count = items.len() as u64;
            ((before as u64) - item_count, item_count)
        };
        if removed > 0 {
            let conv = inner
                .convs
                .get_mut(id)
                .ok_or_else(|| SessionError::not_found(id))?;
            conv.item_count = item_count;
            bump_revision(conv);
        }
        Ok(removed)
    }

    async fn list_tags(&self, scope: TagScope) -> Result<Vec<TagStat>, SessionError> {
        let inner = self.inner.read().await;
        let mut counts: HashMap<String, u64> = HashMap::new();
        match scope {
            TagScope::Conversation => {
                for conv in inner.convs.values() {
                    bump_tags(&mut counts, &conv.tags);
                }
            }
            TagScope::Item => {
                for items in inner.items.values() {
                    for item in items {
                        bump_tags(&mut counts, &item.tags);
                    }
                }
            }
        }
        let mut stats: Vec<TagStat> = counts
            .into_iter()
            .map(|(tag, count)| TagStat { tag, count })
            .collect();
        stats.sort_by(|a, b| a.tag.cmp(&b.tag));
        Ok(stats)
    }

    async fn fork(
        &self,
        id: &ConversationId,
        until_seq: Option<u64>,
    ) -> Result<AppendOutcome, SessionError> {
        self.fork_at(id, until_seq, None).await
    }

    async fn patch_item(
        &self,
        id: &ConversationId,
        item_id: &ItemId,
        patch: &ItemPatch,
    ) -> Result<SessionItem, SessionError> {
        let mut inner = self.inner.write().await;
        let cloned = {
            let items = inner
                .items
                .get_mut(id)
                .ok_or_else(|| SessionError::not_found(id))?;
            let item = items
                .iter_mut()
                .find(|i| &i.id == item_id)
                .ok_or_else(|| SessionError::not_found(item_id))?;
            patch.apply(item)?;
            item.clone()
        };
        let conv = inner
            .convs
            .get_mut(id)
            .ok_or_else(|| SessionError::not_found(id))?;
        bump_revision(conv);
        Ok(cloned)
    }

    async fn upsert_run(&self, run: &Run) -> Result<(), SessionError> {
        let mut inner = self.inner.write().await;
        if !inner.convs.contains_key(&run.conversation_id) {
            return Err(SessionError::not_found(&run.conversation_id));
        }
        inner.runs.insert(run.id.clone(), run.clone());
        let list = inner
            .runs_by_conv
            .entry(run.conversation_id.clone())
            .or_default();
        if !list.contains(&run.id) {
            list.push(run.id.clone());
        }
        Ok(())
    }

    async fn get_run(&self, id: &RunId) -> Result<Option<Run>, SessionError> {
        let inner = self.inner.read().await;
        Ok(inner.runs.get(id).cloned())
    }

    async fn patch_run(&self, id: &RunId, patch: &RunPatch) -> Result<Run, SessionError> {
        let mut inner = self.inner.write().await;
        let run = inner
            .runs
            .get_mut(id)
            .ok_or_else(|| SessionError::not_found(id))?;
        if let Some(status) = patch.status {
            if !run.status.can_transit(status) {
                return Err(SessionError::conflict(format!(
                    "回合状态不可从 {:?} 转到 {:?}",
                    run.status, status
                )));
            }
            let now = now_ms();
            if status == RunStatus::Active && run.started_at.is_none() {
                run.started_at = Some(now);
            }
            if !status.is_open() {
                run.ended_at = Some(now);
            }
            run.status = status;
        }
        if let Some(g) = &patch.group_id {
            run.group_id = if g.is_empty() { None } else { Some(g.clone()) };
        }
        if let Some(op) = &patch.refs {
            run.refs.apply(op)?;
        }
        if let Some(m) = &patch.metrics {
            run.metrics.merge(m);
        }
        if let Some(cp) = &patch.checkpoint {
            run.checkpoint = Some(cp.clone());
        }
        if let Some(e) = &patch.error {
            run.error = Some(e.clone());
        }
        if let Some(ext) = &patch.ext {
            merge_ext(&mut run.ext, ext);
        }
        run.updated_at = now_ms();
        Ok(run.clone())
    }

    async fn query_runs(&self, query: &RunQuery) -> Result<Page<Run>, SessionError> {
        let inner = self.inner.read().await;
        let mut matched: Vec<Run> = if let Some(cid) = &query.conversation_id {
            query_runs_of(&inner, cid)
        } else {
            inner.runs.values().cloned().collect()
        };
        matched.retain(|r| {
            if let Some(st) = query.status {
                if r.status != st {
                    return false;
                }
            }
            if let Some(g) = &query.group_id {
                if r.group_id.as_deref() != Some(g.as_str()) {
                    return false;
                }
            }
            true
        });
        matched.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        if let Some(lim) = query.limit {
            matched.truncate(lim as usize);
        }
        Ok(Page {
            items: matched,
            next: None,
        })
    }

    async fn open_run(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Option<Run>, SessionError> {
        let inner = self.inner.read().await;
        Ok(query_runs_of(&inner, conversation_id)
            .into_iter()
            .find(|r| r.status.is_open()))
    }

    async fn restore_snapshot(&self, snap: &SessionSnapshot) -> Result<(), SessionError> {
        let mut inner = self.inner.write().await;
        let id = &snap.conversation.id;
        if inner.convs.contains_key(id) {
            return Err(SessionError::conflict(format!("会话已存在: {id}")));
        }
        inner.convs.insert(id.clone(), snap.conversation.clone());
        inner.items.insert(id.clone(), snap.items.clone());
        let mut run_ids = Vec::new();
        for run in &snap.runs {
            inner.runs.insert(run.id.clone(), run.clone());
            run_ids.push(run.id.clone());
        }
        inner.runs_by_conv.insert(id.clone(), run_ids);
        Ok(())
    }
}

impl MemoryStore {
    async fn fork_at(
        &self,
        id: &ConversationId,
        until_seq: Option<u64>,
        parent_item_id: Option<ItemId>,
    ) -> Result<AppendOutcome, SessionError> {
        let mut inner = self.inner.write().await;
        let src = inner
            .convs
            .get(id)
            .cloned()
            .ok_or_else(|| SessionError::not_found(id))?;
        let src_items = inner.items.get(id).cloned().unwrap_or_default();
        let now = now_ms();
        let new_id = ConversationId::new();
        let filtered: Vec<SessionItem> = src_items
            .into_iter()
            .filter(|item| until_seq.map(|seq| item.seq <= seq).unwrap_or(true))
            .collect();
        let mut id_map: HashMap<ItemId, ItemId> = HashMap::new();
        for item in &filtered {
            id_map.insert(item.id.clone(), ItemId::new());
        }
        let copied: Vec<SessionItem> = filtered
            .into_iter()
            .enumerate()
            .map(|(idx, item)| {
                let new_item_id = id_map.get(&item.id).cloned().unwrap_or_else(ItemId::new);
                let parent = item.parent_item.and_then(|p| id_map.get(&p).cloned());
                SessionItem {
                    id: new_item_id,
                    seq: (idx as u64) + 1,
                    body: item.body,
                    tags: item.tags,
                    created_at: now,
                    updated_at: now,
                    protocol_visible: item.protocol_visible,
                    pinned: item.pinned,
                    group_id: item.group_id,
                    client_id: None,
                    parent_item: parent,
                    refs: item.refs,
                    metrics: crate::ext::MetricMap::new(),
                    ext: item.ext,
                }
            })
            .collect();
        let compact_until = match (src.compact_until, until_seq) {
            (Some(c), Some(u)) if c <= u => Some(c),
            (Some(_), Some(_)) => None,
            (c, None) => c,
            (None, _) => None,
        };
        let conv = Conversation {
            id: new_id.clone(),
            title: src.title,
            status: ConversationStatus::Active,
            tags: src.tags,
            created_at: now,
            updated_at: now,
            item_count: copied.len() as u64,
            revision: 1,
            parent_id: Some(id.clone()),
            parent_item_id,
            compact_until,
            refs: src.refs,
            metrics: crate::ext::MetricMap::new(),
            ext: src.ext,
        };
        inner.convs.insert(new_id.clone(), conv.clone());
        inner.items.insert(new_id, copied.clone());
        Ok(AppendOutcome {
            conversation: conv,
            items: copied,
            created: 0,
        })
    }
}

fn query_runs_of(inner: &MemoryInner, cid: &ConversationId) -> Vec<Run> {
    inner
        .runs_by_conv
        .get(cid)
        .into_iter()
        .flatten()
        .filter_map(|rid| inner.runs.get(rid).cloned())
        .collect()
}

fn bump_tags(counts: &mut HashMap<String, u64>, tags: &TagSet) {
    for tag in tags.iter() {
        *counts.entry(tag.to_string()).or_insert(0) += 1;
    }
}
