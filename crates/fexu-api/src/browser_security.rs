//! HTTP coverage for Referrer-Policy, Origin, Fetch Metadata, and native admin login.
//!
//! The listener is `127.0.0.1`. LAN and HTTPS cases use configured origins in headers:
//! `http://192.168.178.49:3000` and `https://shop.example`. No TLS listener
//! or LAN socket is opened. Public account registration is disabled; a fresh
//! browser session starts through `GET /auth/anonymous` and `GET /admin/login`.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use fexu_admin::AdminRegistry;
use fexu_auth::{apply_migrations, AppState, AuthConfig, AuthService};
use fexu_db::driver::{ConnectionTrait, DatabaseConnection, Statement};
use fexu_middleware::{apply, ReferrerPolicy, Standard};

use super::App;

const LOCAL: &str = "http://127.0.0.1:3000";
const LOCALHOST: &str = "http://localhost:3000";
const LAN: &str = "http://192.168.178.49:3000";
const HTTPS: &str = "https://shop.example";
const FOREIGN: &str = "http://evil.example";

#[tokio::test]
async fn sqlite_browser_security_matrix() {
    let db = fexu_db::connect("sqlite::memory:").await.expect("sqlite");
    exercise(db).await.expect("sqlite browser security");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL pointing at an approved PostgreSQL admin database"]
async fn postgres_browser_security_matrix() {
    let admin = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let isolated = fexu_db::DisposablePostgres::create(&admin)
        .await
        .expect("isolated database");
    let db = fexu_db::connect(isolated.url()).await.expect("postgres");
    let result = exercise(db).await;
    let dropped = isolated.drop_database().await;
    result.expect("postgres browser security");
    dropped.expect("drop isolated database");
}

async fn exercise(db: DatabaseConnection) -> Result<(), String> {
    apply_migrations(&db)
        .await
        .map_err(|error| error.to_string())?;
    let password = fixture_password();
    let dev = development_config();
    let auth = AuthService::new(db.clone(), dev.clone());
    auth.create_user("ada@example.com", &password, true, false)
        .await
        .map_err(|error| error.to_string())?;
    let dev_address = listen(stack(db.clone(), dev)).await;
    let secure = secure_config();
    let secure_address = listen(stack(db.clone(), secure)).await;
    let loopback = loopback_config();
    let loopback_address = listen(stack(db.clone(), loopback)).await;

    document_headers(&dev_address).await?;
    fresh_and_session_states(&dev_address, &db, &password).await?;
    anonymous_registration_and_other_writes(&dev_address).await?;
    reset_stays_isolated(&dev_address).await?;
    dedicated_cookies_do_not_authorize_other_writes(&dev_address, &password).await?;
    recovery_errors_keep_no_referrer(db.clone()).await?;
    https_configuration(&secure_address, &password).await?;
    loopback_bind_is_not_an_origin(&loopback_address, &password).await?;
    Ok(())
}

fn development_config() -> AuthConfig {
    let mut config = AuthConfig::development();
    config.allowed_origins = vec![LOCAL.to_owned(), LOCALHOST.to_owned(), LAN.to_owned()];
    config.trust_proxy = true;
    config.trusted_proxies = vec!["127.0.0.1".to_owned()];
    config.cookie_secure = false;
    config
}

fn secure_config() -> AuthConfig {
    let mut config = AuthConfig::development();
    config.cookie_secure = true;
    config.allowed_origins = vec![HTTPS.to_owned()];
    config.trust_proxy = true;
    config.trusted_proxies = vec!["127.0.0.1".to_owned()];
    config
}

fn loopback_config() -> AuthConfig {
    let mut config = AuthConfig::development();
    config.cookie_secure = false;
    assert!(config.with_loopback_address("0.0.0.0:3000"));
    assert_eq!(
        config.allowed_origins,
        vec![LOCAL.to_owned(), LOCALHOST.to_owned()]
    );
    config
}

fn stack(db: DatabaseConnection, config: AuthConfig) -> Router {
    let auth = AuthService::new(db.clone(), config);
    let app = App::new(db)
        .with_auth(auth)
        .with_admin(AdminRegistry::new())
        .merge(extra_routes())
        .require_browser_writes()
        .into_router();
    apply(app, Standard::default())
}

fn extra_routes() -> Router<AppState> {
    Router::new()
        .route("/shop", get(shop))
        .route("/probe", post(probe))
        .route("/unsafe-referrer", get(unsafe_referrer))
}

async fn shop(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let Some(auth) = state.auth.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let Ok(session) = auth.ensure_anonymous_session(&headers).await else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let mut response = (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        "<a href=\"/admin/login\">Login</a>",
    )
        .into_response();
    if let Some(cookie) = session.cookie {
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    response
}

async fn probe() -> &'static str {
    "written"
}

async fn unsafe_referrer() -> impl IntoResponse {
    ([(header::REFERRER_POLICY, "unsafe-url")], "ok")
}

async fn listen(router: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    address
}

async fn document_headers(address: &SocketAddr) -> Result<(), String> {
    let mut client = Client::new(*address);
    let health = client.call("GET", "/healthz", &[], None).await?;
    expect_status(&health, 200, "health")?;
    expect_header(&health, "referrer-policy", "no-referrer")?;
    expect_header(&health, "x-content-type-options", "nosniff")?;
    expect_header(&health, "x-frame-options", "DENY")?;
    expect_one(&health, "referrer-policy")?;

    let unsafe_page = client.call("GET", "/unsafe-referrer", &[], None).await?;
    expect_status(&unsafe_page, 200, "unsafe referrer")?;
    expect_header(&unsafe_page, "referrer-policy", "no-referrer")?;
    if unsafe_page.head.to_ascii_lowercase().contains("unsafe-url") {
        return Err(format!("unsafe referrer survived: {}", unsafe_page.head));
    }
    expect_header(&unsafe_page, "x-frame-options", "DENY")?;

    let login = client.call("GET", "/admin/login", &[], None).await?;
    expect_status(&login, 200, "login document")?;
    expect_header(&login, "referrer-policy", "same-origin")?;
    expect_header(&login, "x-content-type-options", "nosniff")?;
    expect_header(&login, "x-frame-options", "DENY")?;
    expect_one(&login, "referrer-policy")?;
    let token = form_token(&login.body)?;
    let cookie = client
        .jar
        .get("fexu_login_csrf")
        .ok_or("login csrf cookie missing")?;
    if cookie != token {
        return Err("login form token and cookie differ".into());
    }

    let recovery = client
        .call("GET", "/auth/password-reset", &[], None)
        .await?;
    expect_status(&recovery, 200, "recovery document")?;
    expect_header(&recovery, "referrer-policy", "no-referrer")?;
    if recovery.head.to_ascii_lowercase().contains("same-origin") {
        return Err("recovery lost no-referrer".into());
    }
    Ok(())
}

async fn fresh_and_session_states(
    address: &SocketAddr,
    db: &DatabaseConnection,
    password: &str,
) -> Result<(), String> {
    direct_fresh(address, password).await?;
    after_navigation(address, password).await?;
    expired_anonymous(address, db, password).await?;
    invalid_user_session(address, password).await?;
    expired_user_session(address, db, password).await?;
    Ok(())
}

async fn direct_fresh(address: &SocketAddr, password: &str) -> Result<(), String> {
    let mut client = Client::new(*address);
    let bare = client
        .post_login(
            Some(LOCAL),
            None,
            &login_body(&"ab".repeat(32), password),
            &[],
        )
        .await?;
    expect_status(&bare, 403, "fresh jar without cookies")?;
    client.call("GET", "/admin/login", &[], None).await?;
    let token = issued_token(&client)?;
    rejected(
        &mut client,
        None,
        None,
        "missing",
        password,
        "fresh missing token",
    )
    .await?;
    rejected(
        &mut client,
        Some(LOCAL),
        None,
        "mismatch",
        password,
        "fresh mismatch",
    )
    .await?;
    rejected(
        &mut client,
        Some(FOREIGN),
        None,
        &token,
        password,
        "fresh foreign origin",
    )
    .await?;
    rejected(
        &mut client,
        Some("null"),
        None,
        &token,
        password,
        "fresh null without fetch metadata",
    )
    .await?;
    rejected(
        &mut client,
        Some("null"),
        Some("cross-site"),
        &token,
        password,
        "fresh null cross-site",
    )
    .await?;
    rejected(
        &mut client,
        Some("null"),
        Some("same-site"),
        &token,
        password,
        "fresh null same-site",
    )
    .await?;
    let forwarded = client
        .post_login(
            Some(FOREIGN),
            None,
            &login_body(&token, password),
            &[("x-forwarded-host", LOCAL), ("host", "192.168.178.49:3000")],
        )
        .await?;
    expect_status(&forwarded, 403, "host header is not an origin")?;
    client.store(&forwarded);
    if client.jar.contains("fexu_session") {
        return Err("rejected login issued a session".into());
    }
    succeed(
        &mut client,
        Some(LOCAL),
        None,
        &token,
        password,
        "fresh loopback",
    )
    .await?;
    client.jar.clear();
    client.call("GET", "/admin/login", &[], None).await?;
    let token = issued_token(&client)?;
    succeed(
        &mut client,
        Some(LOCALHOST),
        None,
        &token,
        password,
        "fresh localhost",
    )
    .await?;
    client.jar.clear();
    client.call("GET", "/admin/login", &[], None).await?;
    let token = issued_token(&client)?;
    succeed(&mut client, Some(LAN), None, &token, password, "fresh lan").await?;
    client.jar.clear();
    client.call("GET", "/admin/login", &[], None).await?;
    let token = issued_token(&client)?;
    succeed(
        &mut client,
        None,
        None,
        &token,
        password,
        "fresh absent origin",
    )
    .await?;
    client.jar.clear();
    client.call("GET", "/admin/login", &[], None).await?;
    let token = issued_token(&client)?;
    succeed(
        &mut client,
        Some("null"),
        Some("same-origin"),
        &token,
        password,
        "fresh null same-origin",
    )
    .await?;
    let wrong = Client::new(*address);
    let mut wrong = wrong;
    wrong.call("GET", "/admin/login", &[], None).await?;
    let token = issued_token(&wrong)?;
    let response = wrong
        .post_login(
            Some(LOCAL),
            None,
            &encode(&[
                ("csrf_token", &token),
                ("email", "ada@example.com"),
                ("password", "not-the-password"),
                ("next", "/admin"),
            ]),
            &[],
        )
        .await?;
    expect_status(&response, 401, "wrong password still passes the gate")?;
    expect_header(&response, "referrer-policy", "same-origin")?;
    wrong.store(&response);
    if wrong.jar.contains("fexu_session") {
        return Err("wrong password issued a session".into());
    }
    Ok(())
}

async fn after_navigation(address: &SocketAddr, password: &str) -> Result<(), String> {
    let mut client = Client::new(*address);
    let shop = client
        .call("GET", "/shop", &[("origin", LOCAL)], None)
        .await?;
    expect_status(&shop, 200, "shop")?;
    if !client.jar.contains("fexu_anonymous") {
        return Err("navigation did not issue an anonymous session".into());
    }
    client
        .call("GET", "/admin/login", &[("origin", LOCAL)], None)
        .await?;
    let token = issued_token(&client)?;
    let anonymous = client.jar.get("fexu_anonymous").unwrap().to_owned();
    rejected(
        &mut client,
        Some(LOCAL),
        None,
        "missing",
        password,
        "navigation missing token",
    )
    .await?;
    rejected(
        &mut client,
        Some(LOCAL),
        None,
        "mismatch",
        password,
        "navigation mismatch",
    )
    .await?;
    rejected(
        &mut client,
        Some(LOCAL),
        None,
        &anonymous,
        password,
        "anonymous csrf is not a login proof",
    )
    .await?;
    rejected(
        &mut client,
        Some(FOREIGN),
        None,
        &token,
        password,
        "navigation foreign origin",
    )
    .await?;
    succeed(
        &mut client,
        Some(LOCAL),
        None,
        &token,
        password,
        "login after navigation",
    )
    .await?;
    if client.jar.contains("fexu_anonymous") {
        return Err("login kept the anonymous session".into());
    }
    Ok(())
}

async fn expired_anonymous(
    address: &SocketAddr,
    db: &DatabaseConnection,
    password: &str,
) -> Result<(), String> {
    let mut client = Client::new(*address);
    client
        .call("GET", "/shop", &[("origin", LOCAL)], None)
        .await?;
    expire(db, "_fexu_anonymous_sessions").await?;
    client
        .call("GET", "/admin/login", &[("origin", LOCAL)], None)
        .await?;
    let token = issued_token(&client)?;
    if !client.jar.contains("fexu_anonymous") {
        return Err("expired anonymous cookie was dropped by the test client".into());
    }
    rejected(
        &mut client,
        Some(LOCAL),
        None,
        "missing",
        password,
        "expired anonymous missing token",
    )
    .await?;
    rejected(
        &mut client,
        Some(LOCAL),
        None,
        "mismatch",
        password,
        "expired anonymous mismatch",
    )
    .await?;
    succeed(
        &mut client,
        Some(LOCALHOST),
        None,
        &token,
        password,
        "expired anonymous still logs in",
    )
    .await?;
    Ok(())
}

async fn invalid_user_session(address: &SocketAddr, password: &str) -> Result<(), String> {
    let mut client = Client::new(*address);
    client.call("GET", "/admin/login", &[], None).await?;
    client
        .jar
        .insert("fexu_session", "not-a-live-session".to_owned());
    let token = issued_token(&client)?;
    rejected(
        &mut client,
        Some(LOCAL),
        None,
        "missing",
        password,
        "invalid session missing token",
    )
    .await?;
    rejected(
        &mut client,
        Some(LOCAL),
        None,
        "mismatch",
        password,
        "invalid session mismatch",
    )
    .await?;
    rejected(
        &mut client,
        Some("null"),
        None,
        &token,
        password,
        "invalid session unverified null",
    )
    .await?;
    succeed(
        &mut client,
        Some(LAN),
        None,
        &token,
        password,
        "invalid session with login csrf",
    )
    .await?;
    Ok(())
}

async fn expired_user_session(
    address: &SocketAddr,
    db: &DatabaseConnection,
    password: &str,
) -> Result<(), String> {
    let mut client = Client::new(*address);
    client.call("GET", "/admin/login", &[], None).await?;
    let token = issued_token(&client)?;
    succeed(
        &mut client,
        Some(LOCAL),
        None,
        &token,
        password,
        "session to expire",
    )
    .await?;
    expire(db, "_fexu_sessions").await?;
    client.call("GET", "/admin/login", &[], None).await?;
    let token = issued_token(&client)?;
    if !client.jar.contains("fexu_session") {
        return Err("expired user cookie was dropped by the test client".into());
    }
    rejected(
        &mut client,
        Some(LOCAL),
        None,
        "missing",
        password,
        "expired user missing token",
    )
    .await?;
    rejected(
        &mut client,
        Some(LOCAL),
        None,
        "mismatch",
        password,
        "expired user mismatch",
    )
    .await?;
    succeed(
        &mut client,
        Some(LOCAL),
        None,
        &token,
        password,
        "expired user session still logs in",
    )
    .await?;
    let admin = client
        .call("GET", "/admin", &[("origin", LOCAL)], None)
        .await?;
    expect_status(&admin, 200, "admin after login")?;
    Ok(())
}

async fn anonymous_registration_and_other_writes(address: &SocketAddr) -> Result<(), String> {
    let mut client = Client::new(*address);
    let created = client
        .call("GET", "/auth/anonymous", &[("origin", LOCAL)], None)
        .await?;
    expect_status(&created, 200, "anonymous registration")?;
    let token = json_token(&created.body)?;
    if !client.jar.contains("fexu_anonymous") {
        return Err("anonymous cookie missing".into());
    }
    let written = client
        .call(
            "POST",
            "/probe",
            &[
                ("origin", LOCAL),
                ("content-type", "application/json"),
                ("x-csrf-token", &token),
            ],
            Some("{}"),
        )
        .await?;
    expect_status(&written, 200, "anonymous csrf write")?;
    if written.body != "written" {
        return Err(format!("probe body {}", written.body));
    }
    let foreign = client
        .call(
            "POST",
            "/probe",
            &[
                ("origin", FOREIGN),
                ("content-type", "application/json"),
                ("x-csrf-token", &token),
            ],
            Some("{}"),
        )
        .await?;
    expect_status(&foreign, 403, "anonymous foreign origin")?;
    let mismatched = client
        .call(
            "POST",
            "/probe",
            &[
                ("origin", LOCAL),
                ("content-type", "application/json"),
                ("x-csrf-token", "not-the-token"),
            ],
            Some("{}"),
        )
        .await?;
    expect_status(&mismatched, 403, "anonymous mismatched csrf")?;

    let mut bare = Client::new(*address);
    let blocked = bare
        .call(
            "POST",
            "/probe",
            &[("origin", LOCAL), ("content-type", "application/json")],
            Some("{}"),
        )
        .await?;
    expect_status(&blocked, 403, "cookieless probe")?;
    let bearer = bare
        .call(
            "POST",
            "/probe",
            &[
                ("origin", LOCAL),
                ("authorization", "Bearer opaque-client"),
                ("content-type", "application/json"),
            ],
            Some("{}"),
        )
        .await?;
    expect_status(&bearer, 403, "bearer is not a session")?;
    let api = bare
        .call(
            "POST",
            "/auth/login",
            &[("origin", LOCAL), ("content-type", "application/json")],
            Some("{}"),
        )
        .await?;
    expect_status(&api, 403, "json login without a session stays closed")?;
    Ok(())
}

async fn reset_stays_isolated(address: &SocketAddr) -> Result<(), String> {
    let mut client = Client::new(*address);
    let page = client
        .call("GET", "/auth/password-reset", &[("origin", LOCAL)], None)
        .await?;
    expect_status(&page, 200, "reset form")?;
    expect_header(&page, "referrer-policy", "no-referrer")?;
    let token = form_field(&page.body)?;
    if client.jar.get("fexu_reset_csrf") != Some(token.as_str()) {
        return Err("reset csrf cookie and field differ".into());
    }
    let accepted = client
        .call(
            "POST",
            "/auth/password-reset",
            &[
                ("origin", LOCAL),
                ("content-type", "application/x-www-form-urlencoded"),
            ],
            Some(&encode(&[
                ("csrf_token", &token),
                ("email", "ada@example.com"),
                ("locale", "en"),
            ])),
        )
        .await?;
    expect_status(&accepted, 200, "reset request")?;
    let mut only_reset = Client::new(*address);
    only_reset.jar.insert("fexu_reset_csrf", token);
    let probe = only_reset
        .call(
            "POST",
            "/probe",
            &[
                ("origin", LOCAL),
                ("content-type", "application/json"),
                ("x-csrf-token", "not-a-session-proof"),
            ],
            Some("{}"),
        )
        .await?;
    expect_status(&probe, 403, "reset cookie does not authorize other writes")?;
    let foreign = Client::new(*address);
    let mut foreign = foreign;
    let page = foreign
        .call("GET", "/auth/password-reset", &[("origin", LOCAL)], None)
        .await?;
    let token = form_field(&page.body)?;
    let rejected = foreign
        .call(
            "POST",
            "/auth/password-reset",
            &[
                ("origin", FOREIGN),
                ("content-type", "application/x-www-form-urlencoded"),
            ],
            Some(&encode(&[
                ("csrf_token", &token),
                ("email", "ada@example.com"),
            ])),
        )
        .await?;
    expect_status(&rejected, 403, "reset foreign origin")?;
    Ok(())
}

async fn dedicated_cookies_do_not_authorize_other_writes(
    address: &SocketAddr,
    password: &str,
) -> Result<(), String> {
    let mut client = Client::new(*address);
    client.call("GET", "/admin/login", &[], None).await?;
    let login = issued_token(&client)?;
    for method in ["POST", "PUT", "PATCH", "DELETE"] {
        let response = client
            .call(
                method,
                "/probe",
                &[
                    ("origin", LOCAL),
                    ("content-type", "application/json"),
                    ("x-csrf-token", &login),
                ],
                Some("{}"),
            )
            .await?;
        expect_status(&response, 403, method)?;
        if response.body == "written" {
            return Err(format!("{method} /probe accepted the login cookie"));
        }
    }
    let form = login_body(&login, password);
    for path in [
        "/admin/login/",
        "/admin//login",
        "/admin%2flogin",
        "/admin/login%2f",
        "/admin/%6cogin",
        "/admin/login%2e%2e/probe",
    ] {
        let response = client
            .call(
                "POST",
                path,
                &[
                    ("origin", LOCAL),
                    ("content-type", "application/x-www-form-urlencoded"),
                ],
                Some(&form),
            )
            .await?;
        expect_status(&response, 403, path)?;
        if sets_live_session(&response) || response.body == "written" {
            return Err(format!("{path} authorized a write"));
        }
    }
    let reset_with_login = client
        .call(
            "POST",
            "/auth/password-reset",
            &[
                ("origin", LOCAL),
                ("content-type", "application/x-www-form-urlencoded"),
            ],
            Some(&encode(&[
                ("csrf_token", &login),
                ("email", "ada@example.com"),
            ])),
        )
        .await?;
    expect_status(&reset_with_login, 403, "login cookie on the reset route")?;

    let mut reset_only = Client::new(*address);
    let page = reset_only
        .call("GET", "/auth/password-reset", &[("origin", LOCAL)], None)
        .await?;
    let reset = form_field(&page.body)?;
    reset_only.jar.clear();
    reset_only.jar.insert("fexu_reset_csrf", reset.clone());
    let login_with_reset = reset_only
        .post_login(Some(LOCAL), None, &login_body(&reset, password), &[])
        .await?;
    expect_status(&login_with_reset, 403, "reset cookie on the login route")?;
    if sets_live_session(&login_with_reset) {
        return Err("reset cookie issued a session".into());
    }
    let choose = reset_only
        .call(
            "POST",
            "/auth/password-reset/choose",
            &[
                ("origin", LOCAL),
                ("content-type", "application/x-www-form-urlencoded"),
            ],
            Some(&encode(&[
                ("csrf_token", &reset),
                ("password", password),
                ("password_confirm", password),
            ])),
        )
        .await?;
    if choose.status == 403 {
        return Err("the reset route rejected its own cookie".into());
    }
    if sets_live_session(&choose) {
        return Err("reset choose issued a session".into());
    }

    let mut foreign = Client::new(*address);
    foreign.call("GET", "/admin/login", &[], None).await?;
    let token = issued_token(&foreign)?;
    let spoofed = foreign
        .post_login(
            Some(FOREIGN),
            Some("same-origin"),
            &login_body(&token, password),
            &[],
        )
        .await?;
    expect_status(
        &spoofed,
        403,
        "foreign origin with same-origin fetch metadata",
    )?;
    if sets_live_session(&spoofed) {
        return Err("spoofed fetch metadata issued a session".into());
    }
    let unverified = foreign
        .post_login(Some("null"), None, &login_body(&token, password), &[])
        .await?;
    expect_status(&unverified, 403, "null origin without fetch metadata")?;
    if sets_live_session(&unverified) {
        return Err("unverified null origin issued a session".into());
    }
    Ok(())
}

async fn recovery_errors_keep_no_referrer(db: DatabaseConnection) -> Result<(), String> {
    let policy = Standard {
        referrer_policy: ReferrerPolicy::SameOrigin,
        ..Standard::default()
    };
    let address = listen(apply(
        App::new(db.clone())
            .with_auth(AuthService::new(db.clone(), development_config()))
            .into_router(),
        policy.clone(),
    ))
    .await;
    let mut client = Client::new(address);
    let form = client
        .call("GET", "/auth/password-reset", &[], None)
        .await?;
    expect_status(&form, 200, "recovery form under same-origin default")?;
    expect_header(&form, "referrer-policy", "no-referrer")?;
    expect_one(&form, "referrer-policy")?;
    let forbidden = client
        .call(
            "POST",
            "/auth/password-reset",
            &[
                ("origin", LOCAL),
                ("content-type", "application/x-www-form-urlencoded"),
            ],
            Some("csrf_token=wrong&email=ada@example.com"),
        )
        .await?;
    expect_status(&forbidden, 403, "recovery forbidden")?;
    if !forbidden
        .body
        .contains("Das Formular konnte nicht bestätigt werden.")
    {
        return Err(format!(
            "recovery forbidden page missing: {}",
            forbidden.body.chars().take(180).collect::<String>()
        ));
    }
    expect_header(&forbidden, "referrer-policy", "no-referrer")?;
    expect_one(&forbidden, "referrer-policy")?;
    expect_header(&forbidden, "x-frame-options", "DENY")?;
    expect_header(&forbidden, "x-content-type-options", "nosniff")?;

    let missing_address = listen(apply(App::new(db).into_router(), policy)).await;
    let mut missing = Client::new(missing_address);
    let page = missing
        .call("GET", "/auth/password-reset/choose", &[], None)
        .await?;
    expect_status(&page, 404, "missing auth")?;
    if !page
        .body
        .contains("Authentifizierung ist nicht konfiguriert.")
    {
        return Err(format!(
            "missing auth page missing: {}",
            page.body.chars().take(180).collect::<String>()
        ));
    }
    expect_header(&page, "referrer-policy", "no-referrer")?;
    expect_one(&page, "referrer-policy")?;
    if page.head.to_ascii_lowercase().contains("same-origin") {
        return Err(format!(
            "same-origin replaced recovery no-referrer: {}",
            page.head
        ));
    }
    Ok(())
}

async fn https_configuration(address: &SocketAddr, password: &str) -> Result<(), String> {
    let mut client = Client::new(*address);
    let page = client.call("GET", "/admin/login", &[], None).await?;
    expect_status(&page, 200, "https login document")?;
    expect_header(&page, "referrer-policy", "same-origin")?;
    let cookie_line = set_cookie_line(&page.head, "fexu_login_csrf")?;
    if !cookie_line.to_ascii_lowercase().contains("secure") {
        return Err(format!("login cookie is not Secure: {cookie_line}"));
    }
    let token = issued_token(&client)?;
    rejected(
        &mut client,
        None,
        None,
        &token,
        password,
        "https missing origin",
    )
    .await?;
    rejected(
        &mut client,
        Some("http://shop.example"),
        None,
        &token,
        password,
        "https rejects http origin",
    )
    .await?;
    rejected(
        &mut client,
        Some(LOCAL),
        None,
        &token,
        password,
        "https rejects loopback http",
    )
    .await?;
    rejected(
        &mut client,
        Some("null"),
        None,
        &token,
        password,
        "https null without fetch metadata",
    )
    .await?;
    rejected(
        &mut client,
        Some("null"),
        Some("cross-site"),
        &token,
        password,
        "https null cross-site",
    )
    .await?;
    let host_only = client
        .post_login(
            None,
            None,
            &login_body(&token, password),
            &[("host", "shop.example")],
        )
        .await?;
    expect_status(&host_only, 403, "https host header is not an origin")?;
    succeed(
        &mut client,
        Some("null"),
        Some("same-origin"),
        &token,
        password,
        "https null same-origin",
    )
    .await?;
    client.jar.clear();
    client.call("GET", "/admin/login", &[], None).await?;
    let token = issued_token(&client)?;
    let referer = client
        .post_login(
            None,
            None,
            &login_body(&token, password),
            &[("referer", "https://shop.example/admin/login")],
        )
        .await?;
    expect_login(&referer, "https referer")?;
    client.store(&referer);
    client.jar.clear();
    client.call("GET", "/admin/login", &[], None).await?;
    let token = issued_token(&client)?;
    succeed(
        &mut client,
        Some(HTTPS),
        None,
        &token,
        password,
        "https allowed origin",
    )
    .await?;

    let mut anonymous = Client::new(*address);
    let created = anonymous
        .call("GET", "/auth/anonymous", &[("origin", HTTPS)], None)
        .await?;
    expect_status(&created, 200, "https anonymous")?;
    let session_cookie = set_cookie_line(&created.head, "fexu_anonymous")?;
    if !session_cookie.to_ascii_lowercase().contains("secure") {
        return Err("anonymous cookie is not Secure".into());
    }
    let csrf = json_token(&created.body)?;
    let missing = anonymous
        .call(
            "POST",
            "/probe",
            &[
                ("content-type", "application/json"),
                ("x-csrf-token", &csrf),
            ],
            Some("{}"),
        )
        .await?;
    expect_status(&missing, 403, "https write without origin")?;
    let written = anonymous
        .call(
            "POST",
            "/probe",
            &[
                ("origin", HTTPS),
                ("content-type", "application/json"),
                ("x-csrf-token", &csrf),
            ],
            Some("{}"),
        )
        .await?;
    expect_status(&written, 200, "https anonymous write")?;
    Ok(())
}

async fn loopback_bind_is_not_an_origin(
    address: &SocketAddr,
    password: &str,
) -> Result<(), String> {
    let mut client = Client::new(*address);
    client.call("GET", "/admin/login", &[], None).await?;
    let token = issued_token(&client)?;
    rejected(
        &mut client,
        Some(LAN),
        None,
        &token,
        password,
        "wildcard bind does not allow the lan origin",
    )
    .await?;
    let host = client
        .post_login(
            Some(LAN),
            None,
            &login_body(&token, password),
            &[("host", "192.168.178.49:3000")],
        )
        .await?;
    expect_status(&host, 403, "lan host does not become an origin")?;
    let wildcard = client
        .post_login(
            Some("http://0.0.0.0:3000"),
            None,
            &login_body(&token, password),
            &[("host", "0.0.0.0:3000")],
        )
        .await?;
    expect_status(&wildcard, 403, "wildcard host is not an origin")?;
    succeed(
        &mut client,
        Some(LOCAL),
        None,
        &token,
        password,
        "loopback origin after wildcard bind",
    )
    .await?;
    client.jar.clear();
    client.call("GET", "/admin/login", &[], None).await?;
    let token = issued_token(&client)?;
    succeed(
        &mut client,
        Some(LOCALHOST),
        None,
        &token,
        password,
        "localhost origin after wildcard bind",
    )
    .await?;
    Ok(())
}

async fn succeed(
    client: &mut Client,
    origin: Option<&str>,
    fetch_site: Option<&str>,
    token: &str,
    password: &str,
    label: &str,
) -> Result<(), String> {
    let response = client
        .post_login(origin, fetch_site, &login_body(token, password), &[])
        .await?;
    expect_login(&response, label)?;
    client.store(&response);
    let admin = client
        .call(
            "GET",
            "/admin",
            &[("origin", origin.unwrap_or(LOCAL))],
            None,
        )
        .await?;
    expect_status(&admin, 200, label)?;
    Ok(())
}

async fn rejected(
    client: &mut Client,
    origin: Option<&str>,
    fetch_site: Option<&str>,
    token: &str,
    password: &str,
    label: &str,
) -> Result<(), String> {
    let token = match token {
        "missing" => String::new(),
        "mismatch" => "a".repeat(64),
        other => other.to_owned(),
    };
    let body = if token.is_empty() {
        encode(&[
            ("email", "ada@example.com"),
            ("password", password),
            ("next", "/admin"),
        ])
    } else {
        login_body(&token, password)
    };
    let response = client.post_login(origin, fetch_site, &body, &[]).await?;
    expect_status(&response, 403, label)?;
    if sets_live_session(&response) {
        return Err(format!("{label} issued a session"));
    }
    client.store(&response);
    Ok(())
}

fn sets_live_session(response: &RawResponse) -> bool {
    response.head.lines().any(|line| {
        let Some((key, value)) = line.split_once(':') else {
            return false;
        };
        key.eq_ignore_ascii_case("set-cookie")
            && value.trim().starts_with("fexu_session=")
            && !value.to_ascii_lowercase().contains("max-age=0")
    })
}

fn issued_token(client: &Client) -> Result<String, String> {
    client
        .jar
        .get("fexu_login_csrf")
        .map(str::to_owned)
        .ok_or_else(|| "fexu_login_csrf missing".into())
}

fn login_body(token: &str, password: &str) -> String {
    encode(&[
        ("csrf_token", token),
        ("email", "ada@example.com"),
        ("password", password),
        ("next", "/admin"),
    ])
}

async fn expire(db: &DatabaseConnection, table: &str) -> Result<(), String> {
    if table != "_fexu_sessions" && table != "_fexu_anonymous_sessions" {
        return Err("unexpected session table".into());
    }
    db.execute(Statement::from_string(
        db.get_database_backend(),
        format!("UPDATE {table} SET expires_at = 1, last_seen_at = 0"),
    ))
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn expect_login(response: &RawResponse, label: &str) -> Result<(), String> {
    expect_status(response, 303, label)?;
    let location = header_value(&response.head, "location").unwrap_or_default();
    if location != "/admin" {
        return Err(format!("{label} redirected to {location}"));
    }
    if !response.head.to_ascii_lowercase().contains("fexu_session=") {
        return Err(format!("{label} did not set a session cookie"));
    }
    Ok(())
}

fn expect_status(response: &RawResponse, status: u16, label: &str) -> Result<(), String> {
    if response.status != status {
        return Err(format!(
            "{label}: status {} body {}",
            response.status,
            response.body.chars().take(240).collect::<String>()
        ));
    }
    Ok(())
}

fn expect_header(response: &RawResponse, name: &str, value: &str) -> Result<(), String> {
    match header_value(&response.head, name) {
        Some(found) if found.eq_ignore_ascii_case(value) => Ok(()),
        Some(found) => Err(format!(
            "{name} is {found}, expected {value}; {}",
            response.head
        )),
        None => Err(format!("{name} missing; {}", response.head)),
    }
}

fn expect_one(response: &RawResponse, name: &str) -> Result<(), String> {
    let count = response
        .head
        .lines()
        .filter(|line| {
            line.split_once(':')
                .is_some_and(|(key, _)| key.eq_ignore_ascii_case(name))
        })
        .count();
    if count == 1 {
        Ok(())
    } else {
        Err(format!("{name} appears {count} times"))
    }
}

fn header_value(head: &str, name: &str) -> Option<String> {
    head.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name)
            .then(|| value.trim().to_owned())
    })
}

fn set_cookie_line(head: &str, name: &str) -> Result<String, String> {
    head.lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            if !key.eq_ignore_ascii_case("set-cookie") {
                return None;
            }
            let value = value.trim();
            value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .starts_with(&format!("{name}="))
                .then(|| value.to_owned())
        })
        .ok_or_else(|| format!("{name} set-cookie missing"))
}

fn form_field(body: &str) -> Result<String, String> {
    let marker = "name=\"csrf_token\" value=\"";
    let start = body
        .find(marker)
        .ok_or("csrf field missing")?
        .saturating_add(marker.len());
    let token = body[start..].split('"').next().unwrap_or("").to_owned();
    if token.is_empty() || token.len() > 256 {
        return Err(format!("csrf field length {}", token.len()));
    }
    Ok(token)
}

fn form_token(body: &str) -> Result<String, String> {
    let token = form_field(body)?;
    if token.len() != 64 || !token.chars().all(|char| char.is_ascii_hexdigit()) {
        return Err(format!(
            "csrf field is not 64 hex characters ({})",
            token.len()
        ));
    }
    Ok(token)
}

fn json_token(body: &str) -> Result<String, String> {
    let marker = "\"csrf_token\":\"";
    let start = body
        .find(marker)
        .ok_or("json csrf missing")?
        .saturating_add(marker.len());
    Ok(body[start..].split('"').next().unwrap_or("").to_owned())
}

fn encode(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| format!("{}={}", encode_component(key), encode_component(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn encode_component(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn fixture_password() -> String {
    String::from_utf8(vec![
        99, 111, 114, 114, 101, 99, 116, 45, 104, 111, 114, 115, 101, 45, 98, 97, 116, 116, 101,
        114, 121,
    ])
    .expect("fixture password")
}

struct RawResponse {
    status: u16,
    head: String,
    body: String,
}

struct Client {
    address: SocketAddr,
    jar: Jar,
}

impl Client {
    fn new(address: SocketAddr) -> Self {
        Self {
            address,
            jar: Jar::default(),
        }
    }

    async fn call(
        &mut self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> Result<RawResponse, String> {
        let response = exchange(self.address, self.request(method, path, headers, body)).await?;
        self.store(&response);
        Ok(response)
    }

    async fn post_login(
        &mut self,
        origin: Option<&str>,
        fetch_site: Option<&str>,
        body: &str,
        extra: &[(&str, &str)],
    ) -> Result<RawResponse, String> {
        let mut headers = vec![
            ("content-type", "application/x-www-form-urlencoded"),
            ("x-forwarded-host", "https://evil.example"),
            ("x-forwarded-proto", "https"),
            ("x-forwarded-for", "203.0.113.5"),
        ];
        if let Some(origin) = origin {
            headers.push(("origin", origin));
        }
        if let Some(fetch_site) = fetch_site {
            headers.push(("sec-fetch-site", fetch_site));
        }
        headers.extend_from_slice(extra);
        exchange(
            self.address,
            self.request("POST", "/admin/login", &headers, Some(body)),
        )
        .await
    }

    fn store(&mut self, response: &RawResponse) {
        self.jar.store(&response.head);
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> String {
        let mut out = format!("{method} {path} HTTP/1.1\r\n");
        let mut host = "evil.example";
        for (name, _) in headers {
            if name.eq_ignore_ascii_case("host") {
                host = headers
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case("host"))
                    .map(|(_, value)| *value)
                    .unwrap_or(host);
            }
        }
        out.push_str(&format!("Host: {host}\r\nConnection: close\r\n"));
        let cookie = self.jar.header();
        if !cookie.is_empty() {
            out.push_str(&format!("Cookie: {cookie}\r\n"));
        }
        for (name, value) in headers {
            if name.eq_ignore_ascii_case("host") {
                continue;
            }
            out.push_str(&format!("{name}: {value}\r\n"));
        }
        if let Some(body) = body {
            out.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
        } else {
            out.push_str("\r\n");
        }
        out
    }
}

#[derive(Default)]
struct Jar(BTreeMap<String, String>);

impl Jar {
    fn store(&mut self, head: &str) {
        for line in head.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            if !key.eq_ignore_ascii_case("set-cookie") {
                continue;
            }
            let value = value.trim();
            let Some((name, token)) = value.split(';').next().unwrap_or("").trim().split_once('=')
            else {
                continue;
            };
            if value.to_ascii_lowercase().contains("max-age=0") || token.is_empty() {
                self.0.remove(name);
            } else {
                self.0.insert(name.to_owned(), token.to_owned());
            }
        }
    }

    fn header(&self) -> String {
        self.0
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    fn insert(&mut self, name: &str, value: String) {
        self.0.insert(name.to_owned(), value);
    }

    fn contains(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    fn clear(&mut self) {
        self.0.clear();
    }
}

async fn exchange(address: SocketAddr, request: String) -> Result<RawResponse, String> {
    tokio::task::spawn_blocking(move || exchange_blocking(address, &request))
        .await
        .map_err(|error| error.to_string())?
}

fn exchange_blocking(address: SocketAddr, request: &str) -> Result<RawResponse, String> {
    let mut last = String::from("connection was not accepted");
    for _ in 0..20 {
        match TcpStream::connect_timeout(&address, Duration::from_millis(200)) {
            Ok(stream) => return read_response(stream, request),
            Err(error) => {
                last = error.to_string();
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
    Err(last)
}

fn read_response(mut stream: TcpStream, request: &str) -> Result<RawResponse, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| error.to_string())?;
    stream
        .write_all(request.as_bytes())
        .map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => bytes.extend_from_slice(&buffer[..count]),
            Err(error)
                if error.kind() == std::io::ErrorKind::TimedOut
                    || error.kind() == std::io::ErrorKind::WouldBlock =>
            {
                if bytes.is_empty() {
                    let line = request.lines().next().unwrap_or("request");
                    return Err(format!("response timed out for {line}"));
                }
                break;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return Err(format!("incomplete response: {text}"));
    };
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse().ok())
        .ok_or_else(|| format!("status missing: {head}"))?;
    Ok(RawResponse {
        status,
        head: head.to_owned(),
        body: body.to_owned(),
    })
}
