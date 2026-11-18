use axum::http::{header, Method, StatusCode};
use fexu::auth::{AuthConfig, AuthService};
use fexu::db::driver::DatabaseConnection;
use fexu::db::ModelRepository;
use fexu::testing::{csrf_token_from_html, TestClient, TestResponse};
use fexu::App;

use board::{prepare_database, BoardApp, Note, NotePolicy};

fn fixture_password() -> String {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).expect("test password");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

async fn exercise(db: DatabaseConnection) {
    prepare_database(&db).await.expect("schema");
    let password = fixture_password();
    let auth = AuthService::new(db.clone(), AuthConfig::development());
    let owner = auth
        .create_user("owner@example.com", &password, false, false)
        .await
        .expect("owner");
    for action in ["read", "create", "update", "delete"] {
        auth.grant_permission("editor", &format!("notes.{action}"))
            .await
            .expect("permission");
    }
    auth.assign_role("owner@example.com", "editor")
        .await
        .expect("role");

    let app = App::new(db.clone())
        .with_auth(auth.clone())
        .register_policy::<Note, NotePolicy>()
        .install(BoardApp)
        .expect("install")
        .seal()
        .expect("seal");
    let mut client = TestClient::new(app.into_router());
    sign_in(&mut client, &auth, "owner@example.com", &password).await;

    let form = client
        .request(Method::GET, "/board/items/new")
        .header(header::ACCEPT.as_str(), "text/html")
        .expect("accept")
        .header(header::ACCEPT_LANGUAGE.as_str(), "en")
        .expect("language")
        .send()
        .await
        .expect("form");
    assert_eq!(form.status(), StatusCode::OK);
    let html = form.text().expect("form html").to_owned();
    assert!(html.contains("<form method=\"post\""));
    assert!(html.contains("name=\"title\""));
    assert!(html.contains("name=\"csrf_token\""));
    assert!(!html.contains("<script"));
    assert_eq!(stored(&db).await, 0);

    client
        .post_form(
            "/board/items/new",
            &[("title", "Missing token"), ("body", "")],
        )
        .await
        .expect("missing csrf")
        .assert_status(StatusCode::FORBIDDEN)
        .expect("403");
    assert_eq!(stored(&db).await, 0);

    let long = "ä".repeat(81);
    assert_eq!(long.chars().count(), 81);
    let english = post_fields(
        &mut client,
        &html,
        "en",
        &[("title", &long), ("body", "kept")],
    )
    .await;
    assert_eq!(english.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let english_body = english.text().expect("english html");
    assert!(
        english_body.contains("The text is too long."),
        "{english_body}"
    );
    assert!(english_body.contains(&long), "{english_body}");
    assert!(english_body.contains("kept"));
    assert!(!english_body.contains("<script"));
    assert_eq!(stored(&db).await, 0);

    let german = post_fields(
        &mut client,
        &html,
        "de",
        &[("title", &long), ("body", "behalten")],
    )
    .await;
    assert_eq!(german.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let german_body = german.text().expect("german html");
    assert!(
        german_body.contains("Der Text ist zu lang."),
        "{german_body}"
    );
    assert!(german_body.contains(&long), "{german_body}");
    assert_eq!(stored(&db).await, 0);

    let token = csrf_token_from_html(&html).expect("csrf");
    let json = client
        .request(Method::POST, "/board_notes")
        .header(header::CONTENT_TYPE.as_str(), "application/json")
        .expect("type")
        .header(header::ACCEPT.as_str(), "application/problem+json")
        .expect("accept")
        .header("x-csrf-token", &token)
        .expect("csrf header")
        .bytes(
            fexu::serde_json::json!({
                "title": long,
                "body": "kept",
                "owner_id": owner.id,
            })
            .to_string()
            .into_bytes(),
        )
        .send()
        .await
        .expect("json");
    assert_eq!(json.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let problem: fexu::serde_json::Value =
        fexu::serde_json::from_str(json.text().expect("problem")).expect("problem json");
    assert_eq!(problem["code"], "validation.max_length");
    assert_eq!(problem["field"], "title");
    assert_eq!(problem["params"]["max"], "80");
    assert_eq!(stored(&db).await, 0);

    let unknown_json = client
        .request(Method::POST, "/board_notes")
        .header(header::CONTENT_TYPE.as_str(), "application/json")
        .expect("type")
        .header("x-csrf-token", &token)
        .expect("csrf header")
        .bytes(
            fexu::serde_json::json!({
                "title": "Visible",
                "body": "kept",
                "owner_id": owner.id,
                "role": "admin",
            })
            .to_string()
            .into_bytes(),
        )
        .send()
        .await
        .expect("unknown json");
    assert_eq!(unknown_json.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let unknown_problem: fexu::serde_json::Value =
        fexu::serde_json::from_str(unknown_json.text().expect("unknown problem"))
            .expect("unknown problem json");
    assert_eq!(unknown_problem["code"], "validation.unknown_field");
    assert_eq!(stored(&db).await, 0);

    let duplicate = post_fields(
        &mut client,
        &html,
        "en",
        &[("title", "Alpha"), ("title", "Beta"), ("body", "kept")],
    )
    .await;
    assert_eq!(duplicate.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let duplicate_body = duplicate.text().expect("duplicate html");
    assert!(
        duplicate_body.contains("This field was submitted more than once."),
        "{duplicate_body}"
    );
    assert_eq!(stored(&db).await, 0);

    let unknown = post_fields(
        &mut client,
        &html,
        "en",
        &[("title", "Visible"), ("body", "kept"), ("role", "admin")],
    )
    .await;
    assert_eq!(unknown.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let unknown_body = unknown.text().expect("unknown html");
    assert!(
        unknown_body.contains("role: This field is not allowed."),
        "{unknown_body}"
    );
    assert!(unknown_body.contains("value=\"Visible\""));
    assert_eq!(stored(&db).await, 0);

    let forged = post_fields(
        &mut client,
        &html,
        "en",
        &[
            ("title", "Visible"),
            ("body", "kept"),
            ("owner_id", "00000000-0000-0000-0000-000000000000"),
        ],
    )
    .await;
    assert_eq!(forged.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let forged_body = forged.text().expect("forged html");
    assert!(
        forged_body.contains("owner_id: This field cannot be changed."),
        "{forged_body}"
    );
    assert!(forged_body.contains("value=\"Visible\""));
    assert_eq!(stored(&db).await, 0);
}

async fn post_fields(
    client: &mut TestClient,
    html: &str,
    language: &str,
    fields: &[(&str, &str)],
) -> TestResponse {
    let token = csrf_token_from_html(html).expect("csrf");
    let mut pairs = fields.to_vec();
    pairs.push(("csrf_token", token.as_str()));
    let body = pairs
        .iter()
        .map(|(name, value)| format!("{}={}", encode(name), encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    client
        .request(Method::POST, "/board/items/new")
        .header(
            header::CONTENT_TYPE.as_str(),
            "application/x-www-form-urlencoded",
        )
        .expect("type")
        .header(header::ACCEPT.as_str(), "text/html")
        .expect("accept")
        .header(header::ACCEPT_LANGUAGE.as_str(), language)
        .expect("language")
        .bytes(body.into_bytes())
        .send()
        .await
        .expect("post")
}

async fn stored(db: &DatabaseConnection) -> usize {
    Note::list(db).await.expect("notes").data.len()
}

async fn sign_in(client: &mut TestClient, auth: &AuthService, email: &str, password: &str) {
    let session = auth
        .login(email, password, "board-test")
        .await
        .expect("login");
    client.set_cookie("fexu_session", &session.token);
}

fn encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[tokio::test]
async fn sqlite_note_http_and_json_share_validation_errors() {
    let db = fexu::db::connect("sqlite::memory:").await.expect("sqlite");
    exercise(db).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL, CREATEDB, and an approved PostgreSQL test database"]
async fn postgres_note_http_and_json_share_validation_errors() {
    let admin = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let disposable = fexu::db::DisposablePostgres::create(&admin)
        .await
        .expect("isolated postgres");
    let db = fexu::db::connect(disposable.url()).await.expect("postgres");
    exercise(db.clone()).await;
    db.close().await.expect("close");
    disposable.drop_database().await.expect("drop");
}
