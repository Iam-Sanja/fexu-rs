//! Generated list, read serializer, and OpenAPI 3.1 contract.

use axum::http::{Method, StatusCode};
use fexu::prelude::*;
use fexu::testing::TestClient;

#[derive(Debug, fexu::FexuModel, fexu::serde::Serialize, fexu::serde::Deserialize)]
#[fexu(
    table = "api_contract_items",
    api = "/api/v1/contract_items",
    allow_anonymous = true,
    admin = true
)]
struct ContractItem {
    #[field(primary_key, auto)]
    id: Uuid,
    #[field(required, min_length = 1, max_length = 40)]
    #[admin(filterable, sortable, label = "Name")]
    name: String,
    #[field(max_length = 80)]
    #[admin(filterable, label = "Note")]
    note: Option<String>,
    #[field(write_only, required, min_length = 8, max_length = 80)]
    #[admin(hidden)]
    secret: String,
}

async fn client_for(db: fexu::sea_orm::DatabaseConnection) -> TestClient {
    let app = fexu::App::new(db).register::<ContractItem>();
    app.bootstrap_schema().await.unwrap();
    TestClient::new(app.into_router())
}

async fn exercise(mut client: TestClient) {
    for name in ["Ada", "Bea", "Cara"] {
        let created = client
            .post_json(
                "/api/v1/contract_items",
                &serde_json::json!({"name": name, "secret": "supersecret"}),
            )
            .await
            .unwrap()
            .assert_status(StatusCode::CREATED)
            .unwrap()
            .json()
            .unwrap();
        assert!(created.get("secret").is_none(), "{created}");
        assert!(created["note"].is_null(), "{created}");
        assert!(created["id"].is_string());
    }

    let mut cursor: Option<String> = None;
    let mut seen = Vec::new();
    for _ in 0..4 {
        let path = match &cursor {
            Some(token) => format!(
                "/api/v1/contract_items?limit=1&sort=name&cursor={}",
                encode_query(token)
            ),
            None => "/api/v1/contract_items?limit=1&sort=name".to_owned(),
        };
        let page = client
            .get(&path)
            .await
            .unwrap()
            .assert_status(StatusCode::OK)
            .unwrap()
            .json()
            .unwrap();
        assert_eq!(page["limit"], 1);
        let rows = page["data"].as_array().unwrap();
        if rows.is_empty() {
            assert_eq!(page["has_more"], false);
            assert!(page["next"].is_null());
            break;
        }
        seen.push(rows[0]["name"].as_str().unwrap().to_owned());
        assert!(rows[0].get("secret").is_none());
        if page["has_more"] == true {
            cursor = Some(page["next"].as_str().unwrap().to_owned());
        } else {
            assert!(page["next"].is_null());
            break;
        }
    }
    assert_eq!(seen, ["Ada", "Bea", "Cara"]);

    let filtered = client
        .get("/api/v1/contract_items?filter=name:eq:Bea")
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(filtered["data"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["data"][0]["name"], "Bea");
    assert_eq!(filtered["has_more"], false);

    for index in 0..98 {
        client
            .post_json(
                "/api/v1/contract_items",
                &serde_json::json!({
                    "name": format!("row-{index:03}"),
                    "secret": "supersecret"
                }),
            )
            .await
            .unwrap()
            .assert_status(StatusCode::CREATED)
            .unwrap();
    }
    let mut remaining = 101_usize;
    let mut cursor: Option<String> = None;
    let mut pages = 0_usize;
    while remaining > 0 && pages < 4 {
        let path = match &cursor {
            Some(token) => format!("/api/v1/contract_items?cursor={}", encode_query(token)),
            None => "/api/v1/contract_items".to_owned(),
        };
        let page = client
            .get(&path)
            .await
            .unwrap()
            .assert_status(StatusCode::OK)
            .unwrap()
            .json()
            .unwrap();
        assert_eq!(page["limit"], 100);
        let rows = page["data"].as_array().unwrap();
        assert!(rows.len() <= 100);
        assert!(rows.iter().all(|row| row.get("secret").is_none()));
        remaining -= rows.len();
        pages += 1;
        if page["has_more"] == true {
            assert_eq!(rows.len(), 100);
            cursor = Some(page["next"].as_str().unwrap().to_owned());
        } else {
            assert!(page["next"].is_null());
            break;
        }
    }
    assert_eq!(remaining, 0);
    assert!(pages >= 2);

    let oversized = client
        .get("/api/v1/contract_items?limit=101")
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_problem(&oversized);
    assert_eq!(oversized.json().unwrap()["code"], "list.limit");

    let listed = client
        .get("/api/v1/contract_items?limit=1")
        .await
        .unwrap()
        .json()
        .unwrap();
    let id = listed["data"][0]["id"].as_str().unwrap();
    let rejected = client
        .patch_json(
            &format!("/api/v1/contract_items/{id}"),
            &serde_json::json!({"name": null}),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_problem(&rejected);
    let kept = client
        .patch_json(
            &format!("/api/v1/contract_items/{id}"),
            &serde_json::json!({"note": "kept"}),
        )
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(kept["note"], "kept");
    assert!(kept.get("secret").is_none());
    let cleared = client
        .patch_json(
            &format!("/api/v1/contract_items/{id}"),
            &serde_json::json!({"note": null}),
        )
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap()
        .json()
        .unwrap();
    assert!(cleared["note"].is_null());

    let missing_note = client
        .get("/api/v1/contract_items?filter=note:isnull&limit=1")
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap()
        .json()
        .unwrap();
    assert!(missing_note["data"][0]["note"].is_null());
    assert_eq!(missing_note["has_more"], true);

    let hidden_response = client
        .get("/api/v1/contract_items?filter=secret:eq:supersecret")
        .await
        .unwrap();
    assert_eq!(hidden_response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_problem(&hidden_response);
    let hidden = hidden_response.json().unwrap();
    assert_eq!(hidden["code"], "list.filter");
    assert_eq!(hidden["status"], 422);

    let unsorted = client
        .get("/api/v1/contract_items?sort=secret")
        .await
        .unwrap()
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY)
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(unsorted["code"], "list.sort");

    let bad_cursor = client
        .get("/api/v1/contract_items?cursor=nope")
        .await
        .unwrap()
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY)
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(bad_cursor["code"], "list.cursor");

    let broken_response = client
        .request(Method::POST, "/api/v1/contract_items")
        .header("content-type", "application/json")
        .unwrap()
        .bytes(b"{".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(broken_response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_problem(&broken_response);
    let broken = broken_response.json().unwrap();
    assert_eq!(broken["type"], "about:blank");
    assert_eq!(broken["code"], "validation.invalid_type");
    assert!(broken.get("title").is_some());

    let missing = client
        .get("/api/v1/contract_items/not-a-uuid")
        .await
        .unwrap()
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY)
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(missing["code"], "validation.uuid");

    let ready = client
        .get("/readyz")
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(ready["status"], "ready");

    let document = client
        .get("/api/openapi.json")
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(document["openapi"], "3.1.0");
    assert_eq!(document["info"]["version"], "1");
    let page_schema = &document["components"]["schemas"]["ContractItemPage"];
    let sample = client
        .get("/api/v1/contract_items?limit=1&sort=name")
        .await
        .unwrap()
        .json()
        .unwrap();
    fexu::openapi::matches_schema(&document, page_schema, &sample).unwrap();
    let item_schema = &document["components"]["schemas"]["ContractItem"];
    assert!(item_schema["properties"].get("secret").is_none());
    assert_eq!(
        item_schema["properties"]["note"]["type"],
        serde_json::json!(["string", "null"])
    );
    fexu::openapi::matches_schema(&document, item_schema, &sample["data"][0]).unwrap();
    fexu::openapi::matches_schema(&document, item_schema, &item_schema["example"]).unwrap();
    let create = &document["components"]["schemas"]["ContractItemCreate"];
    assert_eq!(create["properties"]["secret"]["writeOnly"], true);
    let request = serde_json::json!({"name": "Ada", "secret": "supersecret"});
    fexu::openapi::matches_schema(&document, create, &request).unwrap();
    fexu::openapi::matches_schema(&document, create, &create["example"]).unwrap();
    let update = &document["components"]["schemas"]["ContractItemUpdate"];
    assert_eq!(update["properties"]["name"]["type"], "string");
    assert_eq!(
        update["properties"]["note"]["type"],
        serde_json::json!(["string", "null"])
    );
    fexu::openapi::matches_schema(&document, update, &update["example"]).unwrap();
    let problem = &document["components"]["schemas"]["Problem"];
    fexu::openapi::matches_schema(&document, problem, &hidden).unwrap();
    fexu::openapi::matches_schema(&document, problem, &broken).unwrap();
    let parameters = document["paths"]["/api/v1/contract_items"]["get"]["parameters"]
        .as_array()
        .unwrap();
    assert!(parameters.iter().any(|item| item["name"] == "cursor"));
    assert!(parameters.iter().any(|item| item["name"] == "filter"));
    assert!(document["paths"].get("/readyz").is_some());
    assert!(document["paths"].get("/admin").is_none());
    fexu::openapi::validate_document(&document).unwrap();
    let missing = client
        .get("/api/v1/contract_items/00000000-0000-4000-8000-000000000099")
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(missing.json().unwrap()["status"], 404);
}

#[tokio::test]
async fn cursor_filters_and_openapi_describe_the_same_list() {
    let db = fexu::db::connect("sqlite::memory:").await.unwrap();
    exercise(client_for(db).await).await;
}

#[tokio::test]
#[ignore = "needs DATABASE_URL and creates only fexu_test_* databases"]
async fn postgres_cursor_filters_and_openapi_describe_the_same_list() {
    let admin = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let disposable = fexu::db::DisposablePostgres::create(&admin).await.unwrap();
    let url = disposable.url().to_owned();
    let db = fexu::db::connect(&url).await.unwrap();
    exercise(client_for(db).await).await;
    disposable.drop_database().await.unwrap();
}

fn assert_problem(response: &fexu::testing::TestResponse) {
    assert_eq!(
        response.header("content-type").unwrap(),
        Some("application/problem+json")
    );
}

fn encode_query(value: &str) -> String {
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
