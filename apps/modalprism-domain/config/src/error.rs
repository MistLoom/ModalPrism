//! 配置加载错误。来源错误保留在 `source` 中。

use std::fmt;

/// 一条可定位到文件的配置错误。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub file: String,
    pub line: u32,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.file.is_empty() {
            write!(f, "{}", self.message)
        } else if self.line == 0 {
            write!(f, "{}: {}", self.file, self.message)
        } else {
            write!(f, "{}:{}: {}", self.file, self.line, self.message)
        }
    }
}

/// 一次加载中的全部错误。任一错误都使新快照失效。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostics(pub Vec<Diagnostic>);

impl fmt::Display for Diagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, item) in self.0.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }
            write!(f, "{item}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Diagnostics {}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("读取 {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("监听 {path}: {source}")]
    Watch {
        path: String,
        #[source]
        source: notify::Error,
    },
    #[error(transparent)]
    Invalid(#[from] Diagnostics),
}

impl Error {
    pub fn diagnostics(&self) -> Option<&[Diagnostic]> {
        match self {
            Self::Invalid(items) => Some(&items.0),
            _ => None,
        }
    }
}
