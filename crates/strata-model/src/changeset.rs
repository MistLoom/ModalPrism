//! 声明式变更集。

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ModelError;
use crate::facet::{FacetKind, PartRef, SpanRef, check_variant};
use crate::id::{CommitId, IdentityKey, Namespace, TaskId};

/// 产出方。配置代际号随变更集进入血缘。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Producer {
    pub module: String,
    pub version: String,
    pub config_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
}

/// `putFacet` 携带的血缘输入。提交时写入的 `commit_id` 由 Strata 分配。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LineageInput {
    pub inputs: Vec<SpanRef>,
    pub params_hash: String,
}

/// 变更集内的目标：已有资源，或本变更集里的局部别名。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Target {
    Rid(crate::id::Rid),
    Alias(String),
}

/// facet 正文。二进制只以暂存引用出现。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Body {
    Text { text: String },
    Struct { value: Value },
    Staged { part: PartRef },
    External { uri: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Op {
    Ensure {
        identity: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ns: Option<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        labels: BTreeMap<String, String>,
        #[serde(rename = "as")]
        r#as: String,
    },
    PutFacet {
        target: Target,
        kind: FacetKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        variant: Option<String>,
        body: Body,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
        lineage: LineageInput,
    },
    Link {
        src: Target,
        rel: String,
        dst: Target,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attrs: Option<Value>,
    },
    Label {
        target: Target,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        set: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        unset: Vec<String>,
    },
    SetState {
        scope: String,
        key: String,
        value: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expect_version: Option<u64>,
    },
    Tombstone {
        target: Target,
        reason: String,
    },
}

/// 一次声明式写入。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeSet {
    pub producer: Producer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ack: Option<TaskId>,
    pub ops: Vec<Op>,
}

impl ChangeSet {
    pub fn validate(&self) -> Result<(), ModelError> {
        if self.producer.module.is_empty() || self.producer.version.is_empty() {
            return Err(ModelError::InvalidProducer);
        }
        if self
            .idempotency_key
            .as_ref()
            .is_some_and(|key| key.is_empty())
        {
            return Err(ModelError::EmptyId);
        }
        for op in &self.ops {
            validate_op(op)?;
        }
        Ok(())
    }
}

fn validate_op(op: &Op) -> Result<(), ModelError> {
    match op {
        Op::Ensure {
            identity,
            ns,
            labels,
            r#as,
        } => {
            IdentityKey::parse(identity)?;
            if let Some(ns) = ns {
                Namespace::parse(ns)?;
            }
            check_labels(labels)?;
            check_alias(r#as)?;
        }
        Op::PutFacet {
            target,
            kind,
            variant,
            body,
            lineage,
            ..
        } => {
            check_target(target)?;
            match variant {
                Some(variant) => check_variant(variant)?,
                None if kind.requires_variant() => return Err(ModelError::MissingVariant),
                None => {}
            }
            if let Body::External { uri } = body {
                if uri.is_empty() {
                    return Err(ModelError::InvalidEnvelope);
                }
            }
            if lineage.params_hash.is_empty() {
                return Err(ModelError::EmptyId);
            }
        }
        Op::Link { src, rel, dst, .. } => {
            check_target(src)?;
            check_target(dst)?;
            if rel.is_empty() {
                return Err(ModelError::EmptyId);
            }
        }
        Op::Label { target, set, unset } => {
            check_target(target)?;
            check_labels(set)?;
            if unset.iter().any(|key| key.is_empty()) {
                return Err(ModelError::EmptyId);
            }
        }
        Op::SetState { scope, key, .. } => {
            if scope.is_empty() || key.is_empty() {
                return Err(ModelError::EmptyId);
            }
        }
        Op::Tombstone { target, reason } => {
            check_target(target)?;
            if reason.is_empty() {
                return Err(ModelError::EmptyId);
            }
        }
    }
    Ok(())
}

fn check_target(target: &Target) -> Result<(), ModelError> {
    match target {
        Target::Rid(_) => Ok(()),
        Target::Alias(alias) => check_alias(alias),
    }
}

fn check_alias(alias: &str) -> Result<(), ModelError> {
    if alias.is_empty() || alias.chars().any(char::is_whitespace) {
        Err(ModelError::EmptyId)
    } else {
        Ok(())
    }
}

fn check_labels(labels: &BTreeMap<String, String>) -> Result<(), ModelError> {
    if labels.keys().any(|key| key.is_empty()) {
        Err(ModelError::EmptyId)
    } else {
        Ok(())
    }
}

/// `apply` 的等待点。默认 `Accepted`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WaitFor {
    Accepted,
    Visible,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyOpts {
    pub wait: WaitFor,
    #[serde(default, with = "humantime_serde::option")]
    pub timeout: Option<Duration>,
}

impl Default for ApplyOpts {
    fn default() -> Self {
        Self {
            wait: WaitFor::Accepted,
            timeout: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommitResult {
    pub commit_id: CommitId,
    pub status: WaitFor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<u64>,
    pub duplicate: bool,
}
