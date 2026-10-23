//! ModalPrism 领域契约。
//!
//! 本 crate 只定义标识、报文和校验。存储、配置加载、网络访问和 LEAF 装配不在这里。

mod changeset;
mod envelope;
mod error;
mod facet;
mod id;
mod query;
mod task;

pub use changeset::{
    ApplyOpts, Body, ChangeSet, CommitResult, LineageInput, Op, Producer, Target, WaitFor,
};
pub use envelope::{
    EngineInfo, Locator, PartRole, RawEnvelope, RawPart, RenderLevel, Signals, Transport,
};
pub use error::{ErrorClass, ModelError};
pub use facet::{FacetKind, FacetRef, FacetSummary, PartRef, SpanRef, VersionSel, content_hash};
pub use id::{
    CommitId, EnvelopeId, IdentityKey, Namespace, PartId, ReceiptId, Rid, SourceId, TaskId,
    Timestamp,
};
pub use query::{
    AsOf, EdgeDir, Expand, Expr, FacetList, FacetSelector, LineageDir, OrderBy, Query, QueryFrom,
    Search, SelectItem,
};
pub use task::{AccessTask, QueueName, TaskReason};
