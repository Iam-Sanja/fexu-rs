//! OpenAPI document, in-process client, and isolated database helper.

use axum::http::StatusCode;
use fexu::auth::{AuthConfig, AuthService};
use fexu::db::QueryCounter;
use fexu::prelude::*;
use fexu::testing::{assert_query_budget, csrf_token_from_html, TestClient, TestDatabase};
use fexu::{DocsAccess, FexuChoice, OpenApiOperation, OpenApiOptions, OpenApiUi};

#[derive(Clone, Copy, Debug, PartialEq, Eq, FexuChoice)]
enum OfferStatus {
    #[choice(value = "draft", de = "Entwurf", en = "Draft")]
    Draft,
    #[choice(value = "listed", de = "Gelistet", en = "Listed")]
    Listed,
}

#[derive(Debug, fexu::FexuModel, fexu::serde::Serialize, fexu::serde::Deserialize)]
#[fexu(
    table = "openapi_offers",
    api = "/api/v1/offers",
    allow_anonymous = true,
    admin = true
)]
struct Offer {
    #[field(primary_key, auto)]
    id: Uuid,
    #[field(required, min_length = 2, max_length = 40)]
    #[admin(filterable, sortable, label = "Name")]
    name: String,
    #[field(min = 0, max = 1_000)]
    stock: i64,
    #[field(precision = 10, scale = 2, min = 0)]
    price: Decimal,
    #[field(choice)]
    status: OfferStatus,
    #[field(foreign_key = "categories.id", on_delete = "restrict")]
    category_id: Option<Uuid>,
    #[field(readonly)]
    sku: String,
}

#[tokio::test]
async fn document_describes_models_and_hides_unregistered_routes() {
    let db = fexu::db::connect("sqlite::memory:").await.unwrap();
    let app = fexu::App::new(db).register::<Offer>();
    app.bootstrap_schema().await.unwrap();
    let router = app.into_router();
    let mut client = TestClient::new(router);
    let document = client
        .get("/api/openapi.json")
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(document["openapi"], "3.1.0");
    assert!(document["paths"].get("/admin").is_none());
    assert!(document["paths"].get("/admin/api").is_none());
    assert!(document["paths"].get("/missing").is_none());
    let collection = &document["paths"]["/api/v1/offers"];
    assert!(collection.get("get").is_some());
    assert!(collection.get("post").is_some());
    assert!(collection.get("put").is_none());
    let detail = &document["paths"]["/api/v1/offers/{id}"];
    assert!(detail.get("patch").is_some());
    assert!(detail.get("delete").is_some());
    assert!(detail.get("put").is_none());
    let page = &document["components"]["schemas"]["OfferPage"];
    assert_eq!(page["properties"]["limit"]["maximum"], 100);
    assert!(page["properties"].get("has_more").is_some());
    assert!(page["properties"].get("truncated").is_none());
    let list = &document["paths"]["/api/v1/offers"]["get"]["parameters"];
    assert!(list
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "cursor"));
    let response = &document["components"]["schemas"]["Offer"]["properties"];
    assert_eq!(response["name"]["minLength"], 2);
    assert_eq!(response["name"]["maxLength"], 40);
    assert_eq!(response["name"]["x-fexu-filterable"], true);
    assert_eq!(response["stock"]["minimum"], 0);
    assert_eq!(response["stock"]["maximum"], 1000);
    assert_eq!(response["price"]["x-fexu-decimal"]["precision"], 10);
    assert_eq!(response["price"]["x-fexu-decimal"]["scale"], 2);
    assert_eq!(response["price"]["x-fexu-minimum"], 0);
    assert_eq!(response["status"]["enum"][0], "draft");
    assert_eq!(response["status"]["enum"][1], "listed");
    assert_eq!(
        response["category_id"]["x-fexu-relation"]["table"],
        "categories"
    );
    let create = &document["components"]["schemas"]["OfferCreate"];
    assert!(create["required"]
        .as_array()
        .unwrap()
        .iter()
        .any(|field| field == "sku"));
    let update = &document["components"]["schemas"]["OfferUpdate"]["properties"];
    assert!(update.get("sku").is_none());
    assert!(update.get("id").is_none());
    let csrf = collection["post"]["description"].as_str().unwrap();
    assert!(csrf.contains("x-csrf-token"));
    assert!(document["components"]["securitySchemes"]["csrfHeader"]["name"] == "x-csrf-token");
    assert!(document["paths"].get("/healthz").is_some());
    assert!(document["paths"].get("/auth/login").is_some());
    fexu::openapi::validate_document(&document).unwrap();
    let schemes = document["components"]["securitySchemes"]
        .as_object()
        .unwrap();
    assert_eq!(
        schemes.keys().cloned().collect::<Vec<_>>(),
        ["anonymousCookie", "csrfHeader", "sessionCookie"]
    );
    for scheme in schemes.values() {
        assert_eq!(scheme["type"], "apiKey");
    }

    let page = client
        .get("/api/docs")
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap();
    let html = page.text().unwrap();
    assert!(html.contains("/api/docs/swagger-ui-bundle.js"));
    assert!(html.contains("Apache License 2.0"));
    assert!(!html.to_ascii_lowercase().contains("unpkg"));
    assert!(!html.contains("<script>"));
    let initializer = client
        .get("/api/docs/swagger-initializer.js")
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap();
    let initializer = initializer.text().unwrap();
    assert!(initializer.contains("\"/api/openapi.json\""));
    assert!(initializer.contains("validatorUrl: null"));
    assert!(initializer.contains("tryItOutEnabled: false"));
    assert!(initializer.contains("supportedSubmitMethods: []"));
    assert!(initializer.contains("credentials = \"same-origin\""));
    assert!(initializer.contains("persistAuthorization: false"));
    assert!(!initializer.contains("headers[\"Authorization\"]"));
    let license = client
        .get("/api/docs/LICENSE")
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap();
    let license = license.text().unwrap();
    assert!(license.contains("Apache License"));

    let created = client
        .post_json(
            "/api/v1/offers",
            &serde_json::json!({
                "name": "Tasse",
                "stock": 2,
                "price": "4.50",
                "status": "listed",
                "sku": "TASSE"
            }),
        )
        .await
        .unwrap()
        .assert_status(StatusCode::CREATED)
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(created["price"], "4.50");
    client.set_cookie("fexu_anonymous", "not-a-session");
    let rejected = client
        .post_json(
            "/api/v1/offers",
            &serde_json::json!({
                "name": "Zweite",
                "stock": 1,
                "price": "1.00",
                "status": "draft",
                "sku": "ZWEI"
            }),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    let problem = rejected.json().unwrap();
    assert_eq!(problem["status"], 403);
    assert!(problem.get("code").is_some());

    let counter = QueryCounter::install();
    QueryCounter::reset();
    client.clear_cookies();
    client
        .get("/api/v1/offers")
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap();
    let listed = QueryCounter::get();
    assert_query_budget(listed, 2).unwrap();
    assert!(listed >= 1);
    for _ in 0..4 {
        client.get("/api/v1/offers").await.unwrap();
    }
    assert!(QueryCounter::get() > listed);
    drop(counter);
}

#[derive(Debug, fexu::FexuModel, fexu::serde::Serialize, fexu::serde::Deserialize)]
#[fexu(table = "openapi_notes", api = "/api/v1/notes", allow_anonymous = true)]
struct Note {
    #[field(primary_key, auto)]
    id: Uuid,
    #[field(json)]
    body: String,
}

#[tokio::test]
async fn openapi_describes_canonical_json_documents() {
    let db = fexu::db::connect("sqlite::memory:").await.unwrap();
    let app = fexu::App::new(db).register::<Note>();
    app.bootstrap_schema().await.unwrap();
    let router = app.into_router();
    let mut client = TestClient::new(router);
    let document = client
        .get("/api/openapi.json")
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap()
        .json()
        .unwrap();
    let body = &document["components"]["schemas"]["Note"]["properties"]["body"];
    assert_eq!(body["type"], "string");
    assert_eq!(body["contentMediaType"], "application/json");
    assert!(body["description"]
        .as_str()
        .unwrap()
        .contains("Canonical JSON text"));
    let created = client
        .post_json(
            "/api/v1/notes",
            &serde_json::json!({ "body": " { \"b\": 1, \"a\": null } " }),
        )
        .await
        .unwrap()
        .assert_status(StatusCode::CREATED)
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(created["body"], "{\"a\":null,\"b\":1}");
    let rejected = client
        .post_json("/api/v1/notes", &serde_json::json!({ "body": "{" }))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn unregistered_openapi_paths_are_rejected() {
    let db = fexu::db::connect("sqlite::memory:").await.unwrap();
    let app = fexu::App::new(db)
        .document(OpenApiOperation::get("/not-registered", "get_missing").summary("Missing"));
    let error = app.try_into_router().unwrap_err();
    assert_eq!(error.code, "openapi.path");
    assert!(error.detail.contains("not a registered route"));
}

#[test]
fn postgres_helper_refuses_a_foreign_database_name() {
    let refused = TestDatabase::postgres("postgres://fexu:secret@127.0.0.1:5432/fexu");
    assert!(refused.is_err());
    let accepted = TestDatabase::postgres("postgres://fexu:secret@127.0.0.1:5432/fexu_test");
    assert!(accepted.is_ok());
}

#[tokio::test]
async fn sqlite_helper_cleans_only_its_own_directory() {
    let mut database = TestDatabase::sqlite().await.unwrap();
    let url = database.url().to_owned();
    database.record_table("openapi_offers").unwrap();
    assert!(database.record_table("products;drop").is_err());
    database.cleanup().await.unwrap();
    let path = url
        .trim_start_matches("sqlite://")
        .trim_end_matches("?mode=rwc");
    assert!(!std::path::Path::new(path).exists());
}

#[test]
fn csrf_helper_reads_the_form_field() {
    let html = r#"<input type="hidden" name="csrf_token" value="abc-123">"#;
    assert_eq!(csrf_token_from_html(html).unwrap(), "abc-123");
}

#[test]
fn openapi_options_can_move_the_document() {
    let options = OpenApiOptions {
        json_path: "/shop/openapi.json".to_owned(),
        ui_path: None,
        ui: OpenApiUi::None,
        title: "Catalog".to_owned(),
        ..OpenApiOptions::default()
    };
    assert_eq!(options.json_path, "/shop/openapi.json");
    assert!(options.ui_path.is_none());
    assert_eq!(options.ui, OpenApiUi::None);
}

#[derive(Debug, fexu::FexuModel, fexu::serde::Serialize, fexu::serde::Deserialize)]
#[fexu(
    table = "openapi_private",
    api = "/api/v1/private",
    permissions = "offers"
)]
struct PrivateOffer {
    #[field(primary_key, auto)]
    id: Uuid,
    #[field(required, min_length = 1, max_length = 20)]
    name: String,
}

#[derive(Debug, fexu::FexuModel, fexu::serde::Serialize, fexu::serde::Deserialize)]
#[fexu(table = "openapi_closed", api = "/api/v1/closed")]
struct ClosedOffer {
    #[field(primary_key, auto)]
    id: Uuid,
    name: String,
}

#[derive(Debug, fexu::FexuModel, fexu::serde::Serialize, fexu::serde::Deserialize)]
#[fexu(
    table = "openapi_admin_leaks",
    api = "/admin/leaks",
    allow_anonymous = true
)]
struct AdminLeak {
    #[field(primary_key, auto)]
    id: Uuid,
    name: String,
}

async fn quoted() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({"ok": true}))
}

#[tokio::test]
async fn custom_routes_errors_and_admin_paths_follow_the_document() {
    let db = fexu::db::connect("sqlite::memory:").await.unwrap();
    let auth = AuthService::new(db.clone(), AuthConfig::development());
    let app = fexu::App::new(db)
        .with_auth(auth)
        .register::<Offer>()
        .register::<PrivateOffer>()
        .register::<ClosedOffer>()
        .register::<AdminLeak>()
        .urls(
            fexu::urls::UrlSet::new()
                .route("shop.quote", "/api/v1/quotes", axum::routing::get(quoted))
                .unwrap(),
        )
        .unwrap()
        .document(
            OpenApiOperation::get("/api/v1/quotes", "get_quote")
                .summary("Current quote")
                .tag("quotes")
                .description("Reads one quote. The browser still has to pass CSRF on writes.")
                .query("currency", serde_json::json!({"type": "string"}))
                .response(
                    200,
                    Some(serde_json::json!({
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["ok"],
                        "properties": { "ok": { "type": "boolean" } }
                    })),
                )
                .error_status(404),
        );
    app.bootstrap_schema().await.unwrap();
    let mut client = TestClient::new(app.into_router());
    let document = client
        .get("/api/openapi.json")
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap()
        .json()
        .unwrap();
    fexu::openapi::validate_document(&document).unwrap();
    assert!(document["paths"].get("/admin").is_none());
    assert!(document["paths"].get("/admin/leaks").is_none());
    assert!(document["paths"].get("/admin/api").is_none());
    let quote = &document["paths"]["/api/v1/quotes"]["get"];
    assert_eq!(quote["operationId"], "get_quote");
    assert_eq!(quote["parameters"][0]["name"], "currency");
    assert_eq!(quote["parameters"][0]["in"], "query");
    assert!(quote["responses"].get("200").is_some());
    assert!(quote["responses"].get("404").is_some());
    assert!(document["paths"]["/api/v1/private"]["get"]["responses"]
        .get("401")
        .is_some());
    assert!(document["paths"]["/api/v1/closed"]["get"]["responses"]
        .get("403")
        .is_some());
    let created = client
        .post_json(
            "/api/v1/offers",
            &serde_json::json!({
                "name": "Tasse",
                "stock": 1,
                "price": "2.00",
                "status": "draft",
                "sku": "TASSE"
            }),
        )
        .await
        .unwrap()
        .assert_status(StatusCode::CREATED)
        .unwrap()
        .json()
        .unwrap();
    let id = created["id"].as_str().unwrap();
    client
        .get(&format!("/api/v1/offers/{id}"))
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap();
    let missing = client
        .get("/api/v1/offers/00000000-0000-4000-8000-000000000099")
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(missing.json().unwrap()["code"], "resource.not_found");
    let private = client.get("/api/v1/private").await.unwrap();
    assert_eq!(private.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(private.json().unwrap()["status"], 401);
    let closed = client.get("/api/v1/closed").await.unwrap();
    assert_eq!(closed.status(), StatusCode::FORBIDDEN);
    assert_eq!(closed.json().unwrap()["status"], 403);
    let leaked = client
        .get("/admin/leaks")
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap()
        .json()
        .unwrap();
    assert!(leaked.get("data").is_some());
    client
        .get("/api/v1/quotes?currency=EUR")
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap();
}

#[tokio::test]
async fn documentation_can_be_disabled_or_moved_without_changing_the_api() {
    let db = fexu::db::connect("sqlite::memory:").await.unwrap();
    let hidden = fexu::App::new(db.clone())
        .register::<Offer>()
        .without_openapi();
    hidden.bootstrap_schema().await.unwrap();
    let mut client = TestClient::new(hidden.into_router());
    assert_eq!(
        client.get("/api/openapi.json").await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client.get("/api/docs").await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client
            .get("/api/docs/swagger-ui-bundle.js")
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    client
        .get("/api/v1/offers")
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap();

    let db = fexu::db::connect("sqlite::memory:").await.unwrap();
    let moved = fexu::App::new(db)
        .register::<Offer>()
        .openapi(OpenApiOptions {
            json_path: "/shop/openapi.json".to_owned(),
            ui_path: Some("/shop/docs".to_owned()),
            ui: OpenApiUi::Swagger,
            try_it_out: true,
            title: "Shop".to_owned(),
            ..OpenApiOptions::default()
        });
    moved.bootstrap_schema().await.unwrap();
    let mut client = TestClient::new(moved.into_router());
    assert_eq!(
        client.get("/api/openapi.json").await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client.get("/api/docs").await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    client
        .get("/shop/openapi.json")
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap();
    let script = client
        .get("/shop/docs/swagger-initializer.js")
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap();
    let script = script.text().unwrap();
    assert!(script.contains("tryItOutEnabled: true"));
    assert!(script.contains("\"/shop/openapi.json\""));
    assert!(!script.contains("validator.swagger.io"));
    client
        .get("/api/v1/offers")
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap();

    let db = fexu::db::connect("sqlite::memory:").await.unwrap();
    let schema_only = fexu::App::new(db)
        .register::<Offer>()
        .openapi(OpenApiOptions {
            ui: OpenApiUi::None,
            ui_path: None,
            ..OpenApiOptions::default()
        });
    schema_only.bootstrap_schema().await.unwrap();
    let mut client = TestClient::new(schema_only.into_router());
    client
        .get("/api/openapi.json")
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap();
    assert_eq!(
        client.get("/api/docs").await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn session_documentation_fails_closed() {
    let db = fexu::db::connect("sqlite::memory:").await.unwrap();
    let locked = fexu::App::new(db.clone())
        .with_auth(AuthService::new(db, AuthConfig::development()))
        .register::<Offer>()
        .openapi(OpenApiOptions {
            access: DocsAccess::Session,
            ..OpenApiOptions::default()
        });
    locked.bootstrap_schema().await.unwrap();
    let mut client = TestClient::new(locked.into_router());
    let schema = client.get("/api/openapi.json").await.unwrap();
    assert_eq!(schema.status(), StatusCode::UNAUTHORIZED);
    let page = client.get("/api/docs").await.unwrap();
    assert_eq!(page.status(), StatusCode::UNAUTHORIZED);
    let asset = client.get("/api/docs/swagger-ui-bundle.js").await.unwrap();
    assert_eq!(asset.status(), StatusCode::UNAUTHORIZED);
    client
        .get("/api/v1/offers")
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap();

    let db = fexu::db::connect("sqlite::memory:").await.unwrap();
    let unconfigured = fexu::App::new(db)
        .register::<Offer>()
        .openapi(OpenApiOptions {
            access: DocsAccess::Session,
            ..OpenApiOptions::default()
        });
    unconfigured.bootstrap_schema().await.unwrap();
    let mut client = TestClient::new(unconfigured.into_router());
    assert_eq!(
        client.get("/api/docs").await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    client
        .get("/api/v1/offers")
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap();
}
