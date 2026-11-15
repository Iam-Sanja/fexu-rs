# OpenAPI 3.1 and Swagger UI

[Overview](index.md) · **Implemented** in `crates/fexu-api/src/openapi.rs`. The document is built from `ModelDescriptor`, `ColumnSpec`, `NumericBound`, `FieldContract` and `AdminFieldMeta`. It does not keep a second model description, and Swagger UI only renders that document.

`App` publishes `GET /api/openapi.json` and `GET /api/docs` unless documentation is turned off. Earlier builds used `GET /openapi.json` and `GET /docs`. Those paths are not redirected. Set `schema_path` and `path`, or `json_path` and `ui_path`, to keep them. Admin routes are not included, including a model whose API path starts with `/admin`. `App::document` accepts an operation only when its path is already registered.

Swagger UI 5.29.0 is compiled into the crate from `swagger-ui-dist` (Apache License 2.0). The license text is served at `{ui-path}/LICENSE`. The page, the script and the stylesheet come from the same process. The UI does not load a CDN, and it does not send the document to `validator.swagger.io`.

## Configuration

Programmatic options replace `[api.docs]` when both are set:

```rust
let app = fexu::App::new(db)
    .register::<Product>()
    .openapi(fexu::OpenApiOptions {
        enabled: true,
        schema_enabled: true,
        json_path: "/api/openapi.json".to_owned(),
        ui_path: Some("/api/docs".to_owned()),
        title: "Shop".to_owned(),
        ui: fexu::OpenApiUi::Swagger,
        try_it_out: false,
        access: fexu::DocsAccess::Public,
    });
let router = app.into_router();
```

`fexu.toml` uses the same switches. Names follow the other sections (`enabled`, a path, a boolean):

```toml
[api.docs]
enabled = true
ui = "swagger"          # swagger, explorer or none
path = "/api/docs"
schema_path = "/api/openapi.json"
schema = true
try_it_out = false
access = "public"       # public or session
```

An application that calls `with_settings` and does not call `openapi` or `without_openapi` uses this section. `App::new` without settings uses the same omitted-section rule: `FEXU_ENV=production` mounts no documentation routes, and any other environment mounts the defaults. `OpenApiOptions::default` does not read the environment. `openapi` and `without_openapi` win over both the file and `FEXU_ENV`.

| Situation | Result |
|---|---|
| No `[api.docs]` and no `openapi` call, not production | Schema and Swagger UI on, try-it-out off, access `public` |
| No `[api.docs]` and no `openapi` call, `FEXU_ENV=production` | No documentation routes. This includes `App::new` without `with_settings` |
| `openapi(...)` or `without_openapi()` | The explicit options are used, including in production |
| `enabled = false` | No documentation routes. Model, auth and health routes stay mounted |
| `ui = "none"` and `schema = true` | JSON only |
| `ui = "swagger"` or `explorer` | Requires `schema = true` |
| `access` omitted in production while docs are enabled | `session` |
| `access = "session"` | `fexu_session` required. The anonymous cookie is not enough. Without an auth service the routes answer 403 |

`try_it_out = false` is the default. Swagger then builds the page with `supportedSubmitMethods: []` and `tryItOutEnabled: false`. When it is true, the browser calls the real route on the same origin with `credentials: "same-origin"`. The call still passes the cookie, origin and CSRF checks. The UI does not add a bearer token and it does not proxy the request.

`ui = "explorer"` keeps the small same-origin page. It lists operations from the document. Its request form is omitted when try-it-out is off.

Paths must be absolute, must differ from each other, and must not start with `/admin`. A path that an application route already uses is rejected when the router is built.

## What the document contains

- Generated model routes: `GET` and `POST` on the collection, `GET`, `PATCH` and `DELETE` on `/{id}`. `PUT` is omitted because the generated handlers do not implement it.
- Create, update and response schemas. Readonly fields stay on create and leave the update schema. Primary keys and managed timestamps stay on the response.
- Required, nullable, decimal precision and scale, choice enums, `minLength`, `maxLength`, numeric `minimum` and `maximum`, and foreign-key relation metadata.
- The list response is `data`, `limit`, `next` and `has_more`. Query parameters are `limit` (1..=100, default 100), `cursor`, `sort` and repeated `filter`. Sort and filter names are the admin allow-list, plus `id` for sort. A `write_only` or secret-named field is absent from the response schema. `contains` is text only. Offset is not a parameter.
- Session cookie, anonymous cookie and `x-csrf-token`. Cookie writes say that Bearer authorization is not a session. Those are the only security schemes.
- Problem JSON for 401, 403, 404, 422 and 500 where those statuses are produced. 409 is not documented because the handlers do not return it.
- `/healthz`, `/readyz` and the mounted `/auth` routes.

## Custom operations

`App::document` adds `GET`, `POST`, `PUT`, `PATCH` or `DELETE` for a route the application registered itself. An unknown path, an `/admin` path, or a duplicated operation id fails when the router is built.

```rust
app.document(
    fexu::OpenApiOperation::get("/api/v1/quotes", "get_quote")
        .summary("Current quote")
        .tag("quotes")
        .description("Reads one quote.")
        .query("currency", serde_json::json!({"type": "string"}))
        .response(200, Some(serde_json::json!({"type": "object"})))
        .error_status(404),
)
```

## Contract version

`info.version` is the HTTP contract `1`. The crate version is experimental `0.1.2`. Each model declares its own path; new projects use `/api/v1`. POST is not idempotent. PATCH is last-write-wins and does not return 409. Removing an operation removes it from this document. `Deprecation` and `Sunset` headers are not sent. Clients should treat a missing path as removed, not as temporarily hidden.

`fexu::openapi::validate_document` checks the emitted document: OpenAPI 3.1.0, info, operation ids, responses, parameter locations, local `#/components/schemas/` references, and security requirements that name a defined scheme. `crates/fexu/tests/openapi_client.rs` and `crates/fexu/tests/api_contract.rs` run it against the live document and against SQLite. The PostgreSQL test `postgres_cursor_filters_and_openapi_describe_the_same_list` runs the same exercise.

## Example assertion

```rust
let document = client.get("/api/openapi.json").await?.json()?;
assert_eq!(document["openapi"], "3.1.0");
assert!(document["paths"].get("/admin").is_none());
fexu::openapi::validate_document(&document)?;
```

The executable copy is `crates/fexu/tests/openapi_client.rs`. The browser check is `tests/browser/swagger-docs.spec.js`.
