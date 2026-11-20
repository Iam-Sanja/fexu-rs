//! Read-only comparison of migration files, history, and the live schema.
//! Nothing in this module creates, alters, or drops a database object.

use crate::error::MigrationError;
use crate::history::{self, HistoryEntry};
use crate::introspection::{compare_snapshot, introspect_live};
use crate::runner::pending_files;
use crate::snapshot::SchemaSnapshot;
use crate::store::{inspect_directory, MigrationFile};
use fexu_db::driver::{ConnectionTrait, DatabaseConnection};
use fexu_db::kind_of;
use std::fmt::{Display, Formatter};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanLevel {
    Error,
    Drift,
    Warning,
}

impl Display for PlanLevel {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Error => "error",
            Self::Drift => "drift",
            Self::Warning => "warning",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanFinding {
    pub level: PlanLevel,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanReport {
    pub backend: String,
    pub pending: Vec<String>,
    pub findings: Vec<PlanFinding>,
}

impl PlanReport {
    pub fn has_error(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.level == PlanLevel::Error)
    }

    pub fn has_drift(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.level == PlanLevel::Drift)
    }
}

pub async fn plan_database(
    db: &DatabaseConnection,
    directory: &Path,
    current: Option<&SchemaSnapshot>,
) -> Result<PlanReport, MigrationError> {
    plan_database_in(db, directory, history::HISTORY_TABLE, current).await
}

/// Read-only migration plan for one project with root and namespaced apps.
///
/// Every app shares the root history table but has its own on-disk migration
/// chain and snapshots. Validates each chain independently, reports unowned
/// history IDs rather than treating them as root migrations, and compares only
/// the live tables owned by the latest applied snapshot of each component.
/// Nothing is created or migrated.
pub async fn plan_project(
    db: &DatabaseConnection,
    root: &Path,
    current: Option<&SchemaSnapshot>,
    packages: &[crate::packages::PackageMigration],
) -> Result<PlanReport, MigrationError> {
    let mut report =
        plan_database_internal(db, root, history::HISTORY_TABLE, current, true).await?;
    if let Err(error) = crate::packages::validate_package_graph(packages) {
        report.findings.push(finding(PlanLevel::Error, error.0));
        return Ok(report);
    }
    let order = match crate::packages::package_order(packages) {
        Ok(order) => order,
        Err(error) => {
            report.findings.push(finding(PlanLevel::Error, error.0));
            return Ok(report);
        }
    };
    // Keep the known app namespace set separate from the root history.
    // Orphaned namespaced rows must not silently disappear from the plan.
    let existing = history::read_history_in(db, history::HISTORY_TABLE).await?;
    for entry in &existing {
        if let Some((namespace, local_id)) = entry.id.split_once(':') {
            if local_id.is_empty()
                || local_id.contains(':')
                || !packages.iter().any(|package| package.name == namespace)
            {
                report.findings.push(finding(
                    PlanLevel::Error,
                    format!(
                        "Applied namespaced migration {} has no declared app or has an invalid namespace.",
                        entry.id
                    ),
                ));
            }
        }
    }
    for index in order {
        let package = &packages[index];
        if !package.directory.is_dir() {
            report.findings.push(finding(
                PlanLevel::Error,
                format!(
                    "The migrations directory for app `{}` is missing.",
                    package.name
                ),
            ));
            continue;
        }
        let state = inspect_directory(&package.directory);
        for conflict in &state.conflicts {
            report.findings.push(finding(
                PlanLevel::Error,
                format!("App {}: {conflict}", package.name),
            ));
        }
        let status =
            crate::runner::namespaced_status(db, &package.directory, &package.name).await?;
        for conflict in &status.conflicts {
            report.findings.push(finding(
                PlanLevel::Error,
                format!("App {}: {conflict}", package.name),
            ));
        }
        report.pending.extend(
            status
                .lines
                .iter()
                .filter(|line| line.state == "ausstehend")
                .map(|line| line.id.clone()),
        );
        if !status.conflicts.is_empty()
            || report.findings.iter().any(|item| {
                item.level == PlanLevel::Error
                    && item.message.starts_with(&format!("App {}:", package.name))
            })
        {
            continue;
        }
        let last_applied = state
            .files
            .iter()
            .filter(|file| {
                let history_id = format!("{}:{}", package.name, file.id);
                status
                    .lines
                    .iter()
                    .any(|line| line.id == history_id && line.state == "angewendet")
            })
            .max_by_key(|file| crate::store::migration_number(&file.id).unwrap_or(0));
        if let Some(file) = last_applied {
            let live = introspect_live(db).await?;
            let owned = live_tables_owned_by(&file.to_snapshot, &live);
            for difference in compare_snapshot(&file.to_snapshot, &owned) {
                report.findings.push(finding(
                    PlanLevel::Drift,
                    format!("App {}: {}", package.name, drift_text(&difference)),
                ));
            }
        }
    }
    Ok(report)
}

/// Read-only status and live drift for an embedded framework package.
///
/// History tables and schema objects are not created or altered. A checksum or
/// order conflict is an error and suppresses the live comparison. The live
/// comparison covers only tables named by the latest applied package snapshot.
/// Application tables and the other framework package stay out of this drift
/// report. A missing package table, or an extra or changed column on a package
/// table, remains drift.
pub async fn plan_embedded(
    db: &DatabaseConnection,
    files: &[MigrationFile],
    history_table: &str,
    label: &str,
) -> Result<PlanReport, MigrationError> {
    let backend = kind_of(db.get_database_backend())
        .map_err(|error| MigrationError::new(error.to_string()))?;
    if files.is_empty() {
        return Ok(PlanReport {
            backend: history::backend_name(backend).to_owned(),
            pending: Vec::new(),
            findings: vec![finding(
                PlanLevel::Error,
                format!(
                    "Required framework migration resources for `{label}` are missing. Nothing was applied."
                ),
            )],
        });
    }
    let status = crate::status_of_files(db, files, history_table, None).await?;
    let mut findings = status
        .conflicts
        .iter()
        .map(|message| finding(PlanLevel::Error, message))
        .collect::<Vec<_>>();
    let pending = status
        .lines
        .iter()
        .filter(|line| line.state == "ausstehend")
        .map(|line| format!("{label}:{}", line.id))
        .collect::<Vec<_>>();
    let trusted = !findings.iter().any(|item| item.level == PlanLevel::Error);
    if trusted {
        if let Some(file) = latest_applied(files, &status) {
            let live = crate::introspection::introspect_framework_live(db).await?;
            let owned = live_tables_owned_by(&file.to_snapshot, &live);
            for difference in crate::introspection::compare_snapshot(&file.to_snapshot, &owned) {
                findings.push(finding(
                    PlanLevel::Drift,
                    format!("{label}: {}", drift_text(&difference)),
                ));
            }
        } else {
            findings.push(finding(
                PlanLevel::Warning,
                format!(
                    "No {label} migration has been applied. The live schema was not compared with a pending target."
                ),
            ));
        }
    }
    Ok(PlanReport {
        backend: history::backend_name(backend).to_owned(),
        pending,
        findings,
    })
}

/// App dependency errors for a read-only plan. This does not read or write a database.
pub fn package_dependency_findings(
    packages: &[crate::packages::PackageMigration],
) -> Vec<PlanFinding> {
    match crate::packages::package_order(packages) {
        Ok(_) => Vec::new(),
        Err(error) => vec![finding(PlanLevel::Error, error.0)],
    }
}

fn live_tables_owned_by(
    snapshot: &SchemaSnapshot,
    live: &[crate::introspection::LiveTable],
) -> Vec<crate::introspection::LiveTable> {
    live.iter()
        .filter(|table| {
            snapshot
                .tables
                .iter()
                .any(|expected| expected.name == table.name)
        })
        .cloned()
        .collect()
}

fn latest_applied<'a>(
    files: &'a [MigrationFile],
    status: &crate::StatusReport,
) -> Option<&'a MigrationFile> {
    let applied = status
        .lines
        .iter()
        .filter(|line| line.state == "angewendet")
        .map(|line| line.id.as_str())
        .collect::<Vec<_>>();
    files
        .iter()
        .filter(|file| applied.iter().any(|id| *id == file.id))
        .max_by_key(|file| crate::store::migration_number(&file.id).unwrap_or(0))
}

pub async fn plan_database_in(
    db: &DatabaseConnection,
    directory: &Path,
    history_table: &str,
    current: Option<&SchemaSnapshot>,
) -> Result<PlanReport, MigrationError> {
    plan_database_internal(db, directory, history_table, current, false).await
}

async fn plan_database_internal(
    db: &DatabaseConnection,
    directory: &Path,
    history_table: &str,
    current: Option<&SchemaSnapshot>,
    scoped_live: bool,
) -> Result<PlanReport, MigrationError> {
    let state = inspect_directory(directory);
    let backend = kind_of(db.get_database_backend())
        .map_err(|error| MigrationError::new(error.to_string()))?;
    let mut findings = state
        .conflicts
        .iter()
        .map(|message| finding(PlanLevel::Error, message))
        .collect::<Vec<_>>();
    let history_present = history::history_exists_in(db, history_table).await?;
    let applied = if history_present {
        history::read_history_in(db, history_table)
            .await?
            .into_iter()
            // Root migrations and namespaced app migrations share the same
            // history table. Each namespace is validated independently below.
            .filter(|entry| history_table != history::HISTORY_TABLE || !entry.id.contains(':'))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    for entry in &applied {
        if !state.files.iter().any(|file| file.id == entry.id) {
            findings.push(finding(
                PlanLevel::Error,
                format!(
                    "Applied migration {} no longer has a readable file in the migrations directory.",
                    entry.id
                ),
            ));
        }
    }
    let pending = match pending_files(&state.files, &applied) {
        Ok(files) => files.iter().map(|file| file.id.clone()).collect::<Vec<_>>(),
        Err(error) => {
            findings.push(finding(PlanLevel::Error, error.0));
            state
                .files
                .iter()
                .filter(|file| !applied.iter().any(|entry| entry.id == file.id))
                .map(|file| file.id.clone())
                .collect()
        }
    };
    let trusted = !findings.iter().any(|item| item.level == PlanLevel::Error);
    if trusted {
        if let Some(file) = last_applied(&state.files, &applied) {
            let live = introspect_live(db).await?;
            let owned = if scoped_live {
                live_tables_owned_by(&file.to_snapshot, &live)
            } else {
                live
            };
            for difference in compare_snapshot(&file.to_snapshot, &owned) {
                findings.push(finding(PlanLevel::Drift, drift_text(&difference)));
            }
        } else if !state.files.is_empty() {
            findings.push(finding(
                PlanLevel::Warning,
                "No migration has been applied. The live schema was not compared with a pending target.",
            ));
        }
        if let Some(current) = current {
            note_model(
                &state.files,
                &pending,
                last_applied(&state.files, &applied),
                current,
                &mut findings,
            );
        }
    }
    Ok(PlanReport {
        backend: history::backend_name(backend).to_owned(),
        pending,
        findings,
    })
}

fn last_applied<'a>(
    files: &'a [MigrationFile],
    applied: &[HistoryEntry],
) -> Option<&'a MigrationFile> {
    applied
        .last()
        .and_then(|entry| files.iter().find(|file| file.id == entry.id))
}

fn note_model(
    files: &[MigrationFile],
    pending: &[String],
    applied: Option<&MigrationFile>,
    current: &SchemaSnapshot,
    findings: &mut Vec<PlanFinding>,
) {
    if let Some(id) = pending.last() {
        if let Some(file) = files.iter().find(|file| &file.id == id) {
            if !file.to_snapshot.semantically_eq(current) {
                findings.push(finding(
                    PlanLevel::Warning,
                    "The current model does not match the latest pending migration. That difference is not reported as live drift.",
                ));
            }
        }
        return;
    }
    if let Some(file) = applied {
        if !file.to_snapshot.semantically_eq(current) {
            findings.push(finding(
                PlanLevel::Warning,
                "The current model differs from the last applied migration. Create a migration before changing the database.",
            ));
        }
        return;
    }
    if !current.semantically_eq(&SchemaSnapshot::empty()) {
        findings.push(finding(
            PlanLevel::Warning,
            "Models exist but no migration has been created yet.",
        ));
    }
}

fn drift_text(message: &str) -> String {
    if let Some(name) = message
        .strip_prefix("Unbekannte Tabelle `")
        .and_then(|rest| rest.strip_suffix("` blockiert die Baseline."))
    {
        return format!("Table `{name}` is not part of the last applied migration.");
    }
    message.replace(
        "prevents baselining.",
        "is not part of the last applied migration.",
    )
}

fn finding(level: PlanLevel, message: impl Into<String>) -> PlanFinding {
    PlanFinding {
        level,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{ColumnSnapshot, LogicalType, TableSnapshot};
    use crate::store::make_migration;
    use fexu_db::driver::{ConnectionTrait, DatabaseBackend, Statement};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn products() -> SchemaSnapshot {
        SchemaSnapshot::from_tables(vec![TableSnapshot {
            name: "products".to_owned(),
            columns: vec![ColumnSnapshot {
                name: "id".to_owned(),
                logical_type: LogicalType::I64,
                primary_key: true,
                nullable: false,
                default: None,
                precision: None,
                scale: None,
                choices: Vec::new(),
            }],
            unique_constraints: Vec::new(),
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
        }])
    }

    fn probe_table() -> SchemaSnapshot {
        SchemaSnapshot::from_tables(vec![TableSnapshot {
            name: "plan_probe".to_owned(),
            columns: vec![ColumnSnapshot {
                name: "id".to_owned(),
                logical_type: LogicalType::I64,
                primary_key: true,
                nullable: false,
                default: None,
                precision: None,
                scale: None,
                choices: Vec::new(),
            }],
            unique_constraints: Vec::new(),
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
        }])
    }

    fn scratch(label: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("fexu-plan-{label}-{nanos}"));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn sqlite_url(file: &std::path::Path) -> String {
        format!(
            "sqlite://{}?mode=rwc",
            file.to_string_lossy().replace('\\', "/")
        )
    }

    async fn names(db: &DatabaseConnection) -> Vec<String> {
        let rows = db
            .query_all(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name",
            ))
            .await
            .unwrap();
        rows.into_iter()
            .map(|row| row.try_get::<String>("", "name").unwrap())
            .collect()
    }

    #[tokio::test]
    async fn project_plan_handles_applied_namespaced_apps_and_is_read_only() {
        let root = scratch("namespaced");
        let global = root.join("migrations");
        let catalog = root.join("catalog");
        let reports = root.join("reports");
        std::fs::create_dir_all(&global).unwrap();
        make_migration(&catalog, "initial", &products()).unwrap();
        make_migration(&reports, "initial", &probe_table()).unwrap();
        let packages = vec![
            crate::packages::PackageMigration {
                name: "catalog".into(),
                directory: catalog,
                depends_on: vec![],
            },
            crate::packages::PackageMigration {
                name: "reports".into(),
                directory: reports,
                depends_on: vec!["catalog".into()],
            },
        ];
        let file = root.join("db.sqlite");
        let url = sqlite_url(&file);
        let db = fexu_db::connect(&url).await.unwrap();
        crate::apply_packages(&db, &packages, false).await.unwrap();
        drop(db);

        let before = std::fs::read(&file).unwrap();
        let readonly = crate::readonly_database_url(&url).unwrap();
        let db = fexu_db::connect(&readonly).await.unwrap();
        let plan = plan_project(&db, &global, None, &packages).await.unwrap();
        drop(db);
        assert!(!plan.has_error(), "{plan:?}");
        assert!(!plan.has_drift(), "{plan:?}");
        assert!(plan.pending.is_empty(), "{plan:?}");
        assert_eq!(std::fs::read(&file).unwrap(), before);

        let db = fexu_db::connect(&url).await.unwrap();
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "ALTER TABLE products ADD COLUMN unexpected integer",
        ))
        .await
        .unwrap();
        drop(db);
        let changed = std::fs::read(&file).unwrap();
        let db = fexu_db::connect(&readonly).await.unwrap();
        let drift = plan_project(&db, &global, None, &packages).await.unwrap();
        drop(db);
        assert!(drift.has_drift(), "{drift:?}");
        assert!(drift.findings.iter().any(|item| {
            item.message.contains("App catalog:") && item.message.contains("unexpected")
        }));
        assert!(!drift
            .findings
            .iter()
            .any(|item| { item.message.contains("no longer has a readable file") }));
        assert_eq!(std::fs::read(&file).unwrap(), changed);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn project_plan_reports_orphaned_namespaced_history_without_writing() {
        let root = scratch("orphan-namespace");
        let global = root.join("migrations");
        std::fs::create_dir_all(&global).unwrap();
        let file = root.join("db.sqlite");
        let url = sqlite_url(&file);
        let db = fexu_db::connect(&url).await.unwrap();
        history::ensure_history(&db).await.unwrap();
        history::insert_history(&db, "retired:0001_initial", "initial", "a", 1)
            .await
            .unwrap();
        drop(db);
        let before = std::fs::read(&file).unwrap();
        let db = fexu_db::connect(&crate::readonly_database_url(&url).unwrap())
            .await
            .unwrap();
        let plan = plan_project(&db, &global, None, &[]).await.unwrap();
        drop(db);
        assert!(plan.has_error(), "{plan:?}");
        assert!(plan.findings.iter().any(|item| {
            item.message.contains("retired:0001_initial")
                && item.message.contains("no declared app")
        }));
        assert_eq!(std::fs::read(&file).unwrap(), before);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn embedded_plan_can_see_its_own_internal_tables() {
        let root = scratch("embedded-internal");
        let migrations = root.join("migrations");
        let mut internal = probe_table();
        internal.tables[0].name = "_fexu_internal_probe".into();
        make_migration(&migrations, "initial", &internal).unwrap();
        let state = inspect_directory(&migrations);
        assert!(state.conflicts.is_empty());
        let db = fexu_db::connect("sqlite::memory:").await.unwrap();
        crate::apply_files_with_history(&db, &state.files, false, "_fexu_internal_probe_history")
            .await
            .unwrap();
        let plan = plan_embedded(&db, &state.files, "_fexu_internal_probe_history", "Auth")
            .await
            .unwrap();
        assert!(!plan.has_drift(), "{plan:?}");
        assert!(!plan.has_error(), "{plan:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn pending_plan_does_not_create_tables() {
        let root = scratch("pending");
        let app = root.join("migrations");
        std::fs::create_dir_all(&app).unwrap();
        make_migration(&app, "initial", &products()).unwrap();
        let db = fexu_db::connect("sqlite::memory:").await.unwrap();
        let before = names(&db).await;
        let report = plan_database(&db, &app, Some(&SchemaSnapshot::empty()))
            .await
            .unwrap();
        assert!(!report.has_error());
        assert!(!report.has_drift());
        assert_eq!(report.pending.len(), 1);
        assert!(report
            .findings
            .iter()
            .any(|item| item.level == PlanLevel::Warning));
        assert_eq!(names(&db).await, before);
        assert!(!history::table_exists(&db, "products").await.unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn applied_plan_is_quiet_and_drift_is_read_only() {
        let root = scratch("drift");
        let app = root.join("migrations");
        std::fs::create_dir_all(&app).unwrap();
        make_migration(&app, "initial", &products()).unwrap();
        let file = root.join("db.sqlite");
        let db = fexu_db::connect(&sqlite_url(&file)).await.unwrap();
        crate::apply_directory(&db, &app, false).await.unwrap();
        drop(db);
        let read_url = crate::readonly_database_url(&sqlite_url(&file)).unwrap();
        let before = std::fs::read(&file).unwrap();
        let db = fexu_db::connect(&read_url).await.unwrap();
        let clean = plan_database(&db, &app, Some(&products())).await.unwrap();
        drop(db);
        assert!(!clean.has_error(), "{clean:?}");
        assert!(!clean.has_drift(), "{clean:?}");
        assert!(clean.pending.is_empty());
        assert!(clean.findings.is_empty(), "{clean:?}");
        assert_eq!(std::fs::read(&file).unwrap(), before);
        let db = fexu_db::connect(&sqlite_url(&file)).await.unwrap();
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "ALTER TABLE products ADD COLUMN extra integer",
        ))
        .await
        .unwrap();
        drop(db);
        let changed = std::fs::read(&file).unwrap();
        let db = fexu_db::connect(&read_url).await.unwrap();
        let drifted = plan_database(&db, &app, Some(&products())).await.unwrap();
        drop(db);
        assert!(drifted.has_drift(), "{drifted:?}");
        assert!(drifted
            .findings
            .iter()
            .any(|item| item.message.contains("extra")));
        assert_eq!(std::fs::read(&file).unwrap(), changed);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn checksum_conflict_is_an_error_without_a_false_drift_report() {
        let root = scratch("checksum");
        let app = root.join("migrations");
        std::fs::create_dir_all(&app).unwrap();
        make_migration(&app, "initial", &products()).unwrap();
        let file = root.join("db.sqlite");
        let db = fexu_db::connect(&sqlite_url(&file)).await.unwrap();
        crate::apply_directory(&db, &app, false).await.unwrap();
        drop(db);
        let migration = std::fs::read_dir(&app)
            .unwrap()
            .find_map(|entry| entry.ok())
            .unwrap()
            .path();
        let mut parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&migration).unwrap()).unwrap();
        parsed["checksum"] = serde_json::Value::String("00".repeat(32));
        std::fs::write(&migration, serde_json::to_string_pretty(&parsed).unwrap()).unwrap();
        let before = std::fs::read(&file).unwrap();
        let db = fexu_db::connect(&crate::readonly_database_url(&sqlite_url(&file)).unwrap())
            .await
            .unwrap();
        let report = plan_database(&db, &app, Some(&products())).await.unwrap();
        drop(db);
        assert!(report.has_error(), "{report:?}");
        assert!(!report.has_drift(), "{report:?}");
        assert!(report
            .findings
            .iter()
            .any(|item| item.message.to_ascii_lowercase().contains("checksum")));
        assert_eq!(std::fs::read(&file).unwrap(), before);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL pointing at an approved PostgreSQL admin database"]
    async fn postgres_project_plan_checks_namespaced_apps_and_orphans_read_only() {
        let admin = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let isolated = fexu_db::DisposablePostgres::create(&admin)
            .await
            .expect("isolated postgres database");
        let root = scratch("postgres-project");
        let global = root.join("migrations");
        let catalog = root.join("catalog");
        let reports = root.join("reports");
        std::fs::create_dir_all(&global).unwrap();
        make_migration(&catalog, "initial", &products()).unwrap();
        make_migration(&reports, "initial", &probe_table()).unwrap();
        let packages = vec![
            crate::packages::PackageMigration {
                name: "catalog".into(),
                directory: catalog,
                depends_on: vec![],
            },
            crate::packages::PackageMigration {
                name: "reports".into(),
                directory: reports,
                depends_on: vec!["catalog".into()],
            },
        ];
        let outcome = async {
            let db = fexu_db::connect(isolated.url())
                .await
                .map_err(|error| error.to_string())?;
            crate::apply_packages(&db, &packages, false)
                .await
                .map_err(|error| error.0)?;
            let history_before = history::read_history_in(&db, history::HISTORY_TABLE)
                .await
                .map_err(|error| error.0)?;
            if history_before.len() != 2 {
                return Err(format!("expected 2 applied app migrations: {history_before:?}"));
            }
            let clean = plan_project(&db, &global, None, &packages)
                .await
                .map_err(|error| error.0)?;
            if clean.has_error() || clean.has_drift() || !clean.pending.is_empty() {
                return Err(format!("clean PostgreSQL app plan reported a failure: {clean:?}"));
            }
            if history::read_history_in(&db, history::HISTORY_TABLE)
                .await
                .map_err(|error| error.0)?
                != history_before
            {
                return Err("read-only plan modified PostgreSQL migration history".to_owned());
            }

            db.execute(Statement::from_string(
                DatabaseBackend::Postgres,
                "ALTER TABLE products ADD COLUMN unexpected integer",
            ))
            .await
            .map_err(|error| error.to_string())?;
            let drift = plan_project(&db, &global, None, &packages)
                .await
                .map_err(|error| error.0)?;
            if !drift.has_drift()
                || !drift.findings.iter().any(|item| {
                    item.message.contains("App catalog:")
                        && item.message.contains("unexpected")
                })
                || drift.findings.iter().any(|item| {
                    item.message.contains("no longer has a readable file")
                })
            {
                return Err(format!("PostgreSQL namespaced drift was misclassified: {drift:?}"));
            }

            history::insert_history(&db, "retired:0001_old", "old", "test", 1)
                .await
                .map_err(|error| error.0)?;
            let orphan = plan_project(&db, &global, None, &packages)
                .await
                .map_err(|error| error.0)?;
            if !orphan.has_error()
                || !orphan.findings.iter().any(|item| {
                    item.message.contains("retired:0001_old")
                        && item.message.contains("no declared app")
                })
            {
                return Err(format!("orphaned PostgreSQL app history was ignored: {orphan:?}"));
            }
            if history::read_history_in(&db, history::HISTORY_TABLE)
                .await
                .map_err(|error| error.0)?
                .len()
                != 3
            {
                return Err("PostgreSQL plan wrote unexpected migration history".to_owned());
            }
            db.close().await.map_err(|error| error.to_string())?;
            Ok::<(), String>(())
        }
        .await;
        let cleanup = isolated.drop_database().await;
        let _ = std::fs::remove_dir_all(&root);
        assert!(cleanup.is_ok(), "PostgreSQL fixture cleanup failed: {cleanup:?}");
        assert!(outcome.is_ok(), "PostgreSQL app plan regression: {outcome:?}");
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL pointing at an approved PostgreSQL admin database"]
    async fn postgres_plan_detects_live_drift_on_a_disposable_database() {
        let admin = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let isolated = fexu_db::DisposablePostgres::create(&admin)
            .await
            .expect("isolated database");
        let root = scratch("postgres");
        let app = root.join("migrations");
        std::fs::create_dir_all(&app).unwrap();
        make_migration(&app, "initial", &products()).unwrap();
        let result = async {
            let db = fexu_db::connect(isolated.url())
                .await
                .map_err(|error| error.to_string())?;
            crate::apply_directory(&db, &app, false)
                .await
                .map_err(|error| error.0)?;
            let clean = plan_database(&db, &app, Some(&products()))
                .await
                .map_err(|error| error.0)?;
            if clean.has_error() || clean.has_drift() || !clean.findings.is_empty() {
                return Err(format!("clean postgres plan was not quiet: {clean:?}"));
            }
            db.execute(Statement::from_string(
                DatabaseBackend::Postgres,
                "ALTER TABLE products ADD COLUMN extra integer",
            ))
            .await
            .map_err(|error| error.to_string())?;
            let drifted = plan_database(&db, &app, Some(&products()))
                .await
                .map_err(|error| error.0)?;
            if !drifted.has_drift()
                || !drifted
                    .findings
                    .iter()
                    .any(|item| item.message.contains("extra"))
            {
                return Err(format!("postgres drift was not reported: {drifted:?}"));
            }
            let before = plan_embedded(&db, &[], "_fexu_plan_probe", "Auth")
                .await
                .map_err(|error| error.0)?;
            if !before.has_error() {
                return Err(format!("empty embedded package was accepted: {before:?}"));
            }
            let sealed = crate::seal(
                "0001_probe".into(),
                "probe".into(),
                SchemaSnapshot::empty(),
                probe_table(),
                &crate::MigrationPlan {
                    operations: Vec::new(),
                    risks: Vec::new(),
                    refusal: None,
                },
            )
            .map_err(|error| error.0)?;
            let pending = plan_embedded(
                &db,
                std::slice::from_ref(&sealed),
                "_fexu_plan_probe",
                "Auth",
            )
            .await
            .map_err(|error| error.0)?;
            if pending.has_error() || pending.has_drift() || pending.pending != ["Auth:0001_probe"]
            {
                return Err(format!(
                    "pending embedded plan was not read-only: {pending:?}"
                ));
            }
            crate::apply_files_with_history(
                &db,
                std::slice::from_ref(&sealed),
                false,
                "_fexu_plan_probe",
            )
            .await
            .map_err(|error| error.0)?;
            db.execute(Statement::from_string(
                DatabaseBackend::Postgres,
                "CREATE TABLE plan_probe (id bigint NOT NULL PRIMARY KEY)",
            ))
            .await
            .map_err(|error| error.to_string())?;
            let quiet = plan_embedded(
                &db,
                std::slice::from_ref(&sealed),
                "_fexu_plan_probe",
                "Auth",
            )
            .await
            .map_err(|error| error.0)?;
            if quiet.has_error() || quiet.has_drift() {
                return Err(format!("applied embedded plan was not quiet: {quiet:?}"));
            }
            db.execute(Statement::from_string(
                DatabaseBackend::Postgres,
                "ALTER TABLE plan_probe ADD COLUMN embedded_extra integer",
            ))
            .await
            .map_err(|error| error.to_string())?;
            let embedded = plan_embedded(&db, &[sealed], "_fexu_plan_probe", "Auth")
                .await
                .map_err(|error| error.0)?;
            if !embedded.has_drift()
                || !embedded
                    .findings
                    .iter()
                    .any(|item| item.message.contains("embedded_extra"))
            {
                return Err(format!(
                    "embedded postgres drift was not reported: {embedded:?}"
                ));
            }
            Ok(())
        }
        .await;
        let dropped = isolated.drop_database().await;
        result.expect("postgres plan");
        dropped.expect("drop isolated database");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn embedded_plan_reports_drift_without_writing() {
        let db = fexu_db::connect("sqlite::memory:").await.unwrap();
        let empty = plan_embedded(&db, &[], "_fexu_plan_probe", "Auth")
            .await
            .unwrap();
        assert!(empty.has_error());
        assert!(empty
            .findings
            .iter()
            .any(|item| item.message.contains("Auth") && item.message.contains("missing")));
        assert!(
            names(&db).await.is_empty()
                || !names(&db)
                    .await
                    .iter()
                    .any(|name| name == "_fexu_plan_probe")
        );
        let sealed = crate::seal(
            "0001_products".into(),
            "products".into(),
            SchemaSnapshot::empty(),
            products(),
            &crate::MigrationPlan {
                operations: Vec::new(),
                risks: Vec::new(),
                refusal: None,
            },
        )
        .unwrap();
        let pending = plan_embedded(
            &db,
            std::slice::from_ref(&sealed),
            "_fexu_plan_probe",
            "Auth",
        )
        .await
        .unwrap();
        assert!(!pending.has_error());
        assert_eq!(pending.pending, vec!["Auth:0001_products".to_owned()]);
        assert!(!history::table_exists(&db, "_fexu_plan_probe")
            .await
            .unwrap());
        crate::apply_files_with_history(
            &db,
            std::slice::from_ref(&sealed),
            false,
            "_fexu_plan_probe",
        )
        .await
        .unwrap();
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "CREATE TABLE products (id integer NOT NULL PRIMARY KEY)",
        ))
        .await
        .unwrap();
        let quiet = plan_embedded(
            &db,
            std::slice::from_ref(&sealed),
            "_fexu_plan_probe",
            "Auth",
        )
        .await
        .unwrap();
        assert!(!quiet.has_error(), "{quiet:?}");
        assert!(!quiet.has_drift(), "{quiet:?}");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "ALTER TABLE products ADD COLUMN extra integer",
        ))
        .await
        .unwrap();
        let drifted = plan_embedded(&db, &[sealed], "_fexu_plan_probe", "Auth")
            .await
            .unwrap();
        assert!(drifted.has_drift());
        assert!(drifted
            .findings
            .iter()
            .any(|item| item.message.contains("extra")));
        let columns = db
            .query_all(Statement::from_string(
                DatabaseBackend::Sqlite,
                "PRAGMA table_info(products)",
            ))
            .await
            .unwrap();
        assert!(columns
            .iter()
            .any(|row| { row.try_get::<String>("", "name").unwrap_or_default() == "extra" }));
    }

    #[tokio::test]
    async fn embedded_plan_ignores_other_component_tables_and_keeps_owned_drift() {
        let root = scratch("scope");
        let app = root.join("migrations");
        std::fs::create_dir_all(&app).unwrap();
        make_migration(&app, "initial", &products()).unwrap();
        let db = fexu_db::connect(&sqlite_url(&root.join("db.sqlite")))
            .await
            .unwrap();
        crate::apply_directory(&db, &app, false).await.unwrap();
        let sealed = crate::seal(
            "0001_probe".into(),
            "probe".into(),
            SchemaSnapshot::empty(),
            probe_table(),
            &crate::MigrationPlan {
                operations: Vec::new(),
                risks: Vec::new(),
                refusal: None,
            },
        )
        .unwrap();
        crate::apply_files_with_history(
            &db,
            std::slice::from_ref(&sealed),
            false,
            "_fexu_plan_probe",
        )
        .await
        .unwrap();
        let missing = plan_embedded(
            &db,
            std::slice::from_ref(&sealed),
            "_fexu_plan_probe",
            "Auth",
        )
        .await
        .unwrap();
        assert!(missing.has_drift(), "{missing:?}");
        assert!(
            missing
                .findings
                .iter()
                .any(|item| item.message.contains("plan_probe") && item.message.contains("missing")),
            "{missing:?}"
        );
        assert!(
            missing
                .findings
                .iter()
                .all(|item| !item.message.contains("products")),
            "{missing:?}"
        );
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "CREATE TABLE plan_probe (id integer NOT NULL PRIMARY KEY)",
        ))
        .await
        .unwrap();
        let quiet = plan_embedded(
            &db,
            std::slice::from_ref(&sealed),
            "_fexu_plan_probe",
            "Auth",
        )
        .await
        .unwrap();
        assert!(!quiet.has_error(), "{quiet:?}");
        assert!(!quiet.has_drift(), "{quiet:?}");
        assert!(
            quiet
                .findings
                .iter()
                .all(|item| !item.message.contains("products")),
            "{quiet:?}"
        );
        let app_sees_probe = plan_database(&db, &app, Some(&products())).await.unwrap();
        assert!(app_sees_probe.has_drift(), "{app_sees_probe:?}");
        assert!(
            app_sees_probe
                .findings
                .iter()
                .any(|item| item.message.contains("plan_probe")),
            "{app_sees_probe:?}"
        );
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "ALTER TABLE plan_probe ADD COLUMN embedded_extra integer",
        ))
        .await
        .unwrap();
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "ALTER TABLE products ADD COLUMN extra integer",
        ))
        .await
        .unwrap();
        let embedded = plan_embedded(&db, &[sealed], "_fexu_plan_probe", "Auth")
            .await
            .unwrap();
        assert!(embedded.has_drift(), "{embedded:?}");
        assert!(
            embedded
                .findings
                .iter()
                .any(|item| item.message.contains("embedded_extra")),
            "{embedded:?}"
        );
        assert!(
            embedded
                .findings
                .iter()
                .all(|item| !item.message.contains("products")),
            "{embedded:?}"
        );
        let app_column = plan_database(&db, &app, Some(&products())).await.unwrap();
        assert!(
            app_column
                .findings
                .iter()
                .any(|item| item.message.contains("products.extra")),
            "{app_column:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dependency_conflicts_are_plan_errors_without_a_database() {
        let root = scratch("deps");
        let billing = crate::packages::PackageMigration {
            name: "billing".into(),
            directory: root.join("billing"),
            depends_on: vec!["accounts".into()],
        };
        let missing = package_dependency_findings(std::slice::from_ref(&billing));
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].level, PlanLevel::Error);
        assert!(
            missing[0].message.contains("accounts"),
            "{}",
            missing[0].message
        );
        assert!(
            missing[0].message.contains("missing app"),
            "{}",
            missing[0].message
        );
        let cycle = package_dependency_findings(&[
            crate::packages::PackageMigration {
                name: "accounts".into(),
                directory: root.join("accounts"),
                depends_on: vec!["billing".into()],
            },
            billing,
        ]);
        assert!(cycle[0].message.contains("cycle"), "{}", cycle[0].message);
        assert!(!root.join("billing").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
