//! Hochgeladene Bilder eines Datensatzes. Die Admin-Datei bestimmt, welches Feld ein Bild ist.

use fexu_core::FexuError;
use fexu_db::driver::sea_query::{Alias, Expr, PostgresQueryBuilder, Query, SqliteQueryBuilder};
use fexu_db::driver::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, Value as DbValue,
};
use std::collections::HashSet;
use uuid::Uuid;

const FILES: &str = "_fexu_record_files";
pub(crate) const IMAGE_BYTES: usize = 900_000;

pub(crate) async fn save(
    db: &DatabaseConnection,
    model_key: &str,
    record: Uuid,
    field: &str,
    content_type: &str,
    bytes: &[u8],
) -> Result<(), FexuError> {
    ensure_table(db).await?;
    let backend = db.get_database_backend();
    let mut delete = Query::delete();
    delete
        .from_table(Alias::new(FILES))
        .and_where(Expr::col(Alias::new("model_key")).eq(model_key))
        .and_where(Expr::col(Alias::new("record_id")).eq(record.to_string()))
        .and_where(Expr::col(Alias::new("field")).eq(field));
    execute(db, backend, &delete).await?;
    let mut insert = Query::insert();
    insert
        .into_table(Alias::new(FILES))
        .columns([
            Alias::new("model_key"),
            Alias::new("record_id"),
            Alias::new("field"),
            Alias::new("content_type"),
            Alias::new("bytes"),
        ])
        .values_panic([
            DbValue::from(model_key).into(),
            DbValue::from(record.to_string()).into(),
            DbValue::from(field).into(),
            DbValue::from(content_type).into(),
            DbValue::Bytes(Some(Box::new(bytes.to_vec()))).into(),
        ]);
    execute(db, backend, &insert).await
}

pub(crate) async fn load(
    db: &DatabaseConnection,
    model_key: &str,
    record: Uuid,
    field: &str,
) -> Result<Option<(String, Vec<u8>)>, FexuError> {
    ensure_table(db).await?;
    let backend = db.get_database_backend();
    let mut query = Query::select();
    query
        .columns([Alias::new("bytes")])
        .from(Alias::new(FILES))
        .and_where(Expr::col(Alias::new("model_key")).eq(model_key))
        .and_where(Expr::col(Alias::new("record_id")).eq(record.to_string()))
        .and_where(Expr::col(Alias::new("field")).eq(field));
    let rows = fetch(db, backend, &query).await?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    let bytes = row
        .try_get::<Vec<u8>>("", "bytes")
        .or_else(|_| {
            row.try_get::<Option<Vec<u8>>>("", "bytes")
                .map(|value| value.unwrap_or_default())
        })
        .map_err(FexuError::database)?;
    let Some(kind) = image_type(&bytes) else {
        return Ok(None);
    };
    Ok(Some((kind.to_owned(), bytes)))
}

pub(crate) async fn exists(
    db: &DatabaseConnection,
    model_key: &str,
    record: Uuid,
    field: &str,
) -> bool {
    load(db, model_key, record, field)
        .await
        .ok()
        .flatten()
        .is_some()
}

pub(crate) async fn stored_ids(
    db: &DatabaseConnection,
    model_key: &str,
    field: &str,
    ids: &[Uuid],
) -> Result<HashSet<Uuid>, FexuError> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    ensure_table(db).await?;
    let backend = db.get_database_backend();
    let keys: Vec<String> = ids.iter().map(ToString::to_string).collect();
    let mut query = Query::select();
    query
        .column(Alias::new("record_id"))
        .from(Alias::new(FILES))
        .and_where(Expr::col(Alias::new("model_key")).eq(model_key))
        .and_where(Expr::col(Alias::new("field")).eq(field))
        .and_where(Expr::col(Alias::new("record_id")).is_in(keys));
    let mut found = HashSet::new();
    for row in fetch(db, backend, &query).await? {
        let id = row
            .try_get::<String>("", "record_id")
            .map_err(FexuError::database)?;
        if let Ok(id) = Uuid::parse_str(&id) {
            found.insert(id);
        }
    }
    Ok(found)
}

pub(crate) async fn delete_record(
    db: &DatabaseConnection,
    model_key: &str,
    record: Uuid,
) -> Result<(), FexuError> {
    ensure_table(db).await?;
    let backend = db.get_database_backend();
    let mut delete = Query::delete();
    delete
        .from_table(Alias::new(FILES))
        .and_where(Expr::col(Alias::new("model_key")).eq(model_key))
        .and_where(Expr::col(Alias::new("record_id")).eq(record.to_string()));
    execute(db, backend, &delete).await
}

pub(crate) fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.is_empty() || bytes.len() > IMAGE_BYTES {
        return None;
    }
    if let Some(kind) = fexu_storage::detect_type(bytes) {
        return match kind {
            "image/png" | "image/jpeg" | "image/gif" | "image/webp" => Some(kind),
            _ => None,
        };
    }
    if bytes.len() >= 6
        && bytes[0] == 0
        && bytes[1] == 0
        && bytes[2] == 1
        && bytes[3] == 0
        && u16::from_le_bytes([bytes[4], bytes[5]]) >= 1
    {
        Some("image/x-icon")
    } else {
        None
    }
}

async fn ensure_table(db: &DatabaseConnection) -> Result<(), FexuError> {
    let backend = db.get_database_backend();
    let bytes = match backend {
        DatabaseBackend::Postgres => "BYTEA",
        DatabaseBackend::Sqlite => "BLOB",
        DatabaseBackend::MySql => {
            return Err(FexuError::Validation(
                "MySQL wird nicht unterstützt.".into(),
            ))
        }
    };
    db.execute(Statement::from_string(
        backend,
        "CREATE TABLE IF NOT EXISTS _fexu_record_files (
            model_key TEXT NOT NULL,
            record_id TEXT NOT NULL,
            field TEXT NOT NULL,
            content_type TEXT NOT NULL,
            bytes BLOB,
            PRIMARY KEY (model_key, record_id, field)
        )"
        .replace("BLOB", bytes),
    ))
    .await
    .map(|_| ())
    .map_err(FexuError::database)
}

async fn execute(
    db: &impl ConnectionTrait,
    backend: DatabaseBackend,
    query: &impl fexu_db::driver::sea_query::QueryStatementWriter,
) -> Result<(), FexuError> {
    let (sql, values) = build(backend, query)?;
    db.execute(Statement::from_sql_and_values(backend, sql, values))
        .await
        .map(|_| ())
        .map_err(FexuError::database)
}

async fn fetch(
    db: &impl ConnectionTrait,
    backend: DatabaseBackend,
    query: &impl fexu_db::driver::sea_query::QueryStatementWriter,
) -> Result<Vec<fexu_db::driver::QueryResult>, FexuError> {
    let (sql, values) = build(backend, query)?;
    db.query_all(Statement::from_sql_and_values(backend, sql, values))
        .await
        .map_err(FexuError::database)
}

fn build(
    backend: DatabaseBackend,
    query: &impl fexu_db::driver::sea_query::QueryStatementWriter,
) -> Result<(String, Vec<DbValue>), FexuError> {
    let (sql, values) = match backend {
        DatabaseBackend::Postgres => query.build(PostgresQueryBuilder),
        DatabaseBackend::Sqlite => query.build(SqliteQueryBuilder),
        DatabaseBackend::MySql => {
            return Err(FexuError::Validation(
                "MySQL wird nicht unterstützt.".into(),
            ))
        }
    };
    Ok((sql, values.0))
}
