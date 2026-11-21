# Views and URLs

Fexu builds on Axum handlers. Apps declare named routes with `UrlSet`; a route maps to a function in `views.rs`.

**File: `apps/catalog/urls.rs`**

```rust
use axum::routing::get;
use fexu::urls::{UrlError, UrlSet};

pub fn routes() -> Result<UrlSet, UrlError> {
    UrlSet::new()
        .route("catalog.index", "/", get(super::views::index))?
        .route("catalog.status", "/status", get(super::views::status))
}
```

The generator registers your app with prefix `/catalog`. The routes become `/catalog/` and `/catalog/status`. The route names remain `catalog.index` and `catalog.status`.

**File: `apps/catalog/views.rs`** — append a JSON handler:

```rust
pub async fn status() -> axum::response::Response {
    fexu::views::json(&fexu::serde_json::json!({
        "app": "catalog",
        "ready": true
    }))
}
```

For a template-rendered handler, the generated `index` uses the shared `TemplateCatalog`, project settings, and `catalog.render("catalog/index.html", &context)`. The full source-compatible handler appears in [the Bookstore tutorial](../tutorial/first-project.md).

Use appropriate authentication and CSRF checks in state-changing handlers. Registering a URL does not itself confer authorization.
