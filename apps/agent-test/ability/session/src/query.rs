use serde::{Deserialize, Serialize};

use crate::model::{ConversationId, ConversationStatus, ItemKind, TagSet};

/// 标签过滤（可组合）。默认多标签为全匹配。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagFilter {
    /// 同时具备全部标签。
    All(Vec<String>),
    /// 具备任一标签。
    Any(Vec<String>),
    /// 不具备所列任一标签。
    None(Vec<String>),
    And(Vec<TagFilter>),
    Or(Vec<TagFilter>),
    Not(Box<TagFilter>),
}

impl TagFilter {
    pub fn all(tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::All(tags.into_iter().map(Into::into).collect())
    }

    pub fn any(tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::Any(tags.into_iter().map(Into::into).collect())
    }

    pub fn none(tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::None(tags.into_iter().map(Into::into).collect())
    }

    pub fn matches(&self, tags: &TagSet) -> bool {
        match self {
            Self::All(required) => required.iter().all(|t| tags.contains(t)),
            Self::Any(wanted) => wanted.iter().any(|t| tags.contains(t)),
            Self::None(forbidden) => forbidden.iter().all(|t| !tags.contains(t)),
            Self::And(parts) => parts.iter().all(|p| p.matches(tags)),
            Self::Or(parts) => parts.iter().any(|p| p.matches(tags)),
            Self::Not(inner) => !inner.matches(tags),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    #[default]
    UpdatedDesc,
    CreatedDesc,
    TitleAsc,
}

/// 会话查询。`limit = None` 表示不过页（供批量变更）。
#[derive(Clone, Debug, Default)]
pub struct ConversationQuery {
    pub ids: Option<Vec<ConversationId>>,
    pub tags: Option<TagFilter>,
    pub status: Option<ConversationStatus>,
    pub title_contains: Option<String>,
    pub content_contains: Option<String>,
    pub created_from: Option<u64>,
    pub created_to: Option<u64>,
    pub updated_from: Option<u64>,
    pub updated_to: Option<u64>,
    pub ref_kind: Option<String>,
    pub ref_slot: Option<String>,
    pub after: Option<String>,
    pub limit: Option<u32>,
    pub order: SortOrder,
}

impl ConversationQuery {
    pub fn clamped_limit(&self) -> Option<u32> {
        self.limit.map(|n| n.clamp(1, 500))
    }

    pub fn has_constraint(&self) -> bool {
        self.ids.is_some()
            || self.tags.is_some()
            || self.status.is_some()
            || self.title_contains.is_some()
            || self.content_contains.is_some()
            || self.created_from.is_some()
            || self.created_to.is_some()
            || self.updated_from.is_some()
            || self.updated_to.is_some()
            || self.ref_kind.is_some()
            || self.ref_slot.is_some()
    }
}

/// 条目查询。
#[derive(Clone, Debug, Default)]
pub struct ItemQuery {
    pub tags: Option<TagFilter>,
    pub kinds: Option<Vec<ItemKind>>,
    pub group_id: Option<String>,
    pub ext_kind: Option<String>,
    pub text_contains: Option<String>,
    pub pinned: Option<bool>,
    pub after_seq: Option<u64>,
    pub before_seq: Option<u64>,
    pub limit: Option<u32>,
    pub reverse: bool,
}

impl ItemQuery {
    pub fn latest(limit: u32) -> Self {
        Self {
            limit: Some(limit),
            reverse: true,
            ..Self::default()
        }
    }

    pub fn all() -> Self {
        Self::default()
    }

    pub fn clamped_limit(&self) -> Option<u32> {
        self.limit.map(|n| n.clamp(1, 500))
    }
}

#[derive(Clone, Debug, Default)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next: Option<String>,
}

impl<T> Page<T> {
    pub fn empty() -> Self {
        Self {
            items: Vec::new(),
            next: None,
        }
    }
}
