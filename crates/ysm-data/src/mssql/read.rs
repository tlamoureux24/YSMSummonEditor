use std::borrow::Cow;
use std::collections::HashMap;
use std::time::Duration;

use mysql::consts::{ColumnFlags, ColumnType};
use mysql::prelude::Queryable;
use mysql::{Column, Conn, OptsBuilder, Params, SslOpts, Value};
use rust_decimal::Decimal;
use tiberius::numeric::Numeric;
use tiberius::{ColumnData, ToSql};

use super::{SqlClient, Timeouts, classify};
use crate::{DataError, DbSettings, Result};

pub(super) enum ReadClient {
    SqlServer(Box<SqlClient>, Duration),
    MariaDb(Box<Conn>),
}

pub(super) struct ReadRow {
    pub names: Vec<String>,
    pub values: Vec<ColumnData<'static>>,
}

tokio::task_local! {
    /// Set by `SqlRepository::run` for one operation: read the game texts from
    /// `StringResource_EN` instead of `StringResource`.
    static ENGLISH_TEXTS: bool;
}

pub(super) async fn with_english_texts<T>(english: bool, operation: impl Future<Output = T>) -> T {
    ENGLISH_TEXTS.scope(english, operation).await
}

/// The read queries name the text table `StringResource`, bracketed or not.
fn text_table(sql: &str) -> Cow<'_, str> {
    if ENGLISH_TEXTS.try_with(|english| *english).unwrap_or(false) {
        Cow::Owned(
            sql.replace("[StringResource]", "[StringResource_EN]")
                .replace("JOIN StringResource ", "JOIN StringResource_EN "),
        )
    } else {
        Cow::Borrowed(sql)
    }
}

impl ReadClient {
    pub fn is_mariadb(&self) -> bool {
        matches!(self, Self::MariaDb(_))
    }

    pub async fn is_alive(&mut self) -> bool {
        match self {
            Self::SqlServer(..) => self.simple_query("SELECT 1").await.is_ok(),
            Self::MariaDb(client) => client.ping().is_ok(),
        }
    }

    pub async fn simple_query(&mut self, sql: impl AsRef<str>) -> Result<Vec<ReadRow>> {
        self.query(sql, &[]).await
    }

    pub async fn query(&mut self, sql: impl AsRef<str>, params: &[&dyn ToSql]) -> Result<Vec<ReadRow>> {
        let sql = text_table(sql.as_ref());
        let sql = sql.as_ref();
        match self {
            Self::SqlServer(client, timeout) => {
                let stream =
                    if params.is_empty() { client.simple_query(sql).await } else { client.query(sql, params).await }
                        .map_err(|error| classify(&error, *timeout))?;
                let rows = stream.into_first_result().await.map_err(|error| classify(&error, *timeout))?;
                Ok(rows
                    .into_iter()
                    .map(|row| ReadRow {
                        names: row.columns().iter().map(|column| column.name().to_owned()).collect(),
                        values: row.into_iter().collect(),
                    })
                    .collect())
            }
            Self::MariaDb(client) => {
                let sql = mariadb_query(sql, params.len());
                let rows: Vec<mysql::Row> = if params.is_empty() {
                    client.query(sql)
                } else {
                    let params = params
                        .iter()
                        .enumerate()
                        .map(|(index, value)| {
                            let value = match value.to_sql() {
                                ColumnData::I32(value) => Value::from(value),
                                ColumnData::Bit(value) => Value::from(value),
                                ColumnData::String(value) => Value::from(value.as_deref()),
                                _ => return Err(DataError::Invalid("Unsupported query parameter type".into())),
                            };
                            Ok((format!("p{}", index + 1).into_bytes(), value))
                        })
                        .collect::<Result<HashMap<_, _>>>()?;
                    client.exec(sql, Params::Named(params))
                }
                .map_err(mariadb_error)?;
                rows.into_iter().map(mariadb_row).collect()
            }
        }
    }
}

pub(super) fn connect_mariadb(settings: &DbSettings, timeouts: Timeouts) -> Result<Conn> {
    let tls = SslOpts::default()
        .with_danger_accept_invalid_certs(settings.trust_server_certificate)
        .with_danger_skip_domain_validation(settings.trust_server_certificate);
    let opts = OptsBuilder::default()
        .ip_or_hostname(Some(settings.server.trim()))
        .tcp_port(settings.effective_port())
        .db_name(Some(settings.database.trim()))
        .user(Some(settings.user_id.trim()))
        .pass(Some(settings.password.as_str()))
        .prefer_socket(false)
        .ssl_opts(tls)
        .tcp_connect_timeout(Some(timeouts.connect))
        .read_timeout(Some(timeouts.command))
        .write_timeout(Some(timeouts.command))
        .init(vec!["SET NAMES utf8mb4"]);
    Conn::new(opts).map_err(|error| DataError::Connection(error.to_string()))
}

pub(super) fn mariadb_error(error: mysql::Error) -> DataError {
    match &error {
        mysql::Error::MySqlError(_) => DataError::Query(error.to_string()),
        mysql::Error::IoError(io)
            if matches!(io.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) =>
        {
            DataError::Timeout(error.to_string())
        }
        _ => DataError::Connection(error.to_string()),
    }
}

/// These substitutions apply only to the fixed read queries. User search terms
/// are still driver parameters, and the only inlined inputs are integer IDs.
fn mariadb_query(sql: &str, param_count: usize) -> String {
    let mut sql = sql.trim().replace(['[', ']'], "`").replace("AS nvarchar(32)", "AS CHAR");
    if let Some(rest) = sql.strip_prefix("SELECT TOP 1") {
        sql = format!("SELECT{} LIMIT 1", rest.trim_end_matches(';'));
    } else if let Some(rest) = sql.strip_prefix("SELECT TOP (@P1)") {
        sql = format!("SELECT{} LIMIT @P1", rest.trim_end_matches(';'));
    }
    for index in (1..=param_count).rev() {
        sql = sql.replace(&format!("@P{index}"), &format!(":p{index}"));
    }
    sql
}

fn mariadb_row(row: mysql::Row) -> Result<ReadRow> {
    let columns = row.columns();
    let names = columns.iter().map(|column| column.name_str().into_owned()).collect();
    let values =
        columns.iter().zip(row.unwrap()).map(|(column, value)| mariadb_cell(column, value)).collect::<Result<_>>()?;
    Ok(ReadRow { names, values })
}

fn mariadb_cell(column: &Column, value: Value) -> Result<ColumnData<'static>> {
    use ColumnType::*;
    fn read<T: mysql::prelude::FromValue>(column: &Column, value: Value) -> Result<Option<T>> {
        mysql::from_value_opt(value)
            .map_err(|_| DataError::Query(format!("Unexpected value in MariaDB column {}", column.name_str())))
    }
    let unsigned = column.flags().contains(ColumnFlags::UNSIGNED_FLAG);
    Ok(match column.column_type() {
        MYSQL_TYPE_TINY if unsigned => ColumnData::U8(read(column, value)?),
        MYSQL_TYPE_TINY | MYSQL_TYPE_SHORT if !unsigned => ColumnData::I16(read(column, value)?),
        MYSQL_TYPE_SHORT | MYSQL_TYPE_INT24 => ColumnData::I32(read(column, value)?),
        MYSQL_TYPE_LONG if !unsigned => ColumnData::I32(read(column, value)?),
        MYSQL_TYPE_LONG | MYSQL_TYPE_LONGLONG => ColumnData::I64(read(column, value)?),
        MYSQL_TYPE_FLOAT => ColumnData::F32(read(column, value)?),
        MYSQL_TYPE_DOUBLE => ColumnData::F64(read(column, value)?),
        MYSQL_TYPE_BIT => {
            let bytes: Option<Vec<u8>> = read(column, value)?;
            ColumnData::Bit(bytes.map(|bytes| bytes.into_iter().any(|byte| byte != 0)))
        }
        MYSQL_TYPE_DECIMAL | MYSQL_TYPE_NEWDECIMAL => {
            let text: Option<String> = read(column, value)?;
            let number = text
                .map(|text| {
                    text.parse::<Decimal>()
                        .map(|decimal| Numeric::new_with_scale(decimal.mantissa(), decimal.scale() as u8))
                        .map_err(|_| DataError::Query(format!("Decimal out of range in {}", column.name_str())))
                })
                .transpose()?;
            ColumnData::Numeric(number)
        }
        MYSQL_TYPE_STRING
        | MYSQL_TYPE_VAR_STRING
        | MYSQL_TYPE_VARCHAR
        | MYSQL_TYPE_BLOB
        | MYSQL_TYPE_TINY_BLOB
        | MYSQL_TYPE_MEDIUM_BLOB
        | MYSQL_TYPE_LONG_BLOB
            if column.character_set() != 63 =>
        {
            let text: Option<String> = read(column, value)?;
            ColumnData::String(text.map(Cow::Owned))
        }
        MYSQL_TYPE_NULL => ColumnData::String(None),
        _ => ColumnData::Binary(None),
    })
}

#[cfg(test)]
mod text_table_tests {
    use super::*;

    fn in_scope(english: bool, sql: &'static str) -> String {
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        runtime.block_on(with_english_texts(english, async move { text_table(sql).into_owned() }))
    }

    #[test]
    fn english_reads_the_english_text_table() {
        let sql = "LEFT JOIN [StringResource] sn ON a LEFT JOIN StringResource sr ON b";
        assert_eq!(in_scope(true, sql), "LEFT JOIN [StringResource_EN] sn ON a LEFT JOIN StringResource_EN sr ON b");
        assert_eq!(in_scope(false, sql), sql);
        assert_eq!(text_table(sql), sql, "outside an operation nothing changes");
    }
}
