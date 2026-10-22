//! 数据库连接、行抽象与事务。
//!
//! 方言差异在 `sql` 模块；这里只负责把绑定值送进驱动、把结果行变成 `Row`。

use std::path::Path;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use sqlx::{Column, Row as SqlxRow, TypeInfo, ValueRef};

use crate::config::Backend;
use crate::error::{Error, Result};
use crate::sql::{Bind, Dialect, Sql};

/// 无类型的一列值。整数承载 u64/bool，布尔统一 0/1。
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Bytes(Vec<u8>),
}

/// 一行结果，按列名取值。
#[derive(Clone, Debug, Default)]
pub struct Row {
    columns: Vec<String>,
    cells: Vec<Cell>,
}

impl Row {
    pub fn new(columns: Vec<String>, cells: Vec<Cell>) -> Self {
        Self { columns, cells }
    }

    pub fn cell(&self, name: &str) -> Option<&Cell> {
        self.columns
            .iter()
            .position(|column| column == name)
            .and_then(|index| self.cells.get(index))
    }

    fn require(&self, name: &str) -> Result<&Cell> {
        self.cell(name)
            .ok_or_else(|| Error::UnknownColumn(name.to_string()))
    }

    pub fn text(&self, name: &str) -> Result<String> {
        match self.require(name)? {
            Cell::Text(value) => Ok(value.clone()),
            other => Err(Error::ColumnType(format!("{name} 不是文本: {other:?}"))),
        }
    }

    pub fn opt_text(&self, name: &str) -> Result<Option<String>> {
        match self.require(name)? {
            Cell::Null => Ok(None),
            Cell::Text(value) => Ok(Some(value.clone())),
            other => Err(Error::ColumnType(format!("{name} 不是文本: {other:?}"))),
        }
    }

    pub fn int(&self, name: &str) -> Result<i64> {
        match self.require(name)? {
            Cell::Int(value) => Ok(*value),
            other => Err(Error::ColumnType(format!("{name} 不是整数: {other:?}"))),
        }
    }

    pub fn u64(&self, name: &str) -> Result<u64> {
        self.int(name).map(|value| value.max(0) as u64)
    }

    pub fn opt_u64(&self, name: &str) -> Result<Option<u64>> {
        match self.require(name)? {
            Cell::Null => Ok(None),
            Cell::Int(value) => Ok(Some((*value).max(0) as u64)),
            other => Err(Error::ColumnType(format!("{name} 不是整数: {other:?}"))),
        }
    }

    pub fn bool(&self, name: &str) -> Result<bool> {
        self.int(name).map(|value| value != 0)
    }

    pub fn bytes(&self, name: &str) -> Result<Vec<u8>> {
        match self.require(name)? {
            Cell::Bytes(value) => Ok(value.clone()),
            other => Err(Error::ColumnType(format!("{name} 不是字节: {other:?}"))),
        }
    }

    pub fn opt_bytes(&self, name: &str) -> Result<Option<Vec<u8>>> {
        match self.require(name)? {
            Cell::Null => Ok(None),
            Cell::Bytes(value) => Ok(Some(value.clone())),
            other => Err(Error::ColumnType(format!("{name} 不是字节: {other:?}"))),
        }
    }

    pub fn real(&self, name: &str) -> Result<f64> {
        match self.require(name)? {
            Cell::Real(value) => Ok(*value),
            Cell::Int(value) => Ok(*value as f64),
            other => Err(Error::ColumnType(format!("{name} 不是数值: {other:?}"))),
        }
    }
}

fn sql_error(context: &'static str) -> impl Fn(sqlx::Error) -> Error {
    move |source| Error::Sql { context, source }
}

/// 元数据后端连接。
pub enum Db {
    Sqlite(SqlitePool),
    #[cfg(feature = "postgres")]
    Postgres(sqlx::postgres::PgPool),
}

impl Db {
    pub async fn open(backend: &Backend, root: &Path) -> Result<Self> {
        match backend {
            Backend::Sqlite { path, extensions } => {
                let path = path.clone().unwrap_or_else(|| root.join("gateway.db"));
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                let mut options = SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .busy_timeout(Duration::from_secs(10))
                    .foreign_keys(true)
                    .synchronous(sqlx::sqlite::SqliteSynchronous::Normal);
                for extension in extensions {
                    options = options.extension(extension.clone());
                }
                let pool = SqlitePoolOptions::new()
                    .max_connections(4)
                    .connect_with(options)
                    .await
                    .map_err(sql_error("打开 SQLite"))?;
                Ok(Self::Sqlite(pool))
            }
            #[cfg(feature = "postgres")]
            Backend::Postgres { url } => {
                let pool = sqlx::postgres::PgPoolOptions::new()
                    .max_connections(8)
                    .connect(url)
                    .await
                    .map_err(sql_error("连接 PostgreSQL"))?;
                Ok(Self::Postgres(pool))
            }
            #[cfg(not(feature = "postgres"))]
            Backend::Postgres { .. } => Err(Error::BackendDisabled("postgres")),
        }
    }

    pub fn dialect(&self) -> Dialect {
        match self {
            Self::Sqlite(_) => Dialect::Sqlite,
            #[cfg(feature = "postgres")]
            Self::Postgres(_) => Dialect::Postgres,
        }
    }

    pub async fn exec(&self, sql: &Sql) -> Result<u64> {
        match self {
            Self::Sqlite(pool) => {
                let result = sqlite_query(sql)
                    .execute(pool)
                    .await
                    .map_err(sql_error("执行 SQL"))?;
                Ok(result.rows_affected())
            }
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => {
                let result = pg_query(sql)
                    .execute(pool)
                    .await
                    .map_err(sql_error("执行 SQL"))?;
                Ok(result.rows_affected())
            }
        }
    }

    pub async fn exec_batch(&self, statements: &[Sql]) -> Result<()> {
        let mut tx = self.begin().await?;
        for statement in statements {
            tx.exec(statement).await?;
        }
        tx.commit().await
    }

    pub async fn fetch(&self, sql: &Sql) -> Result<Vec<Row>> {
        match self {
            Self::Sqlite(pool) => {
                let rows = sqlite_query(sql)
                    .fetch_all(pool)
                    .await
                    .map_err(sql_error("查询"))?;
                rows.iter().map(sqlite_row).collect()
            }
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => {
                let rows = pg_query(sql)
                    .fetch_all(pool)
                    .await
                    .map_err(sql_error("查询"))?;
                rows.iter().map(pg_row).collect()
            }
        }
    }

    pub async fn fetch_one(&self, sql: &Sql) -> Result<Option<Row>> {
        Ok(self.fetch(sql).await?.into_iter().next())
    }

    pub async fn fetch_int(&self, sql: &Sql) -> Result<Option<i64>> {
        let mut rows = self.fetch(sql).await?;
        match rows.pop() {
            Some(row) => {
                let columns = row.columns.clone();
                let name = columns.first().ok_or(Error::UnknownColumn("第一列".into()))?;
                Ok(Some(row.int(name)?))
            }
            None => Ok(None),
        }
    }

    pub async fn begin(&self) -> Result<Tx<'_>> {
        match self {
            Self::Sqlite(pool) => Ok(Tx::Sqlite(
                pool.begin().await.map_err(sql_error("开启事务"))?,
            )),
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => {
                use sqlx::Acquire;
                Ok(Tx::Postgres(
                    pool.begin().await.map_err(sql_error("开启事务"))?,
                ))
            }
        }
    }
}

/// 显式事务。所有写入都走这里，保证数据与索引同版本。
pub enum Tx<'a> {
    Sqlite(sqlx::Transaction<'a, sqlx::Sqlite>),
    #[cfg(feature = "postgres")]
    Postgres(sqlx::Transaction<'a, sqlx::Postgres>),
}

impl Tx<'_> {
    pub fn dialect(&self) -> Dialect {
        match self {
            Self::Sqlite(_) => Dialect::Sqlite,
            #[cfg(feature = "postgres")]
            Self::Postgres(_) => Dialect::Postgres,
        }
    }

    pub async fn exec(&mut self, sql: &Sql) -> Result<u64> {
        match self {
            Self::Sqlite(tx) => {
                let result = sqlite_query(sql)
                    .execute(&mut **tx)
                    .await
                    .map_err(sql_error("执行 SQL"))?;
                Ok(result.rows_affected())
            }
            #[cfg(feature = "postgres")]
            Self::Postgres(tx) => {
                let result = pg_query(sql)
                    .execute(&mut **tx)
                    .await
                    .map_err(sql_error("执行 SQL"))?;
                Ok(result.rows_affected())
            }
        }
    }

    pub async fn fetch(&mut self, sql: &Sql) -> Result<Vec<Row>> {
        match self {
            Self::Sqlite(tx) => {
                let rows = sqlite_query(sql)
                    .fetch_all(&mut **tx)
                    .await
                    .map_err(sql_error("查询"))?;
                rows.iter().map(sqlite_row).collect()
            }
            #[cfg(feature = "postgres")]
            Self::Postgres(tx) => {
                let rows = pg_query(sql)
                    .fetch_all(&mut **tx)
                    .await
                    .map_err(sql_error("查询"))?;
                rows.iter().map(pg_row).collect()
            }
        }
    }

    pub async fn fetch_one(&mut self, sql: &Sql) -> Result<Option<Row>> {
        Ok(self.fetch(sql).await?.into_iter().next())
    }

    pub async fn fetch_int(&mut self, sql: &Sql) -> Result<Option<i64>> {
        let mut rows = self.fetch(sql).await?;
        match rows.pop() {
            Some(row) => {
                let columns = row.columns.clone();
                let name = columns.first().ok_or(Error::UnknownColumn("第一列".into()))?;
                Ok(Some(row.int(name)?))
            }
            None => Ok(None),
        }
    }

    pub async fn commit(self) -> Result<()> {
        match self {
            Self::Sqlite(tx) => tx.commit().await.map_err(sql_error("提交事务")),
            #[cfg(feature = "postgres")]
            Self::Postgres(tx) => tx.commit().await.map_err(sql_error("提交事务")),
        }
    }

    pub async fn rollback(self) -> Result<()> {
        match self {
            Self::Sqlite(tx) => tx.rollback().await.map_err(sql_error("回滚事务")),
            #[cfg(feature = "postgres")]
            Self::Postgres(tx) => tx.rollback().await.map_err(sql_error("回滚事务")),
        }
    }
}

fn sqlite_query<'q>(
    sql: &'q Sql,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    let mut query = sqlx::query(sql.as_text());
    for bind in sql.binds() {
        query = match bind {
            Bind::Int(value) => query.bind(*value),
            Bind::OptInt(value) => query.bind(*value),
            Bind::Text(value) => query.bind(value.as_str()),
            Bind::OptText(value) => query.bind(value.as_deref()),
            Bind::Bytes(value) => query.bind(value.as_slice()),
            Bind::OptBytes(value) => query.bind(value.as_deref()),
            Bind::Real(value) => query.bind(*value),
        };
    }
    query
}

#[cfg(feature = "postgres")]
fn pg_query<'q>(
    sql: &'q Sql,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    let mut query = sqlx::query(sql.as_text());
    for bind in sql.binds() {
        query = match bind {
            Bind::Int(value) => query.bind(*value),
            Bind::OptInt(value) => query.bind(*value),
            Bind::Text(value) => query.bind(value.as_str()),
            Bind::OptText(value) => query.bind(value.as_deref()),
            Bind::Bytes(value) => query.bind(value.as_slice()),
            Bind::OptBytes(value) => query.bind(value.as_deref()),
            Bind::Real(value) => query.bind(*value),
        };
    }
    query
}

fn sqlite_row(row: &sqlx::sqlite::SqliteRow) -> Result<Row> {
    let columns: Vec<String> = row
        .columns()
        .iter()
        .map(|column| column.name().to_string())
        .collect();
    let mut cells = Vec::with_capacity(columns.len());
    for index in 0..columns.len() {
        let raw = row
            .try_get_raw(index)
            .map_err(sql_error("读取 SQLite 列"))?;
        let cell = if raw.is_null() {
            Cell::Null
        } else {
            match raw.type_info().name() {
                "INTEGER" | "INT" => Cell::Int(
                    row.try_get::<i64, _>(index)
                        .map_err(sql_error("读取整数列"))?,
                ),
                "REAL" => Cell::Real(
                    row.try_get::<f64, _>(index)
                        .map_err(sql_error("读取实数列"))?,
                ),
                "BLOB" => Cell::Bytes(
                    row.try_get::<Vec<u8>, _>(index)
                        .map_err(sql_error("读取字节列"))?,
                ),
                _ => Cell::Text(
                    row.try_get::<String, _>(index)
                        .map_err(sql_error("读取文本列"))?,
                ),
            }
        };
        cells.push(cell);
    }
    Ok(Row::new(columns, cells))
}

#[cfg(feature = "postgres")]
fn pg_row(row: &sqlx::postgres::PgRow) -> Result<Row> {
    let columns: Vec<String> = row
        .columns()
        .iter()
        .map(|column| column.name().to_string())
        .collect();
    let mut cells = Vec::with_capacity(columns.len());
    for index in 0..columns.len() {
        let raw = row
            .try_get_raw(index)
            .map_err(sql_error("读取 PostgreSQL 列"))?;
        let name = raw.type_info().name();
        let cell = if raw.is_null() {
            Cell::Null
        } else {
            match name {
                "INT2" => Cell::Int(
                    row.try_get::<i16, _>(index)
                        .map_err(sql_error("读取 INT2"))? as i64,
                ),
                "INT4" => Cell::Int(
                    row.try_get::<i32, _>(index)
                        .map_err(sql_error("读取 INT4"))? as i64,
                ),
                "INT8" => Cell::Int(
                    row.try_get::<i64, _>(index)
                        .map_err(sql_error("读取 INT8"))?,
                ),
                "FLOAT4" => Cell::Real(
                    row.try_get::<f32, _>(index)
                        .map_err(sql_error("读取 FLOAT4"))? as f64,
                ),
                "FLOAT8" => Cell::Real(
                    row.try_get::<f64, _>(index)
                        .map_err(sql_error("读取 FLOAT8"))?,
                ),
                "BOOL" => Cell::Int(
                    if row
                        .try_get::<bool, _>(index)
                        .map_err(sql_error("读取 BOOL"))?
                    {
                        1
                    } else {
                        0
                    },
                ),
                "BYTEA" => Cell::Bytes(
                    row.try_get::<Vec<u8>, _>(index)
                        .map_err(sql_error("读取 BYTEA"))?,
                ),
                _ => Cell::Text(
                    row.try_get::<String, _>(index)
                        .map_err(sql_error("读取文本列"))?,
                ),
            }
        };
        cells.push(cell);
    }
    Ok(Row::new(columns, cells))
}
