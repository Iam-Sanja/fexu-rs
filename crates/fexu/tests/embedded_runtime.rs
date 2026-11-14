//! Embedded framework migrations and the read-only readiness split.

use fexu::auth::{AuthConfig, AuthService};
use fexu::db::DisposablePostgres;
use fexu::sea_orm::ConnectionTrait;

#[tokio::test]
async fn readiness_reports_auth_jobs_and_an_absent_app_without_migrating() {
    let db = fexu::db::connect("sqlite::memory:").await.unwrap();
    let auth_files = fexu::auth::embedded_migrations().unwrap();
    let jobs_files = fexu::jobs::embedded_migrations().unwrap();
    let absent = std::env::temp_dir().join(format!("fexu-absent-app-{}", std::process::id()));
    let report = fexu::migrations::inspect_framework(
        &db,
        &auth_files,
        &jobs_files,
        fexu::jobs::JOB_HISTORY,
        &absent,
        None,
        &[],
    )
    .await
    .unwrap();
    assert!(report.pending_auth() >= 6, "{:?}", report.auth.lines);
    assert_eq!(report.pending_jobs(), 2);
    assert_eq!(report.pending_app(), 0);
    assert!(
        report.app.schema_note.contains("absent"),
        "{}",
        report.app.schema_note
    );
    assert!(
        !fexu::migrations::table_exists(&db, "_fexu_auth_migrations")
            .await
            .unwrap()
    );
    assert!(
        !fexu::migrations::table_exists(&db, fexu::jobs::JOB_HISTORY)
            .await
            .unwrap()
    );
    assert!(!fexu::migrations::table_exists(&db, "_fexu_users")
        .await
        .unwrap());
}

#[tokio::test]
async fn sqlite_embedded_auth_and_jobs_cover_the_failure_matrix() {
    exercise("sqlite::memory:").await.unwrap();
}

#[tokio::test]
#[ignore = "requires DATABASE_URL pointing at an approved PostgreSQL admin database"]
async fn postgres_embedded_auth_and_jobs() {
    let admin = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let isolated = DisposablePostgres::create(&admin)
        .await
        .expect("isolated database");
    let result = exercise(isolated.url()).await;
    let dropped = isolated.drop_database().await;
    result.unwrap();
    dropped.expect("drop isolated database");
}

async fn exercise(url: &str) -> Result<(), String> {
    let db = fexu::db::connect(url)
        .await
        .map_err(|error| error.to_string())?;
    let auth_files = fexu::auth::embedded_migrations().map_err(|error| error.to_string())?;
    let jobs_files = fexu::jobs::embedded_migrations()
        .map_err(|error| format!("{} {}", error.code(), error.detail()))?;
    if auth_files.len() != 6 || jobs_files.len() != 2 {
        return Err(format!(
            "embedded package size auth={} jobs={}",
            auth_files.len(),
            jobs_files.len()
        ));
    }
    let empty_app = std::env::temp_dir().join(format!(
        "fexu-empty-app-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&empty_app).map_err(|error| error.to_string())?;
    let absent_app = std::env::temp_dir().join(format!(
        "fexu-absent-app-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    let missing = fexu::migrations::inspect_framework(
        &db,
        &[],
        &jobs_files,
        fexu::jobs::JOB_HISTORY,
        &empty_app,
        None,
        &[],
    )
    .await;
    if missing.is_ok() {
        return Err("empty auth resources were reported as a successful inspection".into());
    }
    let missing = missing.unwrap_err().0;
    if !missing.contains("fexu-auth") || !missing.contains("missing") {
        return Err(format!("missing auth resources were not named: {missing}"));
    }
    if fexu::migrations::table_exists(&db, "_fexu_users")
        .await
        .map_err(|error| error.0)?
    {
        return Err("a failed readiness check created _fexu_users".into());
    }
    let ready = fexu::migrations::inspect_framework(
        &db,
        &auth_files,
        &jobs_files,
        fexu::jobs::JOB_HISTORY,
        &empty_app,
        None,
        &[],
    )
    .await
    .map_err(|error| error.0)?;
    if ready.pending_auth() < 6 || ready.pending_jobs() != 2 || ready.pending_app() != 0 {
        return Err(format!(
            "fresh readiness was not pending auth/jobs with an empty app: {ready:?}"
        ));
    }
    if ready.has_error() {
        return Err(format!(
            "fresh database was reported as a framework error: {ready:?}"
        ));
    }
    let absent = fexu::migrations::inspect_framework(
        &db,
        &auth_files,
        &jobs_files,
        fexu::jobs::JOB_HISTORY,
        &absent_app,
        None,
        &[],
    )
    .await
    .map_err(|error| error.0)?;
    if !absent.app.schema_note.contains("absent") {
        return Err(format!(
            "absent app directory was not described: {}",
            absent.app.schema_note
        ));
    }
    if fexu::migrations::table_exists(&db, "_fexu_auth_migrations")
        .await
        .map_err(|error| error.0)?
        || fexu::migrations::table_exists(&db, fexu::jobs::JOB_HISTORY)
            .await
            .map_err(|error| error.0)?
    {
        return Err("readiness created a history table".into());
    }
    let auth_applied = fexu::auth::apply_migrations(&db)
        .await
        .map_err(|error| error.to_string())?;
    if auth_applied.len() != 6 {
        return Err(format!(
            "expected 6 auth migrations, applied {auth_applied:?}"
        ));
    }
    let jobs_applied = fexu::jobs::apply_migrations(&db)
        .await
        .map_err(|error| format!("{} {}", error.code(), error.detail()))?;
    if jobs_applied.len() != 2 {
        return Err(format!(
            "expected 2 job migrations, applied {jobs_applied:?}"
        ));
    }
    let auth = AuthService::new(db.clone(), AuthConfig::development());
    let password = ephemeral_password();
    auth.create_user("embedded-runtime@example.com", &password, true, false)
        .await
        .map_err(|error| error.to_string())?;
    auth.login("embedded-runtime@example.com", &password, "127.0.0.1")
        .await
        .map_err(|error| error.to_string())?;
    let again = fexu::auth::apply_migrations(&db)
        .await
        .map_err(|error| error.to_string())?;
    if !again.is_empty() {
        return Err(format!("second auth apply changed history: {again:?}"));
    }
    let jobs_again = fexu::jobs::apply_migrations(&db)
        .await
        .map_err(|error| format!("{} {}", error.code(), error.detail()))?;
    if !jobs_again.is_empty() {
        return Err(format!("second job apply changed history: {jobs_again:?}"));
    }
    let status = fexu::auth::migration_status(&db)
        .await
        .map_err(|error| error.to_string())?;
    if status.lines.iter().any(|line| line.state != "angewendet") || !status.conflicts.is_empty() {
        return Err(format!("auth status is not applied: {status:?}"));
    }
    let queue = fexu::jobs::Queue::new(
        db.clone(),
        std::sync::Arc::new(fexu::jobs::SystemClock),
        fexu::jobs::JobPolicy::default(),
    );
    let created = queue
        .enqueue_json(
            "embedded_probe",
            1,
            &serde_json::json!({"kind": "probe"}),
            fexu::jobs::When::now(),
        )
        .await
        .map_err(|error| format!("{} {}", error.code(), error.detail()))?;
    let fexu::jobs::EnqueueOutcome::Created(id) = created else {
        return Err(format!("enqueue did not create a job: {created:?}"));
    };
    let loaded = queue
        .get(id)
        .await
        .map_err(|error| format!("{} {}", error.code(), error.detail()))?;
    if loaded.handler != "embedded_probe" {
        return Err(format!("loaded job has handler {}", loaded.handler));
    }
    let auth_checksum = auth_files
        .iter()
        .find(|file| file.id == "0001_auth_schema")
        .map(|file| file.checksum.clone())
        .ok_or_else(|| "0001_auth_schema is missing from the embedded package".to_owned())?;
    if !auth_checksum
        .chars()
        .all(|character| character.is_ascii_hexdigit())
    {
        return Err("auth checksum is not hexadecimal".into());
    }
    exec(
        &db,
        "UPDATE \"_fexu_auth_migrations\" SET checksum = '00' WHERE id = '0001_auth_schema'",
    )
    .await?;
    let mismatch = fexu::auth::apply_migrations(&db).await;
    if mismatch.is_ok() {
        return Err("checksum mismatch was applied again".into());
    }
    let mismatch = mismatch.unwrap_err().to_string();
    if !mismatch.to_ascii_lowercase().contains("checksum") {
        return Err(format!("checksum mismatch was not named: {mismatch}"));
    }
    let conflicted = fexu::auth::migration_status(&db)
        .await
        .map_err(|error| error.to_string())?;
    if !conflicted
        .conflicts
        .iter()
        .any(|conflict| conflict.to_ascii_lowercase().contains("checksum"))
    {
        return Err(format!(
            "status did not keep the checksum conflict: {conflicted:?}"
        ));
    }
    exec(
        &db,
        &format!(
            "UPDATE \"_fexu_auth_migrations\" SET checksum = '{auth_checksum}' WHERE id = '0001_auth_schema'"
        ),
    )
    .await?;
    exec(&db, "DROP TABLE \"_fexu_login_challenges\"").await?;
    let dropped = fexu::auth::migration_status(&db)
        .await
        .map_err(|error| error.to_string())?;
    if !dropped.conflicts.iter().any(|conflict| {
        conflict.contains("_fexu_login_challenges")
            && conflict.to_ascii_lowercase().contains("missing")
    }) {
        return Err(format!(
            "missing auth table was reported as success: {dropped:?}"
        ));
    }
    let replay = fexu::auth::apply_migrations(&db)
        .await
        .map_err(|error| error.to_string())?;
    if !replay.is_empty() {
        return Err(format!(
            "apply recreated history after a dropped table: {replay:?}"
        ));
    }
    if fexu::migrations::table_exists(&db, "_fexu_login_challenges")
        .await
        .map_err(|error| error.0)?
    {
        return Err("apply silently recreated _fexu_login_challenges".into());
    }
    exec(&db, "DROP TABLE \"_fexu_jobs\"").await?;
    let jobs_dropped = fexu::jobs::migration_status(&db)
        .await
        .map_err(|error| format!("{} {}", error.code(), error.detail()))?;
    if !jobs_dropped.conflicts.iter().any(|conflict| {
        conflict.contains("_fexu_jobs") && conflict.to_ascii_lowercase().contains("missing")
    }) {
        return Err(format!(
            "missing jobs table was reported as success: {jobs_dropped:?}"
        ));
    }
    let jobs_replay = fexu::jobs::apply_migrations(&db)
        .await
        .map_err(|error| format!("{} {}", error.code(), error.detail()))?;
    if !jobs_replay.is_empty() {
        return Err(format!(
            "job apply recreated history after a dropped table: {jobs_replay:?}"
        ));
    }
    if fexu::migrations::table_exists(&db, "_fexu_jobs")
        .await
        .map_err(|error| error.0)?
    {
        return Err("apply silently recreated _fexu_jobs".into());
    }
    let report = fexu::migrations::inspect_framework(
        &db,
        &auth_files,
        &jobs_files,
        fexu::jobs::JOB_HISTORY,
        &empty_app,
        None,
        &[],
    )
    .await
    .map_err(|error| error.0)?;
    if !report.has_error() {
        return Err("inspect_framework treated missing framework tables as success".into());
    }
    let text = fexu::migrations::render_diagnosis("de", &report);
    if text.contains("Konflikte: keine") || !text.contains("_fexu_login_challenges") {
        return Err(format!("diagnosis hid the missing auth table: {text}"));
    }
    let notice = fexu::migrations::dev_notice("en", &Ok(report));
    if !notice.contains("[WARN]") || !notice.contains("conflicts") {
        return Err(format!("dev notice claimed a clean database: {notice}"));
    }
    let _ = std::fs::remove_dir_all(&empty_app);
    Ok(())
}

async fn exec(db: &fexu::sea_orm::DatabaseConnection, sql: &str) -> Result<(), String> {
    db.execute(fexu::sea_orm::Statement::from_string(
        db.get_database_backend(),
        sql,
    ))
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

fn ephemeral_password() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}
