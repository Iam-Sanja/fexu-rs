use axum::http::{header, StatusCode};
use fexu::auth::{AuthConfig, AuthService};
use fexu::testing::{csrf_token_from_html, TestClient};
use fexu::App;

use board::{prepare_database, BoardApp};

const PASSWORD: &str = "correct-horse-battery";

async fn exercise(db: fexu::db::driver::DatabaseConnection) {
    prepare_database(&db).await.expect("schema");
    let auth = AuthService::new(db.clone(), AuthConfig::development());
    auth.create_user("owner@example.com", PASSWORD, false, false)
        .await
        .expect("owner");
    auth.create_user("other@example.com", PASSWORD, false, false)
        .await
        .expect("other");
    auth.create_user("guest@example.com", PASSWORD, false, false)
        .await
        .expect("guest");
    for action in ["read", "create", "update", "delete"] {
        auth.grant_permission("editor", &format!("notes.{action}"))
            .await
            .expect("permission");
    }
    auth.assign_role("owner@example.com", "editor")
        .await
        .expect("owner role");
    auth.assign_role("other@example.com", "editor")
        .await
        .expect("other role");

    let app = App::new(db)
        .with_auth(auth.clone())
        .install(BoardApp)
        .expect("install")
        .seal()
        .expect("seal");
    let router = app.into_router();
    let mut owner = TestClient::new(router.clone());
    let mut other = TestClient::new(router.clone());
    let mut guest = TestClient::new(router.clone());
    let mut anonymous = TestClient::new(router);
    sign_in(&mut owner, &auth, "owner@example.com").await;
    sign_in(&mut other, &auth, "other@example.com").await;
    sign_in(&mut guest, &auth, "guest@example.com").await;

    let closed = anonymous
        .get("/board/items")
        .await
        .expect("anonymous")
        .assert_status(StatusCode::UNAUTHORIZED)
        .expect("401");
    assert!(closed.text().expect("body").len() < 4_000);

    guest
        .get("/board/items")
        .await
        .expect("guest")
        .assert_status(StatusCode::FORBIDDEN)
        .expect("403");

    owner
        .request(axum::http::Method::GET, "/board/items?scope=all")
        .header(header::ACCEPT.as_str(), "text/html")
        .expect("header")
        .send()
        .await
        .expect("query")
        .assert_status(StatusCode::BAD_REQUEST)
        .expect("bad query");

    let form = owner
        .request(axum::http::Method::GET, "/board/items/new")
        .header(header::ACCEPT.as_str(), "text/html")
        .expect("header")
        .header(header::ACCEPT_LANGUAGE.as_str(), "en")
        .expect("language")
        .send()
        .await
        .expect("form");
    assert_eq!(form.status(), StatusCode::OK);
    let html = form.text().expect("form html").to_owned();
    assert!(html.contains("name=\"csrf_token\""));
    assert!(html.contains("name=\"title\""));

    owner
        .post_form("/board/items/new", &[("title", "Missing token")])
        .await
        .expect("csrf")
        .assert_status(StatusCode::FORBIDDEN)
        .expect("missing csrf");

    owner
        .post_form_with_csrf("/board/items/new", &html, &[("title", ""), ("body", "")])
        .await
        .expect("invalid")
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY)
        .expect("422");
    let invalid = owner
        .post_form_with_csrf(
            "/board/items/new",
            &html,
            &[("title", ""), ("body", "<script>")],
        )
        .await
        .expect("invalid again");
    assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let invalid_body = invalid.text().expect("invalid html");
    assert!(invalid_body.contains("The form is invalid."));
    assert!(invalid_body.contains("&lt;script&gt;") || !invalid_body.contains("<script>"));
    assert!(!invalid_body.contains("<script>"));
    let fresh = owner
        .request(axum::http::Method::GET, "/board/items/new")
        .header(header::ACCEPT.as_str(), "text/html")
        .expect("header")
        .send()
        .await
        .expect("fresh form");
    assert!(!fresh
        .text()
        .expect("fresh")
        .contains("The form is invalid."));

    let created = owner
        .post_form_with_csrf(
            "/board/items/new",
            &html,
            &[
                ("title", "<script>alert(1)</script>"),
                ("body", "hello"),
                ("owner_id", "00000000-0000-0000-0000-000000000000"),
            ],
        )
        .await
        .expect("forged owner");
    assert_eq!(created.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let created = owner
        .post_form_with_csrf(
            "/board/items/new",
            &html,
            &[("title", "<script>alert(1)</script>"), ("body", "hello")],
        )
        .await
        .expect("create");
    assert_eq!(created.status(), StatusCode::SEE_OTHER);
    let location = created
        .header("location")
        .expect("location header")
        .expect("location")
        .to_owned();
    assert!(location.starts_with("/board/items/"), "{location}");
    assert!(!location.contains("://"), "{location}");

    let detail = owner
        .request(axum::http::Method::GET, &location)
        .header(header::ACCEPT.as_str(), "text/html")
        .expect("header")
        .send()
        .await
        .expect("detail");
    assert_eq!(detail.status(), StatusCode::OK);
    let detail_body = detail.text().expect("detail html");
    assert!(detail_body.contains("Saved"));
    assert!(detail_body.contains("&lt;script&gt;"));
    assert!(!detail_body.contains("<script>"));
    let again = owner
        .request(axum::http::Method::GET, &location)
        .header(header::ACCEPT.as_str(), "text/html")
        .expect("header")
        .send()
        .await
        .expect("detail again");
    assert!(!again.text().expect("again").contains("Saved"));

    other
        .request(axum::http::Method::GET, &location)
        .header(header::ACCEPT.as_str(), "text/html")
        .expect("header")
        .send()
        .await
        .expect("other detail")
        .assert_status(StatusCode::NOT_FOUND)
        .expect("hidden");
    let other_list = other
        .request(axum::http::Method::GET, "/board/items")
        .header(header::ACCEPT.as_str(), "text/html")
        .expect("header")
        .send()
        .await
        .expect("other list");
    assert_eq!(other_list.status(), StatusCode::OK);
    assert!(!other_list.text().expect("list").contains("alert"));

    let mut list_html = String::new();
    for index in 1..=3 {
        let page = owner
            .request(axum::http::Method::GET, "/board/items/new")
            .header(header::ACCEPT.as_str(), "text/html")
            .expect("header")
            .send()
            .await
            .expect("new");
        let body = page.text().expect("new html").to_owned();
        owner
            .post_form_with_csrf(
                "/board/items/new",
                &body,
                &[("title", &format!("Note {index}")), ("body", "x")],
            )
            .await
            .expect("extra")
            .assert_status(StatusCode::SEE_OTHER)
            .expect("saved");
        if index == 1 {
            list_html = body;
        }
    }
    let _ = list_html;
    let page = owner
        .request(axum::http::Method::GET, "/board/items?page_size=2")
        .header(header::ACCEPT.as_str(), "text/html")
        .expect("header")
        .send()
        .await
        .expect("page");
    assert_eq!(page.status(), StatusCode::OK);
    let page_body = page.text().expect("page html");
    assert!(
        page_body.contains("/board/items?page_size=2&amp;page=2")
            || page_body.contains("/board/items?page_size=2&page=2"),
        "{page_body}"
    );
    assert!(page_body.contains("rel=\"canonical\""));
    assert!(page_body.contains("http://127.0.0.1:3000/board/items"));
    let second = owner
        .get("/board/items?page_size=2&page=2")
        .await
        .expect("second");
    assert_eq!(second.status(), StatusCode::OK);
    assert!(second.text().expect("second html").contains("Note"));

    let robots = anonymous.get("/board/robots.txt").await.expect("robots");
    assert_eq!(robots.status(), StatusCode::OK);
    let robots_body = robots.text().expect("robots text");
    assert!(robots_body.contains("Disallow: /board/items/new"));
    assert!(!robots_body.contains("http://evil"));
    let etag = robots.header("etag").expect("etag header").expect("etag");
    let cached = anonymous
        .request(axum::http::Method::GET, "/board/robots.txt")
        .header(header::IF_NONE_MATCH.as_str(), etag)
        .expect("header")
        .send()
        .await
        .expect("cached");
    assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);
    assert!(cached.text().expect("empty").is_empty());

    let sitemap = anonymous.get("/board/sitemap.xml").await.expect("sitemap");
    assert_eq!(sitemap.status(), StatusCode::OK);
    let sitemap_body = sitemap.text().expect("sitemap xml");
    assert!(sitemap_body.contains("http://127.0.0.1:3000/board/items"));
    assert!(!sitemap_body.contains("<script"));

    let edit = detail_body
        .split("href=\"")
        .find(|part| part.contains("/edit\""))
        .expect("edit link")
        .split('"')
        .next()
        .expect("edit href")
        .replace("&amp;", "&");
    let edit_page = owner
        .request(axum::http::Method::GET, &edit)
        .header(header::ACCEPT.as_str(), "text/html")
        .expect("header")
        .header(header::ACCEPT_LANGUAGE.as_str(), "en")
        .expect("language")
        .send()
        .await
        .expect("edit");
    assert_eq!(edit_page.status(), StatusCode::OK);
    let edit_html = edit_page.text().expect("edit html").to_owned();
    let updated = owner
        .post_form_with_csrf(
            &edit,
            &edit_html,
            &[("title", "Renamed"), ("body", "hello")],
        )
        .await
        .expect("update");
    assert_eq!(updated.status(), StatusCode::SEE_OTHER);

    let delete = format!("{location}/delete");
    let confirm = owner
        .request(axum::http::Method::GET, &delete)
        .header(header::ACCEPT.as_str(), "text/html")
        .expect("header")
        .send()
        .await
        .expect("confirm");
    assert_eq!(confirm.status(), StatusCode::OK);
    let confirm_html = confirm.text().expect("confirm html").to_owned();
    assert!(confirm_html.contains("Delete"));
    owner
        .post_form_with_csrf(&delete, &confirm_html, &[])
        .await
        .expect("delete")
        .assert_status(StatusCode::SEE_OTHER)
        .expect("deleted");
    let gone = owner
        .request(axum::http::Method::GET, &location)
        .header(header::ACCEPT.as_str(), "text/html")
        .expect("header")
        .send()
        .await
        .expect("gone");
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
    let _ = csrf_token_from_html(&html).expect("token helper");
}

async fn sign_in(client: &mut TestClient, auth: &AuthService, email: &str) {
    let session = auth
        .login(email, PASSWORD, "board-test")
        .await
        .expect("login");
    client.set_cookie("fexu_session", &session.token);
}

#[test]
fn omit_must_name_a_real_column() {
    let mut resource = board::resource();
    resource.omit = &["owner"];
    let error = match fexu::views::crud_urls_with::<board::Note, board::NotePolicy, board::NoteStamp>(
        resource,
    ) {
        Err(error) => error,
        Ok(_) => panic!("unknown omit was accepted"),
    };
    assert_eq!(error.code, "crud.fields");
}

#[tokio::test]
async fn sqlite_board_covers_crud_permissions_csrf_and_pages() {
    let db = fexu::db::connect("sqlite::memory:").await.expect("sqlite");
    exercise(db).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL, CREATEDB, and an approved PostgreSQL test database"]
async fn postgres_board_covers_crud_permissions_csrf_and_pages() {
    let admin = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let disposable = fexu::db::DisposablePostgres::create(&admin)
        .await
        .expect("isolated postgres");
    let db = fexu::db::connect(disposable.url()).await.expect("postgres");
    exercise(db.clone()).await;
    db.close().await.expect("close");
    disposable.drop_database().await.expect("drop");
}
