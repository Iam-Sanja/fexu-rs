//! Disposable PostgreSQL databases for tests.
//!
//! `create` only accepts an administrator URL whose database name is one of
//! the documented test databases. It creates `fexu_test_<32 hex>` by cloning
//! `template0`. Administrative sessions open the approved database, or
//! `postgres` when that name is `template0` or `template1`. They never sit on
//! the database being cloned. `drop_database` drops that generated name only,
//! and only when the current role owns it.

use std::time::Duration;

use crate::driver::{
    ConnectOptions, ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, Statement,
    Value,
};

const CLONE_TEMPLATE: &str = "template0";
const TEMPLATE_BUSY: &str = "55006";
const DUPLICATE_DATABASE: &str = "42P04";
const CREATE_ATTEMPTS: u32 = 5;

pub struct DisposablePostgres {
    url: String,
    db_name: String,
    admin_prefix: String,
    admin_database: String,
}

impl DisposablePostgres {
    pub async fn create(admin_url: &str) -> Result<Self, String> {
        let (prefix, admin_name) = split_database_url(admin_url)?;
        if !approved_admin_database(&admin_name) {
            return Err(format!(
                "Refusing to create a test database from '{admin_name}'. Use fexu, fexu_migrations_down_test, postgres, template1, or a database whose name contains fexu_test."
            ));
        }
        let db_name = format!("fexu_test_{}", uuid::Uuid::new_v4().simple());
        if !is_disposable_name(&db_name) {
            return Err("The generated test database name is not disposable.".into());
        }
        let (admin, session_db) = admin_connection(&prefix, &admin_name).await?;
        let created = create_database(&admin, &db_name).await;
        if let Err(error) = created {
            let _ = admin.close().await;
            return Err(error);
        }
        if let Err(error) = admin.close().await {
            let message = redact(&error.to_string());
            return match drop_owned_database(&prefix, &session_db, &db_name).await {
                Ok(()) => Err(format!(
                    "Could not close the admin connection after creating {db_name}, so that database was dropped: {message}"
                )),
                Err(drop_error) => Err(format!(
                    "Could not close the admin connection after creating {db_name}: {message}. Cleanup also failed: {drop_error}"
                )),
            };
        }
        Ok(Self {
            url: format!("{prefix}/{db_name}"),
            db_name,
            admin_prefix: prefix,
            admin_database: session_db,
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn name(&self) -> &str {
        &self.db_name
    }

    pub async fn drop_database(self) -> Result<(), String> {
        drop_owned_database(&self.admin_prefix, &self.admin_database, &self.db_name).await
    }
}

fn approved_admin_database(name: &str) -> bool {
    matches!(
        name,
        "fexu" | "fexu_migrations_down_test" | "postgres" | "template1"
    ) || (name.contains("fexu_test") && is_safe_database_name(name))
}

fn is_clone_source(name: &str) -> bool {
    matches!(name, "template0" | "template1")
}

fn is_disposable_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("fexu_test_") else {
        return false;
    };
    rest.len() == 32 && rest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_safe_database_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(char) if char.is_ascii_alphabetic() || char == '_')
        && chars.all(|char| char.is_ascii_alphanumeric() || char == '_')
        && name.len() <= 63
}

fn quote_ident(name: &str) -> Result<String, String> {
    if !is_safe_database_name(name) {
        return Err(format!("Refusing to quote database name '{name}'."));
    }
    Ok(format!("\"{name}\""))
}

fn create_database_sql(name: &str) -> Result<String, String> {
    if !is_disposable_name(name) {
        return Err(format!(
            "Refusing to create '{name}' because it is not a disposable test database."
        ));
    }
    Ok(format!(
        "CREATE DATABASE {} TEMPLATE {}",
        quote_ident(name)?,
        quote_ident(CLONE_TEMPLATE)?
    ))
}

fn drop_database_sql(name: &str) -> Result<String, String> {
    if !is_disposable_name(name) {
        return Err(format!(
            "Refusing to drop '{name}' because this helper did not create it."
        ));
    }
    Ok(format!(
        "DROP DATABASE IF EXISTS {} WITH (FORCE)",
        quote_ident(name)?
    ))
}

fn admin_session_candidates(explicit: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    if approved_admin_database(explicit) && !is_clone_source(explicit) {
        candidates.push(explicit.to_owned());
    }
    if !candidates.iter().any(|name| name == "postgres") {
        candidates.push("postgres".to_owned());
    }
    candidates
}

fn split_database_url(url: &str) -> Result<(String, String), String> {
    if !url.starts_with("postgres://") && !url.starts_with("postgresql://") {
        return Err("The isolated PostgreSQL helper accepts only a postgres URL.".into());
    }
    let without_query = url.split('?').next().unwrap_or(url);
    let (prefix, name) = without_query
        .rsplit_once('/')
        .ok_or_else(|| "The PostgreSQL URL has no database name.".to_owned())?;
    if name.is_empty() || name.contains('@') || !is_safe_database_name(name) {
        return Err("The PostgreSQL URL has no usable database name.".into());
    }
    Ok((prefix.to_owned(), name.to_owned()))
}

async fn admin_connection(
    prefix: &str,
    explicit_db: &str,
) -> Result<(DatabaseConnection, String), String> {
    let mut errors = Vec::new();
    for database in admin_session_candidates(explicit_db) {
        let mut options = ConnectOptions::new(format!("{prefix}/{database}"));
        options.max_connections(1);
        match Database::connect(options).await {
            Ok(connection) => return Ok((connection, database)),
            Err(error) => errors.push(redact(&error.to_string())),
        }
    }
    Err(format!(
        "Could not open a PostgreSQL admin connection. {}",
        errors.join(" | ")
    ))
}

async fn create_database(admin: &DatabaseConnection, db_name: &str) -> Result<(), String> {
    let sql = create_database_sql(db_name)?;
    let mut last = String::new();
    for attempt in 1..=CREATE_ATTEMPTS {
        match admin
            .execute(Statement::from_string(
                DatabaseBackend::Postgres,
                sql.clone(),
            ))
            .await
        {
            Ok(_) => return Ok(()),
            Err(error) if error.sql_state() == Some(DUPLICATE_DATABASE) => {
                return match database_ownership(admin, db_name).await? {
                    Ownership::Owned => Ok(()),
                    Ownership::Absent | Ownership::Foreign => Err(format!(
                        "Refusing to adopt '{db_name}' because the current role does not own it."
                    )),
                };
            }
            Err(error) => {
                let message = redact(&error.to_string());
                if error.sql_state() != Some(TEMPLATE_BUSY) || attempt == CREATE_ATTEMPTS {
                    return Err(format!(
                        "Could not create isolated test database {db_name}: {message}"
                    ));
                }
                last = message;
                tokio::time::sleep(template_retry_delay(attempt)).await;
            }
        }
    }
    Err(format!(
        "Could not create isolated test database {db_name}: {last}"
    ))
}

async fn drop_owned_database(
    prefix: &str,
    admin_database: &str,
    db_name: &str,
) -> Result<(), String> {
    if !is_disposable_name(db_name) {
        return Err(format!(
            "Refusing to drop '{db_name}' because this helper did not create it."
        ));
    }
    let (admin, _) = admin_connection(prefix, admin_database).await?;
    let outcome = drop_on_connection(&admin, db_name).await;
    let close = admin
        .close()
        .await
        .map_err(|error| redact(&error.to_string()));
    match (outcome, close) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(error)) => Err(format!(
            "Dropped {db_name}, but closing the admin connection failed: {error}"
        )),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(close_error)) => {
            Err(format!("{error}; admin close also failed: {close_error}"))
        }
    }
}

async fn drop_on_connection(admin: &DatabaseConnection, db_name: &str) -> Result<(), String> {
    match database_ownership(admin, db_name).await? {
        Ownership::Absent => Ok(()),
        Ownership::Foreign => Err(format!(
            "Refusing to drop '{db_name}' because the current role does not own it."
        )),
        Ownership::Owned => {
            let _ = admin
                .execute(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()",
                    [Value::String(Some(Box::new(db_name.to_owned())))],
                ))
                .await;
            drop_database_statement(admin, db_name).await
        }
    }
}

async fn drop_database_statement(admin: &DatabaseConnection, db_name: &str) -> Result<(), String> {
    let sql = drop_database_sql(db_name)?;
    let mut last = String::new();
    for attempt in 1..=CREATE_ATTEMPTS {
        match admin
            .execute(Statement::from_string(
                DatabaseBackend::Postgres,
                sql.clone(),
            ))
            .await
        {
            Ok(_) => return Ok(()),
            Err(error) => {
                let message = redact(&error.to_string());
                if error.sql_state() != Some(TEMPLATE_BUSY) || attempt == CREATE_ATTEMPTS {
                    return Err(format!(
                        "Could not drop isolated test database {db_name}: {message}"
                    ));
                }
                let _ = admin
                    .execute(Statement::from_sql_and_values(
                        DatabaseBackend::Postgres,
                        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()",
                        [Value::String(Some(Box::new(db_name.to_owned())))],
                    ))
                    .await;
                last = message;
                tokio::time::sleep(template_retry_delay(attempt)).await;
            }
        }
    }
    Err(format!(
        "Could not drop isolated test database {db_name}: {last}"
    ))
}

enum Ownership {
    Absent,
    Owned,
    Foreign,
}

async fn database_ownership(
    admin: &DatabaseConnection,
    db_name: &str,
) -> Result<Ownership, String> {
    if !is_disposable_name(db_name) {
        return Err(format!(
            "Refusing to inspect '{db_name}' because it is not a disposable test database."
        ));
    }
    let row = admin
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT datname AS datname, (pg_catalog.pg_get_userbyid(datdba) = CURRENT_USER) AS owned_by_current_user FROM pg_catalog.pg_database WHERE datname = $1",
            [Value::String(Some(Box::new(db_name.to_owned())))],
        ))
        .await
        .map_err(|error| redact(&error.to_string()))?;
    let Some(row) = row else {
        return Ok(Ownership::Absent);
    };
    let name: String = row
        .try_get("", "datname")
        .map_err(|error| redact(&error.to_string()))?;
    if name != db_name {
        return Err(format!(
            "Refusing to touch '{db_name}' because the catalog returned '{name}'."
        ));
    }
    let owned: bool = row
        .try_get("", "owned_by_current_user")
        .map_err(|error| redact(&error.to_string()))?;
    Ok(if owned {
        Ownership::Owned
    } else {
        Ownership::Foreign
    })
}

fn template_retry_delay(attempt: u32) -> Duration {
    Duration::from_millis(25 * u64::from(attempt))
}

fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(scheme) = rest.find("://") {
        out.push_str(&rest[..scheme + 3]);
        rest = &rest[scheme + 3..];
        if let Some(at) = rest.find('@') {
            let userinfo = &rest[..at];
            if !userinfo.contains('/') {
                out.push_str("redacted");
                rest = &rest[at..];
                continue;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use std::future::Future;

    use super::{
        admin_session_candidates, approved_admin_database, create_database_sql, drop_database_sql,
        drop_owned_database, is_disposable_name, redact, split_database_url,
    };
    use crate::driver::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};

    #[test]
    fn disposable_names_and_admin_targets_are_closed() {
        assert!(is_disposable_name(
            "fexu_test_0123456789abcdef0123456789abcdef"
        ));
        assert!(!is_disposable_name("fexu_test"));
        assert!(!is_disposable_name("fexu"));
        assert!(!is_disposable_name("postgres"));
        assert!(!is_disposable_name("template1"));
        assert!(!is_disposable_name("template0"));
        assert!(!is_disposable_name("fexu_test_short"));
        assert!(!is_disposable_name(
            "fexu_test_services_anon_20261112233529"
        ));
        assert!(approved_admin_database("fexu"));
        assert!(approved_admin_database("fexu_test"));
        assert!(!approved_admin_database("shop"));
        assert!(!approved_admin_database("ferrioxid"));
        assert!(!approved_admin_database("fexu_test;drop"));
        let secret = redact("postgres://fexu:fexu_dev_only@127.0.0.1:5432/fexu");
        assert!(!secret.contains("fexu_dev_only"));
        assert!(secret.contains("redacted@"));
    }

    #[test]
    fn admin_sessions_clone_template0_and_avoid_template1() {
        assert_eq!(
            admin_session_candidates("template1"),
            vec!["postgres".to_owned()]
        );
        assert_eq!(
            admin_session_candidates("template0"),
            vec!["postgres".to_owned()]
        );
        assert_eq!(
            admin_session_candidates("postgres"),
            vec!["postgres".to_owned()]
        );
        assert_eq!(
            admin_session_candidates("fexu"),
            vec!["fexu".to_owned(), "postgres".to_owned()]
        );
        assert_eq!(
            admin_session_candidates("shop"),
            vec!["postgres".to_owned()]
        );
        let name = "fexu_test_0123456789abcdef0123456789abcdef";
        let create_sql = create_database_sql(name).expect("create sql");
        assert!(create_sql.contains("TEMPLATE \"template0\""));
        assert!(!create_sql.contains("template1"));
        assert!(drop_database_sql(name)
            .expect("drop sql")
            .contains("DROP DATABASE IF EXISTS"));
        assert!(create_database_sql("template1").is_err());
        assert!(drop_database_sql("postgres").is_err());
        assert!(drop_database_sql("ferrioxid").is_err());
    }

    #[tokio::test]
    async fn create_refuses_an_unapproved_database_before_connecting() {
        let error =
            match super::DisposablePostgres::create("postgres://fexu:secret@127.0.0.1:5432/shop")
                .await
            {
                Ok(_) => panic!("an unapproved database was accepted"),
                Err(error) => error,
            };
        assert!(error.contains("shop"));
        assert!(error.contains("Refusing"));
        assert!(!error.contains("secret"));
    }

    fn loopback_admin_url() -> Result<String, String> {
        let url = std::env::var("FEXU_POSTGRES_ADMIN")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .map_err(|_| {
                "Set FEXU_POSTGRES_ADMIN or DATABASE_URL to a loopback PostgreSQL admin URL."
                    .to_owned()
            })?;
        if !is_loopback_postgres_url(&url) {
            return Err(
                "Refusing a PostgreSQL fixture test whose admin URL is not 127.0.0.1 or localhost."
                    .into(),
            );
        }
        let (_prefix, name) = split_database_url(&url)?;
        if !super::approved_admin_database(&name) {
            return Err(format!(
                "Refusing a PostgreSQL fixture test whose admin database is '{name}'."
            ));
        }
        Ok(url)
    }

    fn is_loopback_postgres_url(url: &str) -> bool {
        let Some(rest) = url
            .strip_prefix("postgres://")
            .or_else(|| url.strip_prefix("postgresql://"))
        else {
            return false;
        };
        let after_user = rest.rsplit_once('@').map(|(_, host)| host).unwrap_or(rest);
        let host = after_user.split([':', '/']).next().unwrap_or("");
        host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost")
    }

    async fn catalog(admin_url: &str) -> Result<Vec<String>, String> {
        let (prefix, name) = split_database_url(admin_url)?;
        let (admin, _) = super::admin_connection(&prefix, &name).await?;
        let rows = admin
            .query_all(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT datname AS datname FROM pg_catalog.pg_database ORDER BY datname".to_owned(),
            ))
            .await
            .map_err(|error| error.to_string())?;
        let mut names = Vec::new();
        for row in rows {
            names.push(
                row.try_get("", "datname")
                    .map_err(|error| error.to_string())?,
            );
        }
        admin.close().await.map_err(|error| error.to_string())?;
        Ok(names)
    }

    async fn server_version_num(admin_url: &str) -> Result<i64, String> {
        let (prefix, name) = split_database_url(admin_url)?;
        let (admin, _) = super::admin_connection(&prefix, &name).await?;
        let row = admin
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT current_setting('server_version_num') AS server_version_num".to_owned(),
            ))
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "PostgreSQL returned no server version.".to_owned())?;
        let text: String = row
            .try_get("", "server_version_num")
            .map_err(|error| error.to_string())?;
        admin.close().await.map_err(|error| error.to_string())?;
        text.parse::<i64>()
            .map_err(|_| format!("PostgreSQL server_version_num was '{text}'."))
    }

    // Only track databases created by this test invocation. A global catalog
    // difference is not proof of ownership: unrelated parallel tests can create
    // fexu_test_* databases at any point.
    type FixtureNames = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

    async fn tracked_create(
        admin: &str,
        names: &FixtureNames,
    ) -> Result<super::DisposablePostgres, String> {
        let database = super::DisposablePostgres::create(admin).await?;
        names
            .lock()
            .expect("fixture name registry")
            .push(database.name().to_owned());
        Ok(database)
    }

    async fn with_owned_fixture_cleanup<F, Fut>(admin_url: &str, body: F) -> Result<(), String>
    where
        F: FnOnce(FixtureNames) -> Fut,
        Fut: Future<Output = Result<(), String>>,
    {
        let names: FixtureNames = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let result = body(names.clone()).await;
        let created = names.lock().expect("fixture name registry").clone();
        let (prefix, admin_name) = split_database_url(admin_url)?;
        let mut cleanup_errors = Vec::new();
        for name in &created {
            // A second drop is safe: absent is accepted, foreign ownership is not.
            // Never infer what to drop from a cluster-wide pg_database diff.
            if let Err(error) = drop_owned_database(&prefix, &admin_name, name).await {
                cleanup_errors.push(error);
            }
        }
        let after = catalog(admin_url).await?;
        let remaining: Vec<_> = created
            .iter()
            .filter(|name| after.contains(name))
            .cloned()
            .collect();
        if !remaining.is_empty() || !cleanup_errors.is_empty() {
            return Err(format!(
                "Owned fixture cleanup incomplete. remaining={remaining:?} cleanup_errors={cleanup_errors:?} body={result:?}"
            ));
        }
        result
    }

    async fn open_one(url: &str) -> Result<DatabaseConnection, String> {
        let mut options = crate::driver::ConnectOptions::new(url.to_owned());
        options.max_connections(1);
        crate::driver::Database::connect(options)
            .await
            .map_err(|error| error.to_string())
    }

    async fn execute(db: &DatabaseConnection, sql: &str) -> Result<(), String> {
        db.execute(Statement::from_string(db.get_database_backend(), sql))
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    async fn scalar(db: &DatabaseConnection, sql: &str) -> Result<i64, String> {
        let row = db
            .query_one(Statement::from_string(db.get_database_backend(), sql))
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "the probe query returned no row".to_owned())?;
        row.try_get("", "total").map_err(|error| error.to_string())
    }

    async fn probe(db: &DatabaseConnection, value: i64) -> Result<(), String> {
        execute(
            db,
            "CREATE TABLE fexu_fixture_probe (id integer PRIMARY KEY)",
        )
        .await?;
        execute(
            db,
            &format!("INSERT INTO fexu_fixture_probe (id) VALUES ({value})"),
        )
        .await?;
        let total = scalar(db, "SELECT id AS total FROM fexu_fixture_probe").await?;
        if total != value {
            return Err(format!("expected probe {value}, read {total}"));
        }
        Ok(())
    }

    async fn create_while_template1_is_open(
        admin: &str,
        names: &FixtureNames,
    ) -> Result<(), String> {
        let (prefix, _) = split_database_url(admin)?;
        let mut options = crate::driver::ConnectOptions::new(format!("{prefix}/template1"));
        options.max_connections(1);
        let holder = crate::driver::Database::connect(options)
            .await
            .map_err(|error| error.to_string())?;
        let row = holder
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT current_database() AS datname".to_owned(),
            ))
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "template1 holder returned no row".to_owned())?;
        let current: String = row
            .try_get("", "datname")
            .map_err(|error| error.to_string())?;
        if current != "template1" {
            let _ = holder.close().await;
            return Err(format!(
                "expected to occupy template1, connected to {current}"
            ));
        }
        let database = tracked_create(admin, names).await?;
        let connection = open_one(database.url()).await?;
        probe(&connection, 5).await?;
        connection
            .close()
            .await
            .map_err(|error| error.to_string())?;
        let dropped = database.drop_database().await;
        let closed = holder.close().await.map_err(|error| error.to_string());
        dropped?;
        closed
    }

    async fn parallel_creates(admin: &str, names: &FixtureNames) -> Result<(), String> {
        let mut creates = Vec::new();
        for index in 0..8 {
            let admin = admin.to_owned();
            let names = names.clone();
            creates.push(tokio::spawn(async move {
                let database = tracked_create(&admin, &names).await?;
                let name = database.name().to_owned();
                let connection = open_one(database.url()).await?;
                probe(&connection, index).await?;
                connection
                    .close()
                    .await
                    .map_err(|error| error.to_string())?;
                database.drop_database().await?;
                Ok::<String, String>(name)
            }));
        }
        let mut completed_names = Vec::new();
        let mut failures = Vec::new();
        // Join every task before cleanup, including when an earlier one fails:
        // an unjoined task could still be creating a database after cleanup.
        for created in creates {
            match created.await {
                Ok(Ok(name)) => completed_names.push(name),
                Ok(Err(error)) => failures.push(error),
                Err(error) => failures.push(error.to_string()),
            }
        }
        if !failures.is_empty() {
            return Err(format!("parallel fixture tasks failed: {failures:?}"));
        }
        completed_names.sort();
        completed_names.dedup();
        if completed_names.len() != 8 {
            return Err(format!(
                "expected 8 distinct databases, got {completed_names:?}"
            ));
        }
        for name in &completed_names {
            if !is_disposable_name(name) {
                return Err(format!("fixture name {name} escaped the prefix check"));
            }
        }
        Ok(())
    }

    async fn isolated_writes(admin: &str, names: &FixtureNames) -> Result<(), String> {
        let left = tracked_create(admin, names).await?;
        let right = tracked_create(admin, names).await?;
        let left_db = open_one(left.url()).await?;
        let right_db = open_one(right.url()).await?;
        probe(&left_db, 11).await?;
        probe(&right_db, 22).await?;
        let left_value = scalar(&left_db, "SELECT id AS total FROM fexu_fixture_probe").await?;
        let right_value = scalar(&right_db, "SELECT id AS total FROM fexu_fixture_probe").await?;
        left_db.close().await.map_err(|error| error.to_string())?;
        right_db.close().await.map_err(|error| error.to_string())?;
        left.drop_database().await?;
        right.drop_database().await?;
        if left_value != 11 || right_value != 22 {
            return Err(format!(
                "isolated writes leaked or moved: left={left_value} right={right_value}"
            ));
        }
        Ok(())
    }

    async fn overlap_create_and_drop(admin: &str, names: &FixtureNames) -> Result<(), String> {
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(8));
        let mut tasks = Vec::new();
        for slot in 0..8 {
            let admin = admin.to_owned();
            let names = names.clone();
            let barrier = barrier.clone();
            tasks.push(tokio::spawn(async move {
                if slot < 4 {
                    let database = tracked_create(&admin, &names).await;
                    barrier.wait().await;
                    let database = database?;
                    database.drop_database().await?;
                } else {
                    barrier.wait().await;
                    let database = tracked_create(&admin, &names).await?;
                    let connection = open_one(database.url()).await?;
                    probe(&connection, i64::from(slot)).await?;
                    connection
                        .close()
                        .await
                        .map_err(|error| error.to_string())?;
                    database.drop_database().await?;
                }
                Ok::<(), String>(())
            }));
        }
        let mut failures = Vec::new();
        for task in tasks {
            match task.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => failures.push(error),
                Err(error) => failures.push(error.to_string()),
            }
        }
        if !failures.is_empty() {
            return Err(format!("overlap fixture tasks failed: {failures:?}"));
        }
        Ok(())
    }

    async fn abandoned_connection(admin: &str, names: &FixtureNames) -> Result<(), String> {
        let database = tracked_create(admin, names).await?;
        let name = database.name().to_owned();
        let url = database.url().to_owned();
        let connection = open_one(&url).await?;
        probe(&connection, 7).await?;
        database.drop_database().await?;
        let _ = connection.close().await;
        if open_one(&url).await.is_ok() {
            return Err(format!(
                "{name} still accepted a connection after an abandoned session was dropped"
            ));
        }
        let (prefix, admin_name) = split_database_url(admin)?;
        drop_owned_database(&prefix, &admin_name, &name).await
    }

    async fn second_drop(admin: &str, names: &FixtureNames) -> Result<(), String> {
        let database = tracked_create(admin, names).await?;
        let name = database.name().to_owned();
        let prefix = database.admin_prefix.clone();
        let session = database.admin_database.clone();
        database.drop_database().await?;
        drop_owned_database(&prefix, &session, &name).await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    #[ignore = "requires FEXU_POSTGRES_ADMIN or DATABASE_URL on 127.0.0.1, CREATEDB, and an approved admin database"]
    async fn postgres_fixture_parallel_create_isolate_and_cleanup() {
        let admin = loopback_admin_url().expect("loopback admin URL");
        with_owned_fixture_cleanup(&admin, |names| {
            let admin = admin.clone();
            async move {
                let version = server_version_num(&admin).await?;
                if version < 160_000 {
                    return Err(format!(
                        "PostgreSQL server_version_num {version} is older than 16; fixture tests target 16, 17, and 18"
                    ));
                }
                create_while_template1_is_open(&admin, &names).await?;
                parallel_creates(&admin, &names).await?;
                isolated_writes(&admin, &names).await?;
                overlap_create_and_drop(&admin, &names).await?;
                abandoned_connection(&admin, &names).await?;
                second_drop(&admin, &names).await?;
                Ok(())
            }
        })
        .await
        .expect("parallel fixtures and owned cleanup");
    }

    #[test]
    fn fixture_urls_do_not_depend_on_windows_paths() {
        let (prefix, name) =
            split_database_url("postgresql://fexu:secret@localhost:5432/postgres?sslmode=disable")
                .expect("url");
        assert_eq!(prefix, "postgresql://fexu:secret@localhost:5432");
        assert_eq!(name, "postgres");
        assert!(!prefix.contains('\\'));
        assert!(is_loopback_postgres_url(
            "postgres://ferrioxid@127.0.0.1:5432/postgres"
        ));
        assert!(!is_loopback_postgres_url(
            "postgres://ferrioxid@10.0.0.8:5432/postgres"
        ));
        assert!(!is_loopback_postgres_url(
            "postgres://ferrioxid@example.test:5432/postgres"
        ));
    }
}
