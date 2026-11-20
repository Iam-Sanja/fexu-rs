//! Populated-data checks for explicit renames, defaults, and indexes.
//!
//! PostgreSQL uses one disposable `fexu_test_<32 hex>` database and drops only
//! that database. It does not write to an application database.

use fexu_core::{secondary_index_name, unique_index_name};
use fexu_db::driver::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};
use fexu_migrations::{
    apply_directory, make_migration, migration_status, plan_database, revert_latest,
    write_explicit_migration, ColumnSnapshot, IndexSnapshot, LogicalType, MakeOutcome,
    MigrationOperation, SchemaSnapshot, TableSnapshot, UniqueConstraintSnapshot,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let tick = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("fexu-evolve-{nanos}-{tick}"));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn sqlite_url(path: &Path) -> String {
    format!(
        "sqlite://{}?mode=rwc",
        path.to_string_lossy().replace('\\', "/")
    )
}

fn column(
    name: &str,
    logical_type: LogicalType,
    primary_key: bool,
    nullable: bool,
    default: Option<bool>,
) -> ColumnSnapshot {
    ColumnSnapshot {
        name: name.to_owned(),
        logical_type,
        primary_key,
        nullable,
        default,
        precision: None,
        scale: None,
        choices: Vec::new(),
    }
}

fn products(extra: Vec<ColumnSnapshot>) -> SchemaSnapshot {
    let mut columns = vec![
        column("id", LogicalType::Uuid, true, false, None),
        column("name", LogicalType::String, false, false, None),
        column("active", LogicalType::Bool, false, false, Some(true)),
    ];
    columns.extend(extra);
    table("products", columns, Vec::new(), Vec::new())
}

fn table(
    name: &str,
    columns: Vec<ColumnSnapshot>,
    unique_constraints: Vec<UniqueConstraintSnapshot>,
    indexes: Vec<IndexSnapshot>,
) -> SchemaSnapshot {
    SchemaSnapshot::from_tables(vec![TableSnapshot {
        name: name.to_owned(),
        columns,
        unique_constraints,
        indexes,
        foreign_keys: Vec::new(),
    }])
}

async fn history_count(db: &DatabaseConnection) -> i64 {
    let row = db
        .query_one(Statement::from_string(
            db.get_database_backend(),
            "SELECT COUNT(*) AS total FROM _fexu_migrations",
        ))
        .await
        .unwrap()
        .unwrap();
    row.try_get::<i64>("", "total")
        .or_else(|_| row.try_get::<i32>("", "total").map(i64::from))
        .unwrap()
}

async fn cell(db: &DatabaseConnection, sql: &str) -> String {
    let row = db
        .query_one(Statement::from_string(db.get_database_backend(), sql))
        .await
        .unwrap()
        .unwrap();
    if let Ok(value) = row.try_get::<String>("", "value") {
        return value;
    }
    if let Ok(value) = row.try_get::<i64>("", "value") {
        return value.to_string();
    }
    if let Ok(value) = row.try_get::<i32>("", "value") {
        return value.to_string();
    }
    if let Ok(value) = row.try_get::<bool>("", "value") {
        return value.to_string();
    }
    panic!("value column was not readable");
}

fn written(directory: &Path, name: &str, snapshot: &SchemaSnapshot) -> PathBuf {
    match make_migration(directory, name, snapshot).unwrap() {
        MakeOutcome::Written { path, .. } => path,
        MakeOutcome::Unchanged => panic!("{name} produced no migration"),
    }
}

#[tokio::test]
async fn populated_sqlite_evolution_keeps_rows_and_rejects_unsafe_changes() {
    let root = temp_dir();
    let migrations = root.join("migrations");
    let initial = products(vec![column("sku", LogicalType::String, false, false, None)]);
    written(&migrations, "initial", &initial);
    let db = fexu_db::connect(&sqlite_url(&root.join("app.sqlite3")))
        .await
        .unwrap();
    apply_directory(&db, &migrations, false).await.unwrap();
    db.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        "INSERT INTO \"products\" (\"id\", \"name\", \"active\", \"sku\") VALUES ('11111111-1111-4111-8111-111111111111', 'Laptop', 0, 'sku-1'), ('22222222-2222-4222-8222-222222222222', 'Phone', 0, 'sku-1')",
    ))
    .await
    .unwrap();

    let rejects = root.join("rejects");
    written(&rejects, "initial", &initial);
    let mut guessed = initial.clone();
    guessed.tables[0].columns[1].name = "title".to_owned();
    let guess = make_migration(&rejects, "guess", &guessed).unwrap_err();
    assert!(guess.0.contains("not interpreted as a rename"));
    let mut ambiguous_schema = initial.clone();
    ambiguous_schema.tables[0].columns[1].name = "title".to_owned();
    ambiguous_schema.tables[0].columns[3].name = "code".to_owned();
    let ambiguous = write_explicit_migration(
        &rejects,
        "rename_title",
        &ambiguous_schema,
        MigrationOperation::RenameColumn {
            table: "products".to_owned(),
            from: "name".to_owned(),
            to: "title".to_owned(),
        },
    )
    .unwrap_err();
    assert!(ambiguous.0.contains("ambiguous"));
    let injected = write_explicit_migration(
        &rejects,
        "rename_title",
        &guessed,
        MigrationOperation::RenameColumn {
            table: "products".to_owned(),
            from: "name".to_owned(),
            to: "title\";drop".to_owned(),
        },
    )
    .unwrap_err();
    assert!(injected.0.contains("Invalid SQL identifier"));
    assert!(!injected.0.contains("drop"));
    let same = write_explicit_migration(
        &rejects,
        "rename_same",
        &initial,
        MigrationOperation::RenameColumn {
            table: "products".to_owned(),
            from: "name".to_owned(),
            to: "name".to_owned(),
        },
    )
    .unwrap_err();
    assert!(same.0.contains("two different names"));

    let mut default_off = initial.clone();
    default_off.tables[0].columns[2].default = Some(false);
    written(&migrations, "default_off", &default_off);
    let old_history = history_count(&db).await;
    let upgrade_plan = plan_database(&db, &migrations, Some(&default_off))
        .await
        .unwrap();
    assert!(!upgrade_plan.has_error(), "{upgrade_plan:?}");
    assert!(!upgrade_plan.has_drift(), "{upgrade_plan:?}");
    assert_eq!(upgrade_plan.pending, vec!["0002_default_off".to_owned()]);
    assert_eq!(history_count(&db).await, old_history);
    let pending = migration_status(&db, &migrations, None).await.unwrap();
    assert!(pending.conflicts.is_empty());
    assert_eq!(pending.lines.len(), 2);
    assert_eq!(pending.lines[0].state, "angewendet");
    assert_eq!(pending.lines[1].state, "ausstehend");
    assert_eq!(history_count(&db).await, old_history);
    apply_directory(&db, &migrations, false).await.unwrap();
    assert_eq!(
        cell(
            &db,
            "SELECT \"active\" AS value FROM \"products\" WHERE \"id\" = '11111111-1111-4111-8111-111111111111'",
        )
        .await,
        "0"
    );
    db.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        "INSERT INTO \"products\" (\"id\", \"name\", \"sku\") VALUES ('33333333-3333-4333-8333-333333333333', 'Tablet', 'sku-3')",
    ))
    .await
    .unwrap();
    assert_eq!(
        cell(
            &db,
            "SELECT \"active\" AS value FROM \"products\" WHERE \"id\" = '33333333-3333-4333-8333-333333333333'",
        )
        .await,
        "0"
    );
    let mut no_default = default_off.clone();
    no_default.tables[0].columns[2].default = None;
    written(&migrations, "drop_default", &no_default);
    apply_directory(&db, &migrations, false).await.unwrap();
    assert_eq!(
        cell(
            &db,
            "SELECT \"active\" AS value FROM \"products\" WHERE \"id\" = '33333333-3333-4333-8333-333333333333'",
        )
        .await,
        "0"
    );
    assert!(db
        .execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "INSERT INTO \"products\" (\"id\", \"name\", \"sku\") VALUES ('44444444-4444-4444-8444-444444444444', 'Watch', 'sku-4')",
        ))
        .await
        .is_err());

    let mut indexed = no_default.clone();
    indexed.tables[0].indexes.push(IndexSnapshot {
        name: secondary_index_name("products", &["name"]),
        columns: vec!["name".to_owned()],
    });
    written(&migrations, "add_name_index", &indexed);
    apply_directory(&db, &migrations, false).await.unwrap();
    written(&migrations, "drop_name_index", &no_default);
    apply_directory(&db, &migrations, false).await.unwrap();
    assert_eq!(
        cell(
            &db,
            "SELECT \"name\" AS value FROM \"products\" WHERE \"id\" = '22222222-2222-4222-8222-222222222222'",
        )
        .await,
        "Phone"
    );

    let history_before_unique = history_count(&db).await;
    let mut unique = no_default.clone();
    unique.tables[0]
        .unique_constraints
        .push(UniqueConstraintSnapshot {
            name: unique_index_name("products", &["sku"]),
            columns: vec!["sku".to_owned()],
        });
    let unique_path = written(&migrations, "unique_sku", &unique);
    let conflict = apply_directory(&db, &migrations, false).await.unwrap_err();
    assert!(conflict.0.contains("rolled back"));
    assert_eq!(history_count(&db).await, history_before_unique);
    assert_eq!(
        cell(
            &db,
            "SELECT COUNT(*) AS value FROM \"products\" WHERE \"sku\" = 'sku-1'",
        )
        .await,
        "2"
    );
    std::fs::remove_file(unique_path).unwrap();

    let mut renamed = no_default.clone();
    renamed.tables[0].columns[1].name = "title".to_owned();
    write_explicit_migration(
        &migrations,
        "rename_title",
        &renamed,
        MigrationOperation::RenameColumn {
            table: "products".to_owned(),
            from: "name".to_owned(),
            to: "title".to_owned(),
        },
    )
    .unwrap();
    plan_database(&db, &migrations, Some(&renamed))
        .await
        .unwrap();
    assert_eq!(
        cell(
            &db,
            "SELECT \"name\" AS value FROM \"products\" WHERE \"id\" = '11111111-1111-4111-8111-111111111111'",
        )
        .await,
        "Laptop"
    );
    apply_directory(&db, &migrations, false).await.unwrap();
    assert_eq!(
        cell(
            &db,
            "SELECT \"title\" AS value FROM \"products\" WHERE \"id\" = '11111111-1111-4111-8111-111111111111'",
        )
        .await,
        "Laptop"
    );
    assert_eq!(
        cell(&db, "SELECT COUNT(*) AS value FROM \"products\"").await,
        "3"
    );
    let dry = revert_latest(&db, &migrations, false, true).await.unwrap();
    assert!(!dry.executed);
    assert_eq!(
        cell(
            &db,
            "SELECT \"title\" AS value FROM \"products\" WHERE \"id\" = '11111111-1111-4111-8111-111111111111'",
        )
        .await,
        "Laptop"
    );
    assert!(
        revert_latest(&db, &migrations, false, false)
            .await
            .unwrap()
            .executed
    );
    assert_eq!(
        cell(
            &db,
            "SELECT \"name\" AS value FROM \"products\" WHERE \"id\" = '11111111-1111-4111-8111-111111111111'",
        )
        .await,
        "Laptop"
    );
    apply_directory(&db, &migrations, false).await.unwrap();
    assert_eq!(
        cell(
            &db,
            "SELECT \"title\" AS value FROM \"products\" WHERE \"id\" = '11111111-1111-4111-8111-111111111111'",
        )
        .await,
        "Laptop"
    );

    let mut noted = renamed.clone();
    noted.tables[0]
        .columns
        .push(column("note", LogicalType::String, false, true, None));
    written(&migrations, "add_note", &noted);
    apply_directory(&db, &migrations, false).await.unwrap();
    db.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        "UPDATE \"products\" SET \"note\" = 'keep' WHERE \"id\" = '11111111-1111-4111-8111-111111111111'",
    ))
    .await
    .unwrap();
    let blocked = revert_latest(&db, &migrations, false, false)
        .await
        .unwrap_err();
    assert!(blocked.0.contains("not executed"));
    assert!(blocked.0.contains("data loss"));
    assert_eq!(
        cell(
            &db,
            "SELECT \"note\" AS value FROM \"products\" WHERE \"id\" = '11111111-1111-4111-8111-111111111111'",
        )
        .await,
        "keep"
    );
    let history_before_drift = history_count(&db).await;
    db.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        "ALTER TABLE products ADD COLUMN unmanaged integer",
    ))
    .await
    .unwrap();
    let drift = plan_database(&db, &migrations, None).await.unwrap();
    assert!(drift.has_drift(), "{drift:?}");
    assert!(drift
        .findings
        .iter()
        .any(|finding| finding.message.contains("unmanaged")));
    assert_eq!(history_count(&db).await, history_before_drift);
    assert_eq!(
        cell(&db, "SELECT COUNT(*) AS value FROM \"products\"").await,
        "3"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL pointing at an approved PostgreSQL admin database"]
async fn postgres_schema_evolution_keeps_rows_on_a_disposable_database() {
    let admin = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let isolated = fexu_db::DisposablePostgres::create(&admin)
        .await
        .expect("isolated database");
    let root = temp_dir();
    let migrations = root.join("migrations");
    let result = async {
        let initial = products(vec![column(
            "sku",
            LogicalType::String,
            false,
            false,
            None,
        )]);
        written(&migrations, "initial", &initial);
        let db = fexu_db::connect(isolated.url())
            .await
            .map_err(|error| error.to_string())?;
        apply_directory(&db, &migrations, false)
            .await
            .map_err(|error| error.0)?;
        db.execute(Statement::from_string(
            DatabaseBackend::Postgres,
            "INSERT INTO \"products\" (\"id\", \"name\", \"active\", \"sku\") VALUES ('11111111-1111-4111-8111-111111111111', 'Laptop', FALSE, 'sku-1'), ('22222222-2222-4222-8222-222222222222', 'Phone', FALSE, 'sku-1')",
        ))
        .await
        .map_err(|error| error.to_string())?;
        let mut default_off = initial.clone();
        default_off.tables[0].columns[2].default = Some(false);
        written(&migrations, "default_off", &default_off);
        let old_history = history_count(&db).await;
        let upgrade_plan = plan_database(&db, &migrations, Some(&default_off))
            .await
            .map_err(|error| error.0)?;
        if upgrade_plan.has_error()
            || upgrade_plan.has_drift()
            || upgrade_plan.pending != ["0002_default_off".to_owned()]
            || history_count(&db).await != old_history
        {
            return Err(format!("older PostgreSQL schema plan changed data or misreported pending upgrade: {upgrade_plan:?}"));
        }
        let pending = migration_status(&db, &migrations, None)
            .await
            .map_err(|error| error.0)?;
        if !pending.conflicts.is_empty()
            || pending.lines.len() != 2
            || pending.lines[0].state != "angewendet"
            || pending.lines[1].state != "ausstehend"
            || history_count(&db).await != old_history
        {
            return Err("read-only status changed or misreported the older schema".to_owned());
        }
        apply_directory(&db, &migrations, false)
            .await
            .map_err(|error| error.0)?;
        db.execute(Statement::from_string(
            DatabaseBackend::Postgres,
            "INSERT INTO \"products\" (\"id\", \"name\", \"sku\") VALUES ('33333333-3333-4333-8333-333333333333', 'Tablet', 'sku-3')",
        ))
        .await
        .map_err(|error| error.to_string())?;
        let created = cell(
            &db,
            "SELECT \"active\" AS value FROM \"products\" WHERE \"id\" = '33333333-3333-4333-8333-333333333333'",
        )
        .await;
        if created != "false" {
            return Err(format!("new row did not use the new default: {created}"));
        }
        let old = cell(
            &db,
            "SELECT \"active\" AS value FROM \"products\" WHERE \"id\" = '11111111-1111-4111-8111-111111111111'",
        )
        .await;
        if old != "false" {
            return Err(format!("existing row was rewritten: {old}"));
        }
        let mut renamed = default_off.clone();
        renamed.tables[0].columns[1].name = "title".to_owned();
        write_explicit_migration(
            &migrations,
            "rename_title",
            &renamed,
            MigrationOperation::RenameColumn {
                table: "products".to_owned(),
                from: "name".to_owned(),
                to: "title".to_owned(),
            },
        )
        .map_err(|error| error.0)?;
        apply_directory(&db, &migrations, false)
            .await
            .map_err(|error| error.0)?;
        let title = cell(
            &db,
            "SELECT \"title\" AS value FROM \"products\" WHERE \"id\" = '11111111-1111-4111-8111-111111111111'",
        )
        .await;
        if title != "Laptop" {
            return Err(format!("rename lost the row value: {title}"));
        }
        let history_with_rename = history_count(&db).await;
        let preview = revert_latest(&db, &migrations, false, true)
            .await
            .map_err(|error| error.0)?;
        if preview.executed || preview.destructive || history_count(&db).await != history_with_rename {
            return Err("rename rollback preview changed PostgreSQL history".to_owned());
        }
        let reverted = revert_latest(&db, &migrations, false, false)
            .await
            .map_err(|error| error.0)?;
        if !reverted.executed || history_count(&db).await != history_with_rename - 1 {
            return Err("rename rollback did not remove exactly one history entry".to_owned());
        }
        let restored = cell(
            &db,
            "SELECT \"name\" AS value FROM \"products\" WHERE \"id\" = '11111111-1111-4111-8111-111111111111'",
        )
        .await;
        if restored != "Laptop" {
            return Err(format!("rename rollback lost data: {restored}"));
        }
        apply_directory(&db, &migrations, false)
            .await
            .map_err(|error| error.0)?;
        if history_count(&db).await != history_with_rename {
            return Err("reapplying rename did not restore migration history".to_owned());
        }
        let reapplied = cell(
            &db,
            "SELECT \"title\" AS value FROM \"products\" WHERE \"id\" = '11111111-1111-4111-8111-111111111111'",
        )
        .await;
        if reapplied != "Laptop" {
            return Err(format!("reapplying rename lost data: {reapplied}"));
        }
        let history_before = history_count(&db).await;
        let mut unique = renamed.clone();
        unique.tables[0].unique_constraints.push(UniqueConstraintSnapshot {
            name: unique_index_name("products", &["sku"]),
            columns: vec!["sku".to_owned()],
        });
        let unique_path = written(&migrations, "unique_sku", &unique);
        if apply_directory(&db, &migrations, false).await.is_ok() {
            return Err("duplicate sku was accepted".to_owned());
        }
        if history_count(&db).await != history_before {
            return Err("failed unique migration wrote history".to_owned());
        }
        let duplicates = cell(
            &db,
            "SELECT COUNT(*) AS value FROM \"products\" WHERE \"sku\" = 'sku-1'",
        )
        .await;
        if duplicates != "2" {
            return Err(format!("duplicate rows changed: {duplicates}"));
        }
        // Remove only the disposable test's failed, unapplied migration file.
        std::fs::remove_file(unique_path).map_err(|error| error.to_string())?;
        let mut indexed = renamed.clone();
        indexed.tables[0].indexes.push(IndexSnapshot {
            name: secondary_index_name("products", &["sku"]),
            columns: vec!["sku".to_owned()],
        });
        written(&migrations, "index_sku", &indexed);
        let applied = apply_directory(&db, &migrations, false)
            .await
            .map_err(|error| error.0)?;
        if applied.applied.len() != 1 || history_count(&db).await != history_before + 1 {
            return Err("recovery did not apply exactly one valid migration".to_owned());
        }
        if !apply_directory(&db, &migrations, false)
            .await
            .map_err(|error| error.0)?
            .applied
            .is_empty()
        {
            return Err("recovered migration was applied twice".to_owned());
        }
        let recovered_rows = cell(&db, "SELECT COUNT(*) AS value FROM \"products\"").await;
        if recovered_rows != "3" {
            return Err(format!("recovery lost rows: {recovered_rows}"));
        }
        let mut with_note = indexed.clone();
        with_note.tables[0]
            .columns
            .push(column("note", LogicalType::String, false, true, None));
        written(&migrations, "add_note", &with_note);
        apply_directory(&db, &migrations, false)
            .await
            .map_err(|error| error.0)?;
        db.execute(Statement::from_string(
            DatabaseBackend::Postgres,
            "UPDATE \"products\" SET \"note\" = 'keep' WHERE \"id\" = '11111111-1111-4111-8111-111111111111'",
        ))
        .await
        .map_err(|error| error.to_string())?;
        let before_refusal = history_count(&db).await;
        let refusal = revert_latest(&db, &migrations, false, false)
            .await
            .expect_err("dropping a populated column must require approval");
        if !refusal.0.contains("not executed")
            || !refusal.0.contains("data loss")
            || history_count(&db).await != before_refusal
        {
            return Err("unsafe revert did not fail closed without changing history".to_owned());
        }
        let preserved = cell(
            &db,
            "SELECT \"note\" AS value FROM \"products\" WHERE \"id\" = '11111111-1111-4111-8111-111111111111'",
        )
        .await;
        if preserved != "keep" {
            return Err(format!("rejected revert changed data: {preserved}"));
        }
        let preview = revert_latest(&db, &migrations, false, true)
            .await
            .map_err(|error| error.0)?;
        if preview.executed || !preview.destructive || history_count(&db).await != before_refusal {
            return Err("destructive preview changed history or missed data loss".to_owned());
        }
        let approved = revert_latest(&db, &migrations, true, false)
            .await
            .map_err(|error| error.0)?;
        if !approved.executed || !approved.destructive {
            return Err("approved destructive revert did not execute".to_owned());
        }
        if history_count(&db).await != before_refusal - 1 {
            return Err("approved destructive revert did not update history".to_owned());
        }
        let remaining = cell(&db, "SELECT COUNT(*) AS value FROM \"products\"").await;
        if remaining != "3" {
            return Err(format!("approved revert removed unrelated rows: {remaining}"));
        }
        let title = cell(
            &db,
            "SELECT \"title\" AS value FROM \"products\" WHERE \"id\" = '11111111-1111-4111-8111-111111111111'",
        )
        .await;
        if title != "Laptop" {
            return Err(format!("approved revert changed preserved data: {title}"));
        }
        if db
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT \"note\" FROM \"products\"",
            ))
            .await
            .is_ok()
        {
            return Err("approved destructive revert did not drop note column".to_owned());
        }
        let history_before_drift = history_count(&db).await;
        db.execute(Statement::from_string(
            DatabaseBackend::Postgres,
            "ALTER TABLE products ADD COLUMN unmanaged integer",
        ))
        .await
        .map_err(|error| error.to_string())?;
        let drift = plan_database(&db, &migrations, None)
            .await
            .map_err(|error| error.0)?;
        if !drift.has_drift()
            || !drift.findings.iter().any(|finding| finding.message.contains("unmanaged"))
            || history_count(&db).await != history_before_drift
        {
            return Err(format!("PostgreSQL live drift was not read-only or was missed: {drift:?}"));
        }
        if cell(&db, "SELECT COUNT(*) AS value FROM \"products\"").await != "3" {
            return Err("read-only drift plan changed existing PostgreSQL rows".to_owned());
        }
        Ok(())
    }
    .await;
    let dropped = isolated.drop_database().await;
    result.expect("postgres schema evolution");
    dropped.expect("drop isolated database");
    let _ = std::fs::remove_dir_all(&root);
}
