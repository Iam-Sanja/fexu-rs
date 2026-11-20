//! Persisted auth/session continuity across a database reconnect and a repeat
//! application of the auth migration set. The six embedded migration artifacts
//! were verified byte-identical to tag v0.1.2 on 2026-11-20. This test does NOT
//! execute the published v0.1.2 auth library or a v0.2.0 candidate.
use crate::password::ephemeral_password;
use crate::{AuthConfig, AuthError, AuthService};

async fn assert_continuity(url: &str) -> Result<(), String> {
    let db = fexu_db::connect(url)
        .await
        .map_err(|error| error.to_string())?;
    let applied = crate::apply_migrations(&db)
        .await
        .map_err(|error| error.to_string())?;
    if applied.len() != 6 {
        return Err(format!("expected six fresh auth migrations: {applied:?}"));
    }
    let auth = AuthService::new(db.clone(), AuthConfig::development());
    let password = ephemeral_password();
    let user = auth
        .create_user("existing-admin@example.test", &password, true, true)
        .await
        .map_err(|error| error.to_string())?;
    auth.grant_permission("release-operator", "reports.read")
        .await
        .map_err(|error| error.to_string())?;
    auth.assign_role(&user.email, "release-operator")
        .await
        .map_err(|error| error.to_string())?;
    let session = auth
        .login(&user.email, &password, "release-fixture")
        .await
        .map_err(|error| error.to_string())?;
    let original_session = auth
        .authenticate_token(&session.token)
        .await
        .map_err(|error| error.to_string())?;
    auth.authorize(&original_session, Some("reports"), "read")
        .await
        .map_err(|error| error.to_string())?;
    let session_id = original_session.session_id;
    drop(auth);
    db.close().await.map_err(|error| error.to_string())?;

    // Simulates reopening the existing database with the new framework
    // checkout. No session rows, password hashes, roles or secrets are reset.
    let db = fexu_db::connect(url)
        .await
        .map_err(|error| error.to_string())?;
    let reapplied = crate::apply_migrations(&db)
        .await
        .map_err(|error| error.to_string())?;
    if !reapplied.is_empty() {
        return Err(format!(
            "existing auth history was reapplied: {reapplied:?}"
        ));
    }
    let auth = AuthService::new(db.clone(), AuthConfig::development());
    let restored = auth
        .authenticate_token(&session.token)
        .await
        .map_err(|error| error.to_string())?;
    if restored.id != user.id
        || restored.session_id != session_id
        || !restored.is_staff
        || !restored.is_superuser
    {
        return Err("existing account/session identity changed after reconnect".into());
    }
    auth.authorize(&restored, Some("reports"), "read")
        .await
        .map_err(|error| error.to_string())?;
    if !auth
        .list_users()
        .await
        .map_err(|error| error.to_string())?
        .iter()
        .any(|found| found.id == user.id)
    {
        return Err("previous account disappeared after reconnect".into());
    }
    if !matches!(
        auth.login(&user.email, &ephemeral_password(), "release-fixture")
            .await,
        Err(AuthError::InvalidCredentials)
    ) {
        return Err("invalid password was accepted after reconnect".into());
    }
    let fresh = auth
        .login(&user.email, &password, "release-fixture")
        .await
        .map_err(|error| error.to_string())?;
    if fresh.user.id != user.id || fresh.token == session.token {
        return Err("existing password was not accepted or session token was reused".into());
    }
    drop(auth);
    db.close().await.map_err(|error| error.to_string())?;
    Ok(())
}

#[tokio::test]
async fn sqlite_persisted_auth_and_session_survive_migrations_and_reconnect() {
    let dir = std::env::temp_dir().join(format!(
        "fexu-auth-upgrade-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.join("auth.sqlite").display());
    let result = assert_continuity(&url).await;
    let _ = std::fs::remove_dir_all(&dir);
    result.expect("SQLite persisted authentication continuity");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL pointing at an approved PostgreSQL admin database"]
async fn postgres_persisted_auth_and_session_survive_migrations_and_reconnect() {
    let admin = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let fixture = fexu_db::DisposablePostgres::create(&admin)
        .await
        .expect("create isolated database");
    let result = assert_continuity(fixture.url()).await;
    let cleanup = fixture.drop_database().await;
    assert!(
        cleanup.is_ok(),
        "cleanup of isolated PostgreSQL test: {cleanup:?}"
    );
    result.expect("PostgreSQL persisted authentication continuity");
}
