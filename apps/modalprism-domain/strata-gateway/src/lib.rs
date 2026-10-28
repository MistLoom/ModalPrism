//! Strata 数据读写层。
//!
//! 四件事：统一后端接口、内容寻址 Blob 层、声明式派生、类型化查询。
//! 业务只通过 [`Gateway`] 访问，不接触表结构或方言。

pub mod blob;
pub mod commit;
pub mod config;
pub mod data;
pub mod db;
pub mod derive;
#[cfg(feature = "embedded-s3")]
pub mod embedded;
pub mod error;
pub mod gateway;
pub mod index;
pub mod maintenance;
pub mod query;
#[cfg(feature = "rustfs-bin")]
pub mod rustfs_bin;
pub mod schema;
pub mod sql;
pub mod staging;
pub mod task;

pub use blob::{BlobService, BlobStore, BlobTier, PreparedBlob};
pub use config::{
    Backend, BlobBackend, DerivationSpec, FusionOptions, Options, TextOptions, Tokenizer,
};
pub use data::{
    Batch, BlobRow, CommitRow, Dataset, FacetRow, IdentityRow, MarkRow, RelationRow, ResourceRow,
    SnapshotRow, StateRow, TextRow, VectorRow,
};
pub use db::{Cell, Db, Row, Tx};
pub use derive::DerivationTask;
#[cfg(feature = "embedded-s3")]
pub use embedded::EmbeddedBlobStore;
pub use error::{Error, Result};
pub use gateway::{Gateway, PurgeReport};
pub use index::{IndexStore, Score, TextQuery, VectorQuery};
pub use query::{Channel, Hit, Request, Response};
#[cfg(feature = "rustfs-bin")]
pub use rustfs_bin::RustfsBlobStore;
pub use sql::{Bind, Dialect, Sql};
pub use staging::Staging;
