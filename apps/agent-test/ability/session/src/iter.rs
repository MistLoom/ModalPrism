use std::collections::VecDeque;

use crate::error::SessionError;
use crate::hub::SessionHub;
use crate::model::{Conversation, ConversationId, ItemKind, SessionItem};
use crate::query::{ConversationQuery, ItemQuery, TagFilter};

/// 会话分页迭代器。
pub struct ConversationIter {
    hub: SessionHub,
    query: ConversationQuery,
    buffer: VecDeque<Conversation>,
    remaining: Option<u32>,
    page_size: u32,
    done: bool,
}

impl ConversationIter {
    pub(crate) fn new(hub: SessionHub, query: ConversationQuery) -> Self {
        let page_size = query.limit.unwrap_or(50).clamp(1, 500);
        Self {
            hub,
            query,
            buffer: VecDeque::new(),
            remaining: None,
            page_size,
            done: false,
        }
    }

    pub fn tagged(mut self, filter: TagFilter) -> Self {
        self.query.tags = Some(filter);
        self
    }

    pub fn take_n(mut self, n: u32) -> Self {
        self.remaining = Some(n);
        self
    }

    pub async fn next(&mut self) -> Result<Option<Conversation>, SessionError> {
        if matches!(self.remaining, Some(0)) {
            self.done = true;
            return Ok(None);
        }
        if self.buffer.is_empty() {
            self.fill().await?;
        }
        let item = self.buffer.pop_front();
        if item.is_some() {
            if let Some(left) = self.remaining.as_mut() {
                *left = left.saturating_sub(1);
            }
        }
        Ok(item)
    }

    pub async fn collect(mut self) -> Result<Vec<Conversation>, SessionError> {
        let mut out = Vec::new();
        while let Some(item) = self.next().await? {
            out.push(item);
        }
        Ok(out)
    }

    async fn fill(&mut self) -> Result<(), SessionError> {
        if self.done {
            return Ok(());
        }
        let want = self.remaining.unwrap_or(self.page_size).min(self.page_size);
        if want == 0 {
            self.done = true;
            return Ok(());
        }
        self.query.limit = Some(want);
        let page = self.hub.query(self.query.clone()).await?;
        if page.items.is_empty() {
            self.done = true;
            return Ok(());
        }
        self.query.after = page.items.last().map(|c| c.id.to_string());
        if page.next.is_none() {
            self.done = true;
        }
        self.buffer.extend(page.items);
        Ok(())
    }
}

/// 条目分页迭代器。
pub struct ItemIter {
    hub: SessionHub,
    id: ConversationId,
    query: ItemQuery,
    buffer: VecDeque<SessionItem>,
    remaining: Option<u32>,
    page_size: u32,
    done: bool,
}

impl ItemIter {
    pub(crate) fn new(hub: SessionHub, id: ConversationId, query: ItemQuery) -> Self {
        let page_size = query.limit.unwrap_or(50).clamp(1, 500);
        Self {
            hub,
            id,
            query,
            buffer: VecDeque::new(),
            remaining: None,
            page_size,
            done: false,
        }
    }

    pub fn tagged(mut self, filter: TagFilter) -> Self {
        self.query.tags = Some(filter);
        self
    }

    pub fn group(mut self, group_id: impl Into<String>) -> Self {
        self.query.group_id = Some(group_id.into());
        self
    }

    pub fn ext_kind(mut self, kind: impl Into<String>) -> Self {
        self.query.ext_kind = Some(kind.into());
        self
    }

    pub fn kinds(mut self, kinds: impl Into<Vec<ItemKind>>) -> Self {
        self.query.kinds = Some(kinds.into());
        self
    }

    pub fn reverse(mut self) -> Self {
        self.query.reverse = true;
        self
    }

    pub fn take_n(mut self, n: u32) -> Self {
        self.remaining = Some(n);
        self
    }

    pub async fn next(&mut self) -> Result<Option<SessionItem>, SessionError> {
        if matches!(self.remaining, Some(0)) {
            self.done = true;
            return Ok(None);
        }
        if self.buffer.is_empty() {
            self.fill().await?;
        }
        let item = self.buffer.pop_front();
        if item.is_some() {
            if let Some(left) = self.remaining.as_mut() {
                *left = left.saturating_sub(1);
            }
        }
        Ok(item)
    }

    pub async fn collect(mut self) -> Result<Vec<SessionItem>, SessionError> {
        let mut out = Vec::new();
        while let Some(item) = self.next().await? {
            out.push(item);
        }
        Ok(out)
    }

    async fn fill(&mut self) -> Result<(), SessionError> {
        if self.done {
            return Ok(());
        }
        let want = self.remaining.unwrap_or(self.page_size).min(self.page_size);
        if want == 0 {
            self.done = true;
            return Ok(());
        }
        self.query.limit = Some(want);
        let page = self.hub.items(&self.id, self.query.clone()).await?;
        if page.items.is_empty() {
            self.done = true;
            return Ok(());
        }
        self.query.after_seq = page.items.last().map(|i| i.seq);
        if page.next.is_none() {
            self.done = true;
        }
        self.buffer.extend(page.items);
        Ok(())
    }
}
