//! Shared contracts, independent of HTTP and database implementations.

pub mod i18n;

pub mod choice;
pub mod civil;
pub mod decimal;
mod messages;
mod problem;
pub mod temporal;
mod validation;

pub use choice::{choice_is_known, choice_label, ChoiceValue, FexuChoice};
pub use civil::{
    display_datetime_local, format_civil_datetime, parse_civil_datetime, parse_filter_datetime,
    parse_form_datetime, project_zone_from_env, CivilFold, LocalDateTimeInput, ProjectTimeZone,
};
pub use decimal::{
    decimal_sql_literal, decode_decimal, encode_decimal, format_decimal, format_decimal_text,
    parse_decimal, prepare_decimal, validate_decimal_meta, DecimalError,
};
pub use messages::{localize_detail, message_for, message_text, FieldOverride};
pub use problem::problem_json;
pub use rust_decimal::Decimal;
pub use temporal::{
    hierarchy_bound, parse_date, parse_datetime_local, parse_datetime_utc, parse_time, DateBound,
};
pub use validation::{is_email, is_http_url, parse_bool, parse_i64, parse_uuid, text_code};

#[derive(Debug, Clone, Copy)]
pub struct ModelDescriptor {
    pub name: &'static str,
    pub table: &'static str,
    pub api_path: &'static str,
    /// Explicit opt-in for unauthenticated access (demo only).
    pub allow_anonymous: bool,
    /// `None` denies access unless `allow_anonymous` is set.
    /// `Some("authenticated")` requires an active session.
    /// Any other value names the resource for `{name}.read|create|update|delete`.
    pub permission: Option<&'static str>,
    /// Object policies provide a `QueryScope`. Routes remain denied without a bound policy.
    pub requires_object_policy: bool,
}

/// Comparison value in a query scope. It comes from the policy, not the HTTP request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeValue {
    Text(String),
    Uuid(uuid::Uuid),
    BigInt(i64),
    Bool(bool),
}

/// Database filter from an object policy. `None` matches no rows; `All` requires explicit policy authorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryScope {
    All,
    None,
    Equals {
        column: &'static str,
        value: ScopeValue,
    },
}

/// Named form section. Fields retain their struct declaration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdminFieldset {
    pub name: &'static str,
    pub description: &'static str,
    pub optional: bool,
}

/// Admin field. Values come from the same model as the schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdminFieldMeta {
    pub name: &'static str,
    pub label: &'static str,
    pub list_display: bool,
    pub list_link: bool,
    pub searchable: bool,
    pub sortable: bool,
    pub filterable: bool,
    pub readonly: bool,
    pub hidden: bool,
    /// Field values must never appear in drafts or audit history.
    pub sensitive: bool,
    /// Empty when the model declares no fieldsets.
    pub fieldset: &'static str,
}

/// Has-many section. `form` saves parent and children in one transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdminInline {
    pub name: &'static str,
    pub child_table: &'static str,
    pub column: &'static str,
    /// Empty: the interface uses the child model label.
    pub label: &'static str,
    /// Empty: all writable fields except the parent foreign key.
    pub fields: &'static [&'static str],
    pub readonly_fields: &'static [&'static str],
    pub min_count: u16,
    /// `0` without `form` disables the combined form.
    pub max_count: u16,
    pub extra: u16,
    pub can_add: bool,
    pub can_change: bool,
    pub can_delete: bool,
    pub ordering: Option<&'static str>,
    pub form: bool,
}

/// Admin exposure of a model. `None` from `admin_meta` means the model is absent from admin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdminModelMeta {
    pub key: &'static str,
    pub label: &'static str,
    /// `None`: active superuser only. Otherwise `{permission}.read|create|update|delete` applies.
    pub permission: Option<&'static str>,
    pub fields: &'static [AdminFieldMeta],
    /// `Some("-name")` sorts descending. `None` sorts by `id` ascending.
    pub ordering: Option<&'static str>,
    /// HTML page size. The server caps this at 100.
    pub per_page: u16,
    pub fieldsets: &'static [AdminFieldset],
    pub inlines: &'static [AdminInline],
    /// `Some(field)` enables UTC date navigation in lists; only `Date` or `DateTime`.
    pub date_hierarchy: Option<&'static str>,
}

/// Database backend targeted by Fexu DDL and queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseKind {
    Sqlite,
    Postgres,
}

/// Column type independent of SQL dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    Uuid,
    Text,
    BigInt,
    Boolean,
    /// Calendar date without time or timezone. Canonical format: `YYYY-MM-DD`.
    Date,
    /// Time of day without date. Canonical format: `HH:MM:SS`.
    Time,
    /// Absolute UTC instant. Canonical format: RFC 3339 with `Z`.
    DateTime,
    /// Exact decimal value. Storage format: 16-byte coefficient; see `decimal`.
    Decimal,
    /// Enum variant with a fixed stored string. Allowed values are in `ColumnSpec::choices`.
    Choice,
    /// JSON document. PostgreSQL stores `jsonb`; SQLite stores canonical text.
    /// Path operators are not part of the portable contract.
    Json,
}

/// Action for `ON DELETE`. `ON UPDATE` remains `NO ACTION`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferentialAction {
    Restrict,
    Cascade,
    SetNull,
}

/// Metadata for a model column. The macro generates it; the database layer renders it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnSpec {
    pub name: &'static str,
    pub kind: ColumnKind,
    pub nullable: bool,
    pub primary_key: bool,
    pub default_bool: Option<bool>,
    pub unique: bool,
    pub indexed: bool,
    pub foreign_table: Option<&'static str>,
    pub foreign_column: Option<&'static str>,
    pub on_delete: Option<ReferentialAction>,
    /// `0` for non-`Decimal` columns; otherwise `1..=28`.
    pub precision: u8,
    /// Fractional digits. Only set for `Decimal` and always `<= precision`.
    pub scale: u8,
    /// Empty except for `Choice`.
    pub choices: &'static [ChoiceValue],
}

/// Composite unique constraints and indexes. Empty lists are the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableConstraints {
    pub unique_groups: &'static [&'static [&'static str]],
    pub index_groups: &'static [&'static [&'static str]],
}

impl TableConstraints {
    pub const EMPTY: Self = Self {
        unique_groups: &[],
        index_groups: &[],
    };
}

impl Default for TableConstraints {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// Server-side numeric limits declared on a model field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumericBound {
    pub column: &'static str,
    pub min: Option<i64>,
    pub max: Option<i64>,
}

/// Write and validation rules emitted by the model derive.
///
/// The same field attributes that build `ColumnSpec` and `NumericBound` fill this
/// contract. Hand-written models inherit the empty default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldContract {
    pub column: &'static str,
    pub in_create: bool,
    pub in_update: bool,
    pub create_required: bool,
    pub min_length: Option<u32>,
    pub max_length: Option<u32>,
    /// `"slug"` or `"email"` when the field declares `format`.
    pub format: Option<&'static str>,
    /// `false` omits the field from generated read JSON. Create and update stay on `in_create` and `in_update`.
    pub api_read: bool,
}

pub trait FexuModel: Send + Sync + 'static {
    fn descriptor() -> &'static ModelDescriptor;
    fn columns() -> &'static [ColumnSpec];
    fn constraints() -> &'static TableConstraints {
        &TableConstraints::EMPTY
    }
    fn schema_sql(kind: DatabaseKind) -> String;
    fn admin_meta() -> Option<&'static AdminModelMeta> {
        None
    }
    /// Limits from `#[field(min = ..., max = ...)]`. Empty when the model has none.
    fn numeric_bounds() -> &'static [NumericBound] {
        &[]
    }

    /// Create, update and text rules from the same derive. Empty for hand-written models.
    fn field_contracts() -> &'static [FieldContract] {
        &[]
    }
}

pub fn unique_index_name(table: &str, columns: &[&str]) -> String {
    object_name("uq", table, columns)
}

pub fn secondary_index_name(table: &str, columns: &[&str]) -> String {
    object_name("ix", table, columns)
}

pub fn foreign_key_name(
    table: &str,
    column: &str,
    referenced_table: &str,
    referenced_column: &str,
) -> String {
    format!("fk_{table}_{column}_{referenced_table}_{referenced_column}")
}

/// Compact JSON. Object keys follow the `serde_json` map, sorted alphabetically without `preserve_order`.
///
/// A JSON `null` is a document, not SQL `NULL`. Empty text and invalid JSON
/// are rejected. PostgreSQL `jsonb` operators are not generated.
pub fn canonical_json_text(text: &str) -> Result<String, &'static str> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|_| "validation.json")?;
    serde_json::to_string(&value).map_err(|_| "validation.json")
}

fn object_name(prefix: &str, table: &str, columns: &[&str]) -> String {
    let mut name = format!("{prefix}_{table}");
    for column in columns {
        name.push('_');
        name.push_str(column);
    }
    name
}

#[derive(Debug, thiserror::Error)]
pub enum FexuError {
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("permission denied")]
    Forbidden,
    #[error("{0}")]
    Denied(String),
    #[error("authentication required")]
    Unauthorized(String),
    #[error("resource not found")]
    NotFound,
    #[error("database operation failed")]
    Database(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl FexuError {
    pub fn database<E: std::error::Error + Send + Sync + 'static>(error: E) -> Self {
        Self::Database(Box::new(error))
    }
}

/// Reads one `FEXU_<NAME>` variable.
///
/// `DATABASE_URL` and `BIND_ADDRESS` are not framework settings, so this
/// function rejects those names instead of treating them as configuration.
pub fn aliased_env(name: &str) -> Result<Option<String>, String> {
    let Some(rest) = name.strip_prefix("FEXU_") else {
        return Err(format!(
            "{name} is not a FEXU_ setting. DATABASE_URL and BIND_ADDRESS stay unchanged."
        ));
    };
    if rest.is_empty()
        || !rest
            .chars()
            .all(|char| char.is_ascii_uppercase() || char.is_ascii_digit() || char == '_')
    {
        return Err(format!("{name} is not a FEXU_ setting."));
    }
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid Unicode.")),
    }
}

/// Joins `Display` and `source` for server logs.
///
/// The result can contain SQL text from a database driver. Do not write it to
/// an HTTP response. Values that follow `password`, `secret`, `token`,
/// `csrf_token` or a Fexu session cookie name are replaced.
pub fn diagnostic_chain(error: &dyn std::error::Error) -> String {
    let mut parts = Vec::new();
    let mut current = Some(error);
    let mut depth = 0;
    while let Some(item) = current {
        if depth == 8 {
            break;
        }
        parts.push(redact_secrets(&item.to_string()));
        current = item.source();
        depth += 1;
    }
    parts.join(" | ")
}

fn redact_secrets(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    while cursor < text.len() {
        if let Some((key_len, value_end)) = secret_at(&lower, cursor) {
            result.push_str(&text[cursor..cursor + key_len]);
            result.push_str("[redacted]");
            cursor = value_end;
            continue;
        }
        let next = text[cursor..].chars().next().unwrap_or('\u{FFFD}');
        result.push(next);
        cursor += next.len_utf8();
    }
    result
}

fn secret_at(lower: &str, index: usize) -> Option<(usize, usize)> {
    const KEYS: [&str; 6] = [
        "password=",
        "secret=",
        "token=",
        "csrf_token=",
        "fexu_session=",
        "fexu_anonymous=",
    ];
    let rest = lower.get(index..)?;
    let key_len = KEYS
        .iter()
        .copied()
        .find(|key| rest.starts_with(key))
        .map(str::len)
        .or_else(|| namespaced_cookie_key(rest))?;
    let mut end = index + key_len;
    let bytes = lower.as_bytes();
    while end < bytes.len() && !matches!(bytes[end], b' ' | b'\n' | b'\r' | b'\t' | b'&' | b';') {
        end += 1;
    }
    Some((key_len, end))
}

/// Session, anonymous, login, reset and exchange cookies, including app namespaces.
fn namespaced_cookie_key(rest: &str) -> Option<usize> {
    let name_end = rest.find('=')?;
    if name_end == 0 || name_end > 64 {
        return None;
    }
    let name = &rest[..name_end];
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return None;
    }
    let sensitive = name.ends_with("_session")
        || name.ends_with("_anonymous")
        || name.ends_with("_login_csrf")
        || name.ends_with("_reset_csrf")
        || (name.ends_with("_reset") && name != "password_reset");
    sensitive.then_some(name_end + 1)
}

/// Models with both managed timestamps.
pub trait HasTimestamps {
    fn created_at(&self) -> chrono::DateTime<chrono::Utc>;
    fn updated_at(&self) -> chrono::DateTime<chrono::Utc>;
}

thread_local! {
    static FIXED_CLOCK: std::cell::Cell<Option<chrono::DateTime<chrono::Utc>>> =
        const { std::cell::Cell::new(None) };
}

/// UTC instant for managed fields. Tests can override it through [`set_test_clock`].
pub fn utc_now() -> chrono::DateTime<chrono::Utc> {
    FIXED_CLOCK.with(|slot| slot.get().unwrap_or_else(chrono::Utc::now))
}

/// Overrides the current thread's clock. `None` restores system time.
pub fn set_test_clock(now: Option<chrono::DateTime<chrono::Utc>>) {
    FIXED_CLOCK.with(|slot| slot.set(now));
}

pub fn check_slug(field: &str, value: &str) -> Result<(), FexuError> {
    let ok = !value.is_empty()
        && value.len() <= 80
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .chars()
            .all(|char| char.is_ascii_lowercase() || char.is_ascii_digit() || char == '-');
    if ok {
        Ok(())
    } else {
        Err(FexuError::Validation(format!(
            "field:{field}: validation.slug"
        )))
    }
}

pub fn check_email(field: &str, value: &str) -> Result<(), FexuError> {
    let Some((local, domain)) = value.split_once('@') else {
        return Err(FexuError::Validation(format!(
            "field:{field}: validation.email"
        )));
    };
    let labels = domain.split('.').collect::<Vec<_>>();
    let ok = !local.is_empty()
        && local.len() <= 64
        && !local
            .chars()
            .any(|char| char.is_whitespace() || char.is_control())
        && labels.len() >= 2
        && labels
            .iter()
            .all(|label| !label.is_empty() && label.len() <= 63)
        && domain.len() <= 255
        && !domain
            .chars()
            .any(|char| char.is_whitespace() || char.is_control() || char == '@');
    if ok {
        Ok(())
    } else {
        Err(FexuError::Validation(format!(
            "field:{field}: validation.email"
        )))
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::{diagnostic_chain, FexuError};

    #[test]
    fn database_display_hides_the_source_and_the_chain_redacts_secrets() {
        let error = FexuError::database(std::io::Error::other(
            "password=hunter2 csrf_token=abc123 near users",
        ));
        assert_eq!(error.to_string(), "database operation failed");
        assert!(!error.to_string().contains("hunter2"));
        let chain = diagnostic_chain(&error);
        assert!(chain.contains("database operation failed"));
        assert!(chain.contains("password=[redacted]"));
        assert!(chain.contains("csrf_token=[redacted]"));
        assert!(!chain.contains("hunter2"));
        assert!(!chain.contains("abc123"));
        let namespaced = diagnostic_chain(&FexuError::database(std::io::Error::other(
            "salon_session=sekret shop_login_csrf=deadbeef fexu_reset=exchange status=keep",
        )));
        assert!(namespaced.contains("salon_session=[redacted]"));
        assert!(namespaced.contains("shop_login_csrf=[redacted]"));
        assert!(namespaced.contains("fexu_reset=[redacted]"));
        assert!(namespaced.contains("status=keep"));
        assert!(!namespaced.contains("sekret"));
        assert!(!namespaced.contains("deadbeef"));
        assert!(!namespaced.contains("exchange"));
    }
}

#[cfg(test)]
mod json_tests {
    use super::canonical_json_text;

    #[test]
    fn canonical_json_keeps_documents_distinct_from_empty_text() {
        assert_eq!(
            canonical_json_text(" { \"b\": 1, \"a\": null } ").unwrap(),
            "{\"a\":null,\"b\":1}"
        );
        assert_eq!(canonical_json_text("null").unwrap(), "null");
        assert!(canonical_json_text("").is_err());
        assert!(canonical_json_text("{").is_err());
    }
}

#[cfg(test)]
mod alias_tests {
    use super::aliased_env;

    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn framework_env_reads_one_variable() {
        let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        unsafe {
            std::env::set_var("FEXU_ALIAS_TEST", "one");
        }
        assert_eq!(
            aliased_env("FEXU_ALIAS_TEST").unwrap().as_deref(),
            Some("one")
        );
        unsafe {
            std::env::remove_var("FEXU_ALIAS_TEST");
        }
        assert_eq!(aliased_env("FEXU_ALIAS_TEST").unwrap(), None);
        let error = aliased_env("DATABASE_URL").unwrap_err();
        assert!(error.contains("DATABASE_URL"));
        assert!(!error.contains("one"));
    }
}
