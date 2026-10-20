//! Strata 存储。Lance 类型留在本 crate 内。

mod control;
mod engine;
mod error;
mod lance_io;
mod records;
mod resolve;
mod staging;

pub use engine::{
    BLOB_DEDICATED_THRESHOLD, BLOB_INLINE_THRESHOLD, Engine, INLINE_TEXT_MAX, Options, STAGING_TTL,
};
pub use error::Error;
pub use records::{
    FacetRecord, IdentityRecord, MarkRecord, RelationRecord, ResourceRecord, SnapshotInfo,
    StateRecord, View,
};

#[cfg(test)]
mod tests;
