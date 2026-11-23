//! Admin project settings. The configuration file supplies defaults; the table stores changes.

use crate::registry::{is_hex_color, is_public_label, AdminBranding, AdminRegistry, ProjectIcons};
use fexu_core::FexuError;
use fexu_db::driver::sea_query::{Alias, Expr, ExprTrait, PostgresQueryBuilder, Query, SqliteQueryBuilder};
use fexu_db::driver::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, Value as DbValue,
};

const PROJECT: &str = "_fexu_project";
const ROW: &str = "project";
const ICON_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct ProjectRecord {
    pub branding: AdminBranding,
    pub icons: ProjectIcons,
    pub favicon_file: Option<(String, Vec<u8>)>,
    pub apple_file: Option<(String, Vec<u8>)>,
}

pub(crate) enum IconChoice {
    Keep,
    Clear,
    Replace(Vec<u8>),
}

pub(crate) async fn ensure(registry: &AdminRegistry, db: &DatabaseConnection) {
    if registry.project_loaded() {
        return;
    }
    match load(db).await {
        Ok(Some(record)) => {
            registry.apply_project(record.branding, record.icons);
            registry.mark_project_loaded();
        }
        Ok(None) => registry.mark_project_loaded(),
        Err(error) => {
            tracing::warn!(%error, "Projektangaben wurden nicht geladen");
        }
    }
}

pub(crate) async fn load(db: &DatabaseConnection) -> Result<Option<ProjectRecord>, FexuError> {
    ensure_table(db).await?;
    let backend = db.get_database_backend();
    let mut query = Query::select();
    query
        .columns([
            Alias::new("site_title"),
            Alias::new("site_header"),
            Alias::new("app_name"),
            Alias::new("primary_color"),
            Alias::new("default_locale"),
            Alias::new("fallback_locale"),
            Alias::new("currency"),
            Alias::new("favicon"),
            Alias::new("apple_touch_icon"),
            Alias::new("favicon_type"),
            Alias::new("favicon_bytes"),
            Alias::new("apple_touch_type"),
            Alias::new("apple_touch_bytes"),
        ])
        .from(Alias::new(PROJECT))
        .and_where(Expr::col(Alias::new("id")).eq(ROW));
    let rows = fetch(db, backend, &query).await?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    let text = |name: &str| -> Result<String, FexuError> {
        row.try_get::<String>("", name).map_err(FexuError::database)
    };
    let branding = AdminBranding {
        site_title: text("site_title")?,
        site_header: text("site_header")?,
        app_name: text("app_name")?,
        primary_color: text("primary_color")?,
        default_locale: text("default_locale")?,
        fallback_locale: text("fallback_locale")?,
        currency: text("currency")?,
    };
    let icons = ProjectIcons {
        favicon: clean_icon(&text("favicon")?).unwrap_or_default(),
        apple_touch_icon: clean_icon(&text("apple_touch_icon")?).unwrap_or_default(),
    };
    let favicon_file = icon_file(&row, "favicon_bytes")?;
    let apple_file = icon_file(&row, "apple_touch_bytes")?;
    Ok(Some(ProjectRecord {
        branding,
        icons,
        favicon_file,
        apple_file,
    }))
}

fn icon_file(
    row: &fexu_db::driver::QueryResult,
    bytes_column: &str,
) -> Result<Option<(String, Vec<u8>)>, FexuError> {
    let bytes = row
        .try_get::<Vec<u8>>("", bytes_column)
        .or_else(|_| {
            row.try_get::<Option<Vec<u8>>>("", bytes_column)
                .map(|value| value.unwrap_or_default())
        })
        .map_err(FexuError::database)?;
    let Some(kind) = image_kind(&bytes) else {
        return Ok(None);
    };
    Ok(Some((kind.to_owned(), bytes)))
}

pub(crate) async fn save(db: &DatabaseConnection, record: &ProjectRecord) -> Result<(), FexuError> {
    ensure_table(db).await?;
    let backend = db.get_database_backend();
    let mut delete = Query::delete();
    delete
        .from_table(Alias::new(PROJECT))
        .and_where(Expr::col(Alias::new("id")).eq(ROW));
    execute(db, backend, &delete).await?;
    let branding = &record.branding;
    let mut insert = Query::insert();
    insert
        .into_table(Alias::new(PROJECT))
        .columns([
            Alias::new("id"),
            Alias::new("site_title"),
            Alias::new("site_header"),
            Alias::new("app_name"),
            Alias::new("primary_color"),
            Alias::new("default_locale"),
            Alias::new("fallback_locale"),
            Alias::new("currency"),
            Alias::new("favicon"),
            Alias::new("apple_touch_icon"),
            Alias::new("favicon_type"),
            Alias::new("favicon_bytes"),
            Alias::new("apple_touch_type"),
            Alias::new("apple_touch_bytes"),
        ])
        .values_panic([
            DbValue::from(ROW).into(),
            DbValue::from(branding.site_title.as_str()).into(),
            DbValue::from(branding.site_header.as_str()).into(),
            DbValue::from(branding.app_name.as_str()).into(),
            DbValue::from(branding.primary_color.as_str()).into(),
            DbValue::from(branding.default_locale.as_str()).into(),
            DbValue::from(branding.fallback_locale.as_str()).into(),
            DbValue::from(branding.currency.as_str()).into(),
            DbValue::from(record.icons.favicon.as_str()).into(),
            DbValue::from(record.icons.apple_touch_icon.as_str()).into(),
            DbValue::from(
                record
                    .favicon_file
                    .as_ref()
                    .map(|(kind, _)| kind.as_str())
                    .unwrap_or(""),
            )
            .into(),
            blob_value(
                record
                    .favicon_file
                    .as_ref()
                    .map(|(_, bytes)| bytes.as_slice()),
            ),
            DbValue::from(
                record
                    .apple_file
                    .as_ref()
                    .map(|(kind, _)| kind.as_str())
                    .unwrap_or(""),
            )
            .into(),
            blob_value(
                record
                    .apple_file
                    .as_ref()
                    .map(|(_, bytes)| bytes.as_slice()),
            ),
        ]);
    execute(db, backend, &insert).await
}

pub(crate) fn parse(fields: &[(&str, &str)]) -> Option<ProjectRecord> {
    let value = |name: &str| {
        fields
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.trim())
            .unwrap_or("")
    };
    let site_title = value("site_title");
    let site_header = value("site_header");
    let app_name = value("app_name");
    let primary_color = value("primary_color").to_ascii_lowercase();
    let default_locale = value("default_locale");
    let fallback_locale = value("fallback_locale");
    let currency = value("currency").to_ascii_uppercase();
    if !is_public_label(site_title)
        || !is_public_label(site_header)
        || !is_public_label(app_name)
        || !is_hex_color(&primary_color)
        || !matches!(default_locale, "de" | "en")
        || !matches!(fallback_locale, "de" | "en")
        || currency.len() != 3
        || !currency.chars().all(|char| char.is_ascii_uppercase())
    {
        return None;
    }
    Some(ProjectRecord {
        branding: AdminBranding {
            site_title: site_title.to_owned(),
            site_header: site_header.to_owned(),
            app_name: app_name.to_owned(),
            primary_color,
            default_locale: default_locale.to_owned(),
            fallback_locale: fallback_locale.to_owned(),
            currency,
        },
        icons: ProjectIcons::default(),
        favicon_file: None,
        apple_file: None,
    })
}

pub(crate) fn choose_icon(clear: bool, bytes: &[u8]) -> Option<IconChoice> {
    if !bytes.is_empty() {
        image_kind(bytes)?;
        return Some(IconChoice::Replace(bytes.to_vec()));
    }
    Some(if clear {
        IconChoice::Clear
    } else {
        IconChoice::Keep
    })
}

pub(crate) fn apply_icon(
    current_href: &str,
    current_file: Option<(String, Vec<u8>)>,
    choice: IconChoice,
    route: &str,
) -> (String, Option<(String, Vec<u8>)>) {
    match choice {
        IconChoice::Keep => (current_href.to_owned(), current_file),
        IconChoice::Clear => (String::new(), None),
        IconChoice::Replace(bytes) => match image_kind(&bytes) {
            Some(kind) => (icon_href(route, &bytes), Some((kind.to_owned(), bytes))),
            None => (current_href.to_owned(), current_file),
        },
    }
}

pub(crate) fn image_kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.is_empty() || bytes.len() > ICON_BYTES {
        return None;
    }
    if let Some(kind) = fexu_storage::detect_type(bytes) {
        return match kind {
            "image/png" | "image/jpeg" | "image/gif" | "image/webp" => Some(kind),
            _ => None,
        };
    }
    if is_ico(bytes) {
        Some("image/x-icon")
    } else {
        None
    }
}

pub(crate) fn icon_href(route: &str, bytes: &[u8]) -> String {
    let mut stamp = bytes.len() as u32;
    for (index, byte) in bytes.iter().take(8).enumerate() {
        stamp ^= u32::from(*byte) << ((index % 4) * 8);
    }
    format!("/admin/brand/{route}?v={stamp:08x}")
}

fn is_ico(bytes: &[u8]) -> bool {
    bytes.len() >= 6
        && bytes[0] == 0
        && bytes[1] == 0
        && bytes[2] == 1
        && bytes[3] == 0
        && u16::from_le_bytes([bytes[4], bytes[5]]) >= 1
}

fn blob_value(bytes: Option<&[u8]>) -> fexu_db::driver::sea_query::SimpleExpr {
    DbValue::Bytes(bytes.map(|bytes| bytes.to_vec())).into()
}

pub(crate) fn clean_icon(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return Some(String::new());
    }
    if value.len() > 200
        || value.chars().any(|char| {
            char.is_control()
                || char.is_whitespace()
                || matches!(char, '<' | '>' | '"' | '\'' | '\\')
        })
    {
        return None;
    }
    if let Some(path) = value.strip_prefix('/') {
        if path.starts_with('/') || path.contains("..") {
            return None;
        }
        return Some(value.to_owned());
    }
    if value.starts_with("https://") || value.starts_with("http://") {
        return Some(value.to_owned());
    }
    None
}

async fn ensure_table(db: &DatabaseConnection) -> Result<(), FexuError> {
    let backend = db.get_database_backend();
    let bytes = match backend {
        DatabaseBackend::Postgres => "BYTEA",
        DatabaseBackend::Sqlite => "BLOB",
        DatabaseBackend::MySql => {
            return Err(FexuError::Validation("MySQL is not supported.".into()))
        }
    };
    db.execute(Statement::from_string(
        backend,
        "CREATE TABLE IF NOT EXISTS _fexu_project (
            id TEXT PRIMARY KEY NOT NULL,
            site_title TEXT NOT NULL,
            site_header TEXT NOT NULL,
            app_name TEXT NOT NULL,
            primary_color TEXT NOT NULL,
            default_locale TEXT NOT NULL,
            fallback_locale TEXT NOT NULL,
            currency TEXT NOT NULL,
            favicon TEXT NOT NULL,
            apple_touch_icon TEXT NOT NULL,
            favicon_type TEXT NOT NULL DEFAULT '',
            favicon_bytes BLOB,
            apple_touch_type TEXT NOT NULL DEFAULT '',
            apple_touch_bytes BLOB
        )"
        .replace("BLOB", bytes),
    ))
    .await
    .map_err(FexuError::database)?;
    for statement in [
        "ALTER TABLE _fexu_project ADD COLUMN favicon_type TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE _fexu_project ADD COLUMN favicon_bytes BLOB",
        "ALTER TABLE _fexu_project ADD COLUMN apple_touch_type TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE _fexu_project ADD COLUMN apple_touch_bytes BLOB",
    ] {
        let _ = db
            .execute(Statement::from_string(
                backend,
                statement.replace("BLOB", bytes),
            ))
            .await;
    }
    Ok(())
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
            return Err(FexuError::Validation("MySQL is not supported.".into()))
        }
    };
    Ok((sql, values.0))
}
