//! The same admin-login contract on SQLite and on one disposable PostgreSQL database.
//!
//! The PostgreSQL case is ignored unless `DATABASE_URL` names an approved admin database.
//! An ignored run is not a pass. CI invokes it with `--exact` and requires `1 passed`.

use axum::http::{Method, StatusCode};
use fexu::auth::{apply_migrations, AuthConfig, AuthService};
use fexu::db::driver::DatabaseConnection;
use fexu::middleware::{apply, ReferrerPolicy, Standard};
use fexu::testing::{csrf_token_from_html, TestClient, TestResponse};
use fexu::App;

const LOCAL: &str = "http://127.0.0.1:3000";
const FOREIGN: &str = "http://evil.example";

async fn exercise(db: DatabaseConnection, password: &str) -> Result<(), String> {
    apply_migrations(&db)
        .await
        .map_err(|error| error.to_string())?;
    let auth = AuthService::new(db.clone(), AuthConfig::development());
    auth.create_user("ada@example.com", password, true, false)
        .await
        .map_err(|error| error.to_string())?;
    let router = apply(
        App::new(db)
            .with_auth(auth)
            .with_admin(fexu::admin::AdminRegistry::new())
            .require_browser_writes()
            .into_router(),
        Standard {
            referrer_policy: ReferrerPolicy::SameOrigin,
            ..Standard::default()
        },
    );

    let mut fresh = TestClient::new(router.clone());
    let blocked = fresh
        .request(Method::POST, "/admin/logout")
        .header("origin", LOCAL)?
        .header("content-type", "application/x-www-form-urlencoded")?
        .bytes(encode(&[("csrf_token", "missing")]).into_bytes())
        .send()
        .await?;
    if blocked.status() != StatusCode::FORBIDDEN {
        return Err(format!("cookieless logout returned {}", blocked.status()));
    }
    if fresh.cookie("fexu_session").is_some() {
        return Err("cookieless logout issued a session".into());
    }

    let mut client = TestClient::new(router);
    let page = client
        .request(Method::GET, "/admin/login")
        .header("origin", LOCAL)?
        .send()
        .await?;
    if page.status() != StatusCode::OK {
        return Err(format!("login document returned {}", page.status()));
    }
    expect_one(&page, "referrer-policy", "same-origin")?;
    expect_one(&page, "x-frame-options", "DENY")?;
    expect_one(&page, "x-content-type-options", "nosniff")?;
    let html = page.text()?.to_owned();
    let token = csrf_token_from_html(&html)?;
    if client.cookie("fexu_login_csrf") != Some(token.as_str()) {
        return Err("login form token and cookie differ".into());
    }

    let missing = post_login(&mut client, LOCAL, "", password).await?;
    expect_status(&missing, StatusCode::FORBIDDEN, "missing token")?;
    let mismatch = post_login(&mut client, LOCAL, &"ab".repeat(32), password).await?;
    expect_status(&mismatch, StatusCode::FORBIDDEN, "mismatched token")?;
    let foreign = post_login(&mut client, FOREIGN, &token, password).await?;
    expect_status(&foreign, StatusCode::FORBIDDEN, "foreign origin")?;
    if client.cookie("fexu_session").is_some() {
        return Err("rejected login issued a session".into());
    }

    let wrong = post_login(&mut client, LOCAL, &token, "not-the-password").await?;
    expect_status(&wrong, StatusCode::UNAUTHORIZED, "wrong password")?;
    if client.cookie("fexu_session").is_some() {
        return Err("wrong password issued a session".into());
    }

    let again = client
        .request(Method::GET, "/admin/login")
        .header("origin", LOCAL)?
        .send()
        .await?;
    expect_status(&again, StatusCode::OK, "login after rejection")?;
    let html = again.text()?.to_owned();
    let token = csrf_token_from_html(&html)?;
    let signed = post_login(&mut client, LOCAL, &token, password).await?;
    expect_status(&signed, StatusCode::SEE_OTHER, "fresh login")?;
    if signed.header("location")? != Some("/admin") {
        return Err(format!(
            "login location was {:?}",
            signed.header("location")?
        ));
    }
    if client.cookie("fexu_session").is_none() {
        return Err("login did not set a session cookie".into());
    }
    let admin = client
        .request(Method::GET, "/admin")
        .header("origin", LOCAL)?
        .send()
        .await?;
    expect_status(&admin, StatusCode::OK, "dashboard")?;
    Ok(())
}

fn expect_status(response: &TestResponse, status: StatusCode, label: &str) -> Result<(), String> {
    if response.status() == status {
        Ok(())
    } else {
        Err(format!(
            "{label}: status {} body {}",
            response.status(),
            response.text()?.chars().take(180).collect::<String>()
        ))
    }
}

fn expect_one(response: &TestResponse, name: &str, value: &str) -> Result<(), String> {
    let values = response.header_values(name)?;
    if values == [value] {
        Ok(())
    } else {
        Err(format!("{name} values were {values:?}, expected [{value}]"))
    }
}

async fn post_login(
    client: &mut TestClient,
    origin: &str,
    token: &str,
    password: &str,
) -> Result<TestResponse, String> {
    let mut fields = vec![
        ("email", "ada@example.com"),
        ("password", password),
        ("next", "/admin"),
    ];
    if !token.is_empty() {
        fields.push(("csrf_token", token));
    }
    client
        .request(Method::POST, "/admin/login")
        .header("origin", origin)?
        .header("content-type", "application/x-www-form-urlencoded")?
        .bytes(encode(&fields).into_bytes())
        .send()
        .await
}

fn encode(fields: &[(&str, &str)]) -> String {
    fields
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
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn fixture_password() -> String {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).expect("test password");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[tokio::test]
async fn sqlite_contract_login_matches_the_browser_form() {
    let db = fexu::db::connect("sqlite::memory:").await.expect("sqlite");
    exercise(db, &fixture_password())
        .await
        .expect("sqlite contract");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL, CREATEDB, and an approved PostgreSQL admin database"]
async fn postgres_contract_login_matches_sqlite() {
    let admin = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let disposable = fexu::db::DisposablePostgres::create(&admin)
        .await
        .expect("isolated postgres");
    let db = fexu::db::connect(disposable.url()).await.expect("postgres");
    let result = exercise(db, &fixture_password()).await;
    let dropped = disposable.drop_database().await;
    result.expect("postgres contract");
    dropped.expect("drop isolated database");
}
