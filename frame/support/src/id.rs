//! 标识：能力、实例、方法、装载源、locals 作用域。

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

fn valid_name(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

macro_rules! name_id {
    ($name:ident, $err:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(id: impl Into<String>) -> Result<Self, $err> {
                let id = id.into();
                if !valid_name(&id) {
                    return Err($err::Invalid(id));
                }
                Ok(Self(id))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = $err;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::new(s)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        #[derive(Debug, thiserror::Error)]
        pub enum $err {
            #[error("非法标识: {0}")]
            Invalid(String),
        }
    };
}

name_id!(AbilityId, AbilityIdError);
name_id!(InstanceId, InstanceIdError);
name_id!(MethodId, MethodIdError);
name_id!(SourceId, SourceIdError);

/// 调用链 locals 作用域（端口可选携带，非 AI 会话）。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LocalsScope(uuid::Uuid);

impl LocalsScope {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    pub fn as_uuid(&self) -> uuid::Uuid {
        self.0
    }
}

impl Default for LocalsScope {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for LocalsScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for LocalsScope {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(uuid::Uuid::parse_str(s)?))
    }
}
