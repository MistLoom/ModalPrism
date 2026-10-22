//! 方言差异只在这里出现：占位符、类型名与 SQL 片段。
//!
//! 上层只写一份语句，绑定值按顺序追加；SQLite 用 `?`，PostgreSQL 用 `$n`。

/// 支持的 SQL 方言。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    Sqlite,
    Postgres,
}

impl Dialect {
    pub fn placeholder(self, index: usize) -> String {
        match self {
            Self::Sqlite => "?".to_string(),
            Self::Postgres => format!("${index}"),
        }
    }

    /// 二进制类型的列定义。
    pub fn blob_type(self) -> &'static str {
        match self {
            Self::Sqlite => "BLOB",
            Self::Postgres => "BYTEA",
        }
    }

    /// 布尔列统一用整数表达，避免两套解码。
    pub fn bool_type(self) -> &'static str {
        "BIGINT"
    }

    pub fn text_type(self) -> &'static str {
        "TEXT"
    }

    pub fn integer_type(self) -> &'static str {
        "BIGINT"
    }

    /// `INSERT ... ON CONFLICT(<target>) DO NOTHING`，两方言同形。
    pub fn on_conflict_do_nothing(&self, target: &str) -> String {
        format!("ON CONFLICT({target}) DO NOTHING")
    }
}

/// 一个待绑定值。`Opt*` 用来表达 NULL，并同时保留列类型。
#[derive(Clone, Debug, PartialEq)]
pub enum Bind {
    Int(i64),
    OptInt(Option<i64>),
    Text(String),
    OptText(Option<String>),
    Bytes(Vec<u8>),
    OptBytes(Option<Vec<u8>>),
    Real(f64),
}

impl Bind {
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    pub fn opt_text(value: Option<impl Into<String>>) -> Self {
        Self::OptText(value.map(Into::into))
    }
}

/// 累积中的语句与绑定值。
#[derive(Clone, Debug)]
pub struct Sql {
    dialect: Dialect,
    text: String,
    binds: Vec<Bind>,
}

impl Sql {
    pub fn new(dialect: Dialect) -> Self {
        Self {
            dialect,
            text: String::new(),
            binds: Vec::new(),
        }
    }

    /// 不带绑定的完整语句。
    pub fn raw(dialect: Dialect, text: impl AsRef<str>) -> Self {
        Self {
            dialect,
            text: text.as_ref().to_string(),
            binds: Vec::new(),
        }
    }

    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    pub fn as_text(&self) -> &str {
        &self.text
    }

    pub fn binds(&self) -> &[Bind] {
        &self.binds
    }

    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    pub fn into_parts(self) -> (String, Vec<Bind>) {
        (self.text, self.binds)
    }

    /// 追加一段原样 SQL。
    pub fn push(&mut self, raw: impl AsRef<str>) -> &mut Self {
        self.text.push_str(raw.as_ref());
        self
    }

    /// 追加一个占位符并记录绑定值。
    pub fn bind(&mut self, value: Bind) -> &mut Self {
        let placeholder = self.dialect.placeholder(self.binds.len() + 1);
        self.text.push_str(&placeholder);
        self.binds.push(value);
        self
    }

    pub fn text(&mut self, value: impl Into<String>) -> &mut Self {
        self.bind(Bind::Text(value.into()))
    }

    pub fn opt_text(&mut self, value: Option<impl Into<String>>) -> &mut Self {
        self.bind(Bind::OptText(value.map(Into::into)))
    }

    pub fn int(&mut self, value: i64) -> &mut Self {
        self.bind(Bind::Int(value))
    }

    pub fn u64(&mut self, value: u64) -> &mut Self {
        self.bind(Bind::Int(to_i64(value)))
    }

    pub fn opt_int(&mut self, value: Option<i64>) -> &mut Self {
        self.bind(Bind::OptInt(value))
    }

    pub fn real(&mut self, value: f64) -> &mut Self {
        self.bind(Bind::Real(value))
    }

    pub fn bytes(&mut self, value: Vec<u8>) -> &mut Self {
        self.bind(Bind::Bytes(value))
    }

    pub fn opt_bytes(&mut self, value: Option<Vec<u8>>) -> &mut Self {
        self.bind(Bind::OptBytes(value))
    }

    /// NULL 安全比较：`column = ?` 或 `column IS NULL`。
    pub fn nullable_eq(&mut self, column: &str, value: Option<impl Into<String>>) -> &mut Self {
        match value {
            Some(value) => {
                self.push(column).push(" = ");
                self.text(value);
            }
            None => {
                self.push(column).push(" IS NULL");
            }
        }
        self
    }

    /// 追加 `IN (?, ?, ...)` 列表。
    pub fn in_list(&mut self, column: &str, values: &[String]) -> &mut Self {
        self.push(column).push(" IN (");
        for (index, value) in values.iter().enumerate() {
            if index > 0 {
                self.push(", ");
            }
            self.text(value.clone());
        }
        self.push(")");
        self
    }
}

/// u64 落库统一按 i64 处理；真实序列不会越过边界。
pub fn to_i64(value: u64) -> i64 {
    value.min(i64::MAX as u64) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_follow_dialect() {
        let mut sql = Sql::new(Dialect::Postgres);
        sql.push("SELECT 1 WHERE a = ");
        sql.text("x");
        sql.push(" AND b = ");
        sql.int(2);
        assert_eq!(sql.as_text(), "SELECT 1 WHERE a = $1 AND b = $2");
        assert_eq!(sql.binds().len(), 2);

        let mut sql = Sql::new(Dialect::Sqlite);
        sql.push("SELECT 1 WHERE a = ");
        sql.text("x");
        assert_eq!(sql.as_text(), "SELECT 1 WHERE a = ?");
    }

    #[test]
    fn nullable_eq_never_binds_null() {
        let mut sql = Sql::new(Dialect::Sqlite);
        sql.push("SELECT 1 FROM t WHERE ");
        sql.nullable_eq("variant", None::<String>);
        assert_eq!(sql.as_text(), "SELECT 1 FROM t WHERE variant IS NULL");
        assert!(sql.binds().is_empty());
    }
}
