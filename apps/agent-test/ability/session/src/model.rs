use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use ai_protocol::{ContentPart, Message, MessageContent, Role};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::SessionError;
use crate::ext::{MetricMap, RefMap, RefOp, merge_ext, normalize_ident};

/// Unix 毫秒
pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 会话标识（AI 对话容器；与 `ability_support::SessionId` 工具状态互不通用）
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConversationId(uuid::Uuid);

impl ConversationId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    pub fn as_uuid(&self) -> uuid::Uuid {
        self.0
    }
}

impl Default for ConversationId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ConversationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ConversationId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(uuid::Uuid::parse_str(s)?))
    }
}

/// 会话条目标识
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ItemId(uuid::Uuid);

impl ItemId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for ItemId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ItemId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(uuid::Uuid::parse_str(s)?))
    }
}

/// 会话生命周期。删除为物理移除，不占用状态位
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationStatus {
    #[default]
    Active,
    Archived,
}

/// 条目投影类别（由正文推导，不单独落盘）
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    User,
    Assistant,
    System,
    Tool,
    Notice,
    Artifact,
    /// 扩展正文；具kind 由调用方定义
    Ext,
}

impl ItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::System => "system",
            Self::Tool => "tool",
            Self::Notice => "notice",
            Self::Artifact => "artifact",
            Self::Ext => "ext",
        }
    }
}

/// 有序标签集
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagSet(BTreeSet<String>);

impl TagSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_list(
        tags: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, SessionError> {
        let mut set = Self::new();
        for tag in tags {
            set.try_insert(tag)?;
        }
        Ok(set)
    }

    pub fn try_insert(&mut self, tag: impl Into<String>) -> Result<bool, SessionError> {
        let tag = normalize_tag(tag.into())?;
        Ok(self.0.insert(tag))
    }

    pub fn remove(&mut self, tag: &str) -> bool {
        self.0.remove(tag)
    }

    pub fn contains(&self, tag: &str) -> bool {
        self.0.contains(tag)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    pub fn as_slice_owned(&self) -> Vec<String> {
        self.0.iter().cloned().collect()
    }

    pub(crate) fn apply(&mut self, op: &TagOp) -> Result<bool, SessionError> {
        match op {
            TagOp::Add(tags) => {
                let mut changed = false;
                for tag in tags {
                    if self.try_insert(tag)? {
                        changed = true;
                    }
                }
                Ok(changed)
            }
            TagOp::Remove(tags) => {
                let mut changed = false;
                for tag in tags {
                    if self.remove(tag) {
                        changed = true;
                    }
                }
                Ok(changed)
            }
            TagOp::Set(tags) => {
                let next = TagSet::from_list(tags.iter().cloned())?;
                if next == *self {
                    Ok(false)
                } else {
                    *self = next;
                    Ok(true)
                }
            }
        }
    }
}

impl IntoIterator for TagSet {
    type Item = String;
    type IntoIter = std::collections::btree_set::IntoIter<String>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// 标签变更
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagOp {
    Add(Vec<String>),
    Remove(Vec<String>),
    Set(Vec<String>),
}

pub(crate) fn normalize_tag(raw: impl AsRef<str>) -> Result<String, SessionError> {
    normalize_ident(raw, "标签")
}

/// 会话容器
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: ConversationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: ConversationStatus,
    #[serde(default)]
    pub tags: TagSet,
    pub created_at: u64,
    pub updated_at: u64,
    pub item_count: u64,
    /// 内容修订号；渲染缓存按此失效
    pub revision: u64,
    /// 分叉来源；无则为主干
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<ConversationId>,
    /// 分叉源条目（源会话中的条id）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_item_id: Option<ItemId>,
    /// 已压缩水位：该序号及之前的非 pinned 条目默认不再进入协议
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_until: Option<u64>,
    #[serde(default)]
    pub refs: RefMap,
    #[serde(default)]
    pub metrics: MetricMap,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub ext: Map<String, Value>,
}

/// 会话条目正文。协议消息与仅会话可见内容分型，避免把渲染物塞进模型上下文
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ItemBody {
    Message {
        message: Message,
    },
    Notice {
        text: String,
    },
    Artifact {
        name: String,
        /// 媒体提示（如 mime）；由渲染层解释，本层不绑定前端技术栈
        media: String,
        text: String,
    },
    /// 扩展正文。`kind` 由调用方定义，会话层不解释
    Ext {
        kind: String,
        payload: Value,
    },
}

impl ItemBody {
    pub fn as_text(&self) -> String {
        match self {
            Self::Message { message } => message.content_text().unwrap_or_default(),
            Self::Notice { text } => text.clone(),
            Self::Artifact { name, text, .. } => {
                if text.is_empty() {
                    name.clone()
                } else {
                    text.clone()
                }
            }
            Self::Ext { payload, .. } => match payload {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            },
        }
    }

    pub fn title_hint(&self) -> Option<String> {
        match self {
            Self::Artifact { name, .. } => Some(name.clone()),
            Self::Notice { .. } => Some("notice".into()),
            Self::Ext { kind, .. } => Some(kind.clone()),
            Self::Message { .. } => None,
        }
    }

    pub fn ext_kind(&self) -> Option<&str> {
        match self {
            Self::Ext { kind, .. } => Some(kind.as_str()),
            _ => None,
        }
    }

    pub(crate) fn append_text(&mut self, extra: &str) -> Result<(), SessionError> {
        match self {
            Self::Message { message } => {
                match &mut message.content {
                    Some(MessageContent::Text(s)) => s.push_str(extra),
                    Some(MessageContent::Parts(parts)) => {
                        parts.push(ContentPart::Text {
                            text: extra.to_string(),
                        });
                    }
                    None => {
                        message.content = Some(MessageContent::Text(extra.to_string()));
                    }
                }
                Ok(())
            }
            Self::Notice { text } | Self::Artifact { text, .. } => {
                text.push_str(extra);
                Ok(())
            }
            Self::Ext { payload, .. } => {
                if let Value::String(s) = payload {
                    s.push_str(extra);
                    Ok(())
                } else {
                    Err(SessionError::invalid("扩展正文非字符串，无text_append"))
                }
            }
        }
    }
}

/// 已落盘条目
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionItem {
    pub id: ItemId,
    pub seq: u64,
    pub body: ItemBody,
    #[serde(default)]
    pub tags: TagSet,
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
    /// 是否进入协议投影
    pub protocol_visible: bool,
    /// 协议窗口中优先保留
    #[serde(default)]
    pub pinned: bool,
    /// 调用方分组键
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    /// 调用方幂等键
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_item: Option<ItemId>,
    #[serde(default)]
    pub refs: RefMap,
    #[serde(default)]
    pub metrics: MetricMap,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub ext: Map<String, Value>,
}

impl SessionItem {
    pub fn kind(&self) -> ItemKind {
        match &self.body {
            ItemBody::Notice { .. } => ItemKind::Notice,
            ItemBody::Artifact { .. } => ItemKind::Artifact,
            ItemBody::Ext { .. } => ItemKind::Ext,
            ItemBody::Message { message } => match message.role {
                Role::User => ItemKind::User,
                Role::Assistant => ItemKind::Assistant,
                Role::System | Role::Developer => ItemKind::System,
                Role::Tool => ItemKind::Tool,
            },
        }
    }

    pub fn as_protocol_message(&self) -> Option<&Message> {
        if !self.protocol_visible {
            return None;
        }
        match &self.body {
            ItemBody::Message { message } => Some(message),
            _ => None,
        }
    }

    pub(crate) fn from_draft(draft: ItemDraft, seq: u64, now: u64) -> Self {
        Self {
            id: ItemId::new(),
            seq,
            body: draft.body,
            tags: draft.tags,
            created_at: now,
            updated_at: now,
            protocol_visible: draft.protocol_visible,
            pinned: draft.pinned,
            group_id: draft.group_id,
            client_id: draft.client_id,
            parent_item: draft.parent_item,
            refs: draft.refs,
            metrics: draft.metrics,
            ext: draft.ext,
        }
    }
}

/// 追加草稿；序号与 id 由存储分配
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemDraft {
    pub body: ItemBody,
    #[serde(default)]
    pub tags: TagSet,
    pub protocol_visible: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_item: Option<ItemId>,
    #[serde(default)]
    pub refs: RefMap,
    #[serde(default)]
    pub metrics: MetricMap,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub ext: Map<String, Value>,
}

impl ItemDraft {
    fn base(body: ItemBody, protocol_visible: bool) -> Self {
        Self {
            body,
            tags: TagSet::new(),
            protocol_visible,
            pinned: false,
            group_id: None,
            client_id: None,
            parent_item: None,
            refs: RefMap::new(),
            metrics: MetricMap::new(),
            ext: Map::new(),
        }
    }

    pub fn message(message: Message) -> Self {
        Self::base(ItemBody::Message { message }, true)
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self::message(Message::user(text))
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self::message(Message::assistant(text))
    }

    pub fn system(text: impl Into<String>) -> Self {
        Self::message(Message::system(text))
    }

    pub fn notice(text: impl Into<String>) -> Self {
        Self::base(ItemBody::Notice { text: text.into() }, false)
    }

    pub fn artifact(
        name: impl Into<String>,
        media: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self::base(
            ItemBody::Artifact {
                name: name.into(),
                media: media.into(),
                text: text.into(),
            },
            false,
        )
    }

    /// 扩展条目。`kind` 由调用方定义
    pub fn ext(kind: impl Into<String>, payload: Value) -> Result<Self, SessionError> {
        let kind = normalize_ident(kind.into(), "扩展 kind")?;
        Ok(Self::base(ItemBody::Ext { kind, payload }, false))
    }

    pub fn with_tags(mut self, tags: TagSet) -> Self {
        self.tags = tags;
        self
    }

    pub fn protocol_visible(mut self, visible: bool) -> Self {
        self.protocol_visible = visible;
        self
    }

    pub fn pinned(mut self, pinned: bool) -> Self {
        self.pinned = pinned;
        self
    }

    pub fn group(mut self, group_id: impl Into<String>) -> Result<Self, SessionError> {
        self.group_id = Some(normalize_ident(group_id.into(), "分组")?);
        Ok(self)
    }

    pub fn client_id(mut self, client_id: impl Into<String>) -> Self {
        let id = client_id.into();
        self.client_id = (!id.is_empty()).then_some(id);
        self
    }

    pub fn parent_item(mut self, id: ItemId) -> Self {
        self.parent_item = Some(id);
        self
    }

    pub fn bind(
        mut self,
        slot: impl Into<String>,
        refer: crate::ext::ExtRef,
    ) -> Result<Self, SessionError> {
        self.refs.bind(slot, refer)?;
        Ok(self)
    }
}

impl From<Message> for ItemDraft {
    fn from(message: Message) -> Self {
        Self::message(message)
    }
}

/// 创建规格
#[derive(Clone, Debug, Default)]
pub struct CreateSpec {
    pub title: Option<String>,
    pub tags: TagSet,
    pub parent_id: Option<ConversationId>,
    pub parent_item_id: Option<ItemId>,
    pub refs: RefMap,
    pub ext: Map<String, Value>,
}

/// 会话补丁；`None` 表示保持原值
#[derive(Clone, Debug, Default)]
pub struct ConversationPatch {
    pub title: Option<String>,
    pub status: Option<ConversationStatus>,
    pub tag_op: Option<TagOp>,
    pub refs: Option<RefOp>,
    pub metrics: Option<MetricMap>,
    pub ext: Option<Map<String, Value>>,
    pub compact_until: Option<u64>,
    pub parent_item_id: Option<ItemId>,
}

/// 条目补丁。`text_append` `body` 互斥
#[derive(Clone, Debug, Default)]
pub struct ItemPatch {
    pub body: Option<ItemBody>,
    pub text_append: Option<String>,
    pub protocol_visible: Option<bool>,
    pub pinned: Option<bool>,
    pub tags: Option<TagOp>,
    pub group_id: Option<String>,
    pub refs: Option<RefOp>,
    pub metrics: Option<MetricMap>,
    pub ext: Option<Map<String, Value>>,
    pub parent_item: Option<ItemId>,
}

impl ItemPatch {
    pub(crate) fn apply(&self, item: &mut SessionItem) -> Result<(), SessionError> {
        if self.body.is_some() && self.text_append.is_some() {
            return Err(SessionError::invalid("body text_append 不能同时使用"));
        }
        if let Some(body) = &self.body {
            item.body = body.clone();
        }
        if let Some(extra) = &self.text_append {
            item.body.append_text(extra)?;
        }
        if let Some(v) = self.protocol_visible {
            item.protocol_visible = v;
        }
        if let Some(v) = self.pinned {
            item.pinned = v;
        }
        if let Some(op) = &self.tags {
            item.tags.apply(op)?;
        }
        if let Some(g) = &self.group_id {
            item.group_id = if g.is_empty() {
                None
            } else {
                Some(normalize_ident(g, "分组")?)
            };
        }
        if let Some(op) = &self.refs {
            item.refs.apply(op)?;
        }
        if let Some(m) = &self.metrics {
            item.metrics.merge(m);
        }
        if let Some(e) = &self.ext {
            merge_ext(&mut item.ext, e);
        }
        if let Some(p) = &self.parent_item {
            item.parent_item = Some(p.clone());
        }
        item.updated_at = now_ms();
        Ok(())
    }
}

/// 协议投影策略。`max_items = None` 时与全量 `protocol_messages` 一致（仍尊protocol_visible）
#[derive(Clone, Debug)]
pub struct ContextPolicy {
    pub max_items: Option<u32>,
    pub include_pinned: bool,
}

impl Default for ContextPolicy {
    fn default() -> Self {
        Self {
            max_items: None,
            include_pinned: true,
        }
    }
}

/// 协议上下文切片
#[derive(Clone, Debug)]
pub struct ContextView {
    pub messages: Vec<Message>,
    pub item_ids: Vec<ItemId>,
    pub omitted: u64,
}

/// 标签统计
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagStat {
    pub tag: String,
    pub count: u64,
}

/// 标签扫描范围
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagScope {
    Conversation,
    Item,
}

/// 会话及其条目（批量读取）
#[derive(Clone, Debug)]
pub struct ConversationBundle {
    pub conversation: Conversation,
    pub items: Vec<SessionItem>,
}

/// 追加结果
#[derive(Clone, Debug)]
pub struct AppendOutcome {
    pub conversation: Conversation,
    pub items: Vec<SessionItem>,
    /// 新写入条数（幂等命中不计）
    pub created: usize,
}
