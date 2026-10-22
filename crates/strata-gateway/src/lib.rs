//! Strata 数据读写层。
//!
//! 四件事：统一后端接口、内容寻址 Blob 层、声明式派生、类型化查询。
//! Lance / DataFusion 不在这里出现。

pub mod blob;
pub mod config;
pub mod data;
pub mod db;
pub mod derive;
pub mod error;
pub mod gateway;
pub mod index;
pub mod query;
pub mod schema;
pub mod sql;
pub mod task;

pub use blob::{BlobService, BlobStore, BlobTier, PreparedBlob};
pub use config::{
    Backend, BlobBackend, DerivationSpec, FusionOptions, Options, TextOptions, Tokenizer,
};
pub use data::{
    Batch, BlobRow, Dataset, FacetRow, IdentityRow, MarkRow, RelationRow, ResourceRow, SnapshotRow,
    StateRow, TextRow, VectorRow,
};
pub use db::{Cell, Db, Row, Tx};
pub use derive::DerivationTask;
pub use error::{Error, Result};
pub use gateway::Gateway;
pub use index::{IndexStore, Score, TextQuery, VectorQuery};
pub use query::{Channel, Hit, Request, Response};
pub use sql::{Bind, Dialect, Sql};
