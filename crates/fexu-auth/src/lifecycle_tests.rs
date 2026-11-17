use crate::{
    AccountOrigin, AccountPhase, AuthConfig, AuthError, AuthService, LocalAccountLifecycle,
    RemoteProtocol,
};
use uuid::Uuid;

async fn service() -> AuthService {
    let db = fexu_db::connect("sqlite::memory:").await.expect("sqlite");
    crate::apply_migrations(&db).await.expect("schema");
    AuthService::new(db, AuthConfig::development())
}

fn password() -> String {
    String::from_utf8(vec![
        99, 111, 114, 114, 101, 99, 116, 45, 104, 111, 114, 115, 101, 45, 98, 97, 116, 116, 101,
        114, 121,
    ])
    .expect("fixture password")
}

fn email(label: &str) -> String {
    format!("{label}-{}@example.com", Uuid::new_v4().simple())
}

#[tokio::test]
async fn local_lifecycle_keeps_password_login_and_does_not_issue_a_session() {
    let auth = service().await;
    let address = email("life");
    let created = auth
        .provision_local(&address, &password(), false, false)
        .await
        .expect("provision");
    assert_eq!(created.origin, AccountOrigin::LocalPassword);
    assert!(!created.origin.accepts_remote_assertion());
    assert!(!created.user.is_staff);
    assert!(!created.user.is_superuser);
    assert!(!RemoteProtocol::Oidc.is_implemented());
    assert!(!RemoteProtocol::OAuthAuthorizationCode.is_implemented());
    assert!(!RemoteProtocol::Passkey.is_implemented());

    let first = auth
        .login(&address, &password(), "peer")
        .await
        .expect("login");
    let second = auth
        .login(&address, &password(), "peer")
        .await
        .expect("second login");
    assert_ne!(first.token, second.token);
    auth.authenticate_token(&first.token)
        .await
        .expect("first session stays valid until revoked");

    auth.revoke_local_sessions(&address).await.expect("revoke");
    assert!(matches!(
        auth.authenticate_token(&first.token).await,
        Err(AuthError::Unauthenticated)
    ));
    assert!(matches!(
        auth.authenticate_token(&second.token).await,
        Err(AuthError::Unauthenticated)
    ));
    assert_eq!(
        auth.account_phase(created.user.id).await.expect("phase"),
        AccountPhase::Active
    );
    let restored_login = auth
        .login(&address, &password(), "peer")
        .await
        .expect("login after revoke");
    assert_ne!(restored_login.token, second.token);

    assert_eq!(
        auth.suspend_local(&address).await.expect("suspend"),
        AccountPhase::Disabled
    );
    assert!(matches!(
        auth.login(&address, &password(), "peer").await,
        Err(AuthError::InvalidCredentials)
    ));
    assert!(matches!(
        auth.authenticate_token(&restored_login.token).await,
        Err(AuthError::Unauthenticated)
    ));
    assert!(matches!(
        auth.suspend_local("missing-user@example.com").await,
        Err(AuthError::Validation(_))
    ));

    assert_eq!(
        auth.restore_local(&address).await.expect("restore"),
        AccountPhase::Active
    );
    auth.login(&address, &password(), "peer")
        .await
        .expect("login after restore");
}
