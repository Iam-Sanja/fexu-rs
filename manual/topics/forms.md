# Forms and validation

Fexu supports typed HTML/JSON form input independently of database models. Your application defines the route and handler; the form layer handles parsing, validation and safe HTML rendering.

This guide uses the same form APIs as the working `examples/myshop/src/contact.rs` example.

## 1. Define the route

In a generated app, extend **`apps/catalog/urls.rs`**:

```rust
use axum::routing::get;
use fexu::urls::{UrlError, UrlSet};

pub fn routes() -> Result<UrlSet, UrlError> {
    UrlSet::new()
        .route("catalog.index", "/", get(super::views::index))?
        .route(
            "catalog.contact",
            "/contact",
            get(super::views::contact_get).post(super::views::contact_post),
        )
}
```

These handlers are additions to your `apps/catalog/views.rs` file, not built-in routes. Because the generated app uses the `/catalog` prefix, the URL is `/catalog/contact`.

## 2. Parse and validate form input

The simplest example of typed validation uses `FormValues` and `required_text`:

```rust
use fexu::forms::{required_text, FormValues, TextSpec};

fn contact_name(body: &str) -> Result<String, fexu::forms::FormErrors> {
    let values = FormValues::parse_urlencoded(body)?;
    fexu::forms::reject_unknown(&values, &["name"])?;

    let name = required_text(
        &values,
        TextSpec {
            name: "name",
            required: true,
            min_length: Some(1),
            max_length: Some(120),
            email: false,
            url: false,
        },
    )
    .map_err(|failure| fexu::forms::FormErrors {
        fields: vec![failure],
    })?;

    Ok(name)
}
```

This validates the name **on the server**. A required HTML attribute is useful for browser interaction but cannot replace server-side validation. Form parsing also detects malformed input and duplicated field names.

For the built-in typed contact example:

```rust
use fexu::forms::{ContactForm, FexuForm, FormValues};

fn parse_contact(body: &str) -> Result<ContactForm, fexu::forms::FormErrors> {
    let values = FormValues::parse_urlencoded(body)?;
    ContactForm::from_form(&values)
}
```

`ContactForm` is a supplied example of the `FexuForm` contract; the form crate can also derive model-bound and standalone forms. The result is typed data, not a database write.

## 3. Render a template

**File: `apps/catalog/templates/catalog/contact.html`**

```html
{% extends "base.html" %}
{% block content %}
  <h1>Contact the bookstore</h1>
  <form method="post" action="/catalog/contact">
    <label for="name">Your name</label>
    <input id="name" name="name" required maxlength="120">
    <!-- A real handler must render the session's actual CSRF token here. -->
    <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
    <button type="submit">Send</button>
  </form>
{% endblock %}
```

A value such as `{{ csrf_token }}` must be added to the template context by the handler and must come from Fexu's authenticated or anonymous session CSRF flow; it is **not** an automatic Tera variable. Don't invent a token or disable CSRF for the tutorial.

## 4. Connect the form to a secure handler

The completed contact handler in the repository is **`examples/myshop/src/contact.rs`**. It demonstrates the full end-to-end sequence:

1. GET: generate or reuse an anonymous session and its CSRF token via `AuthService::csrf_from_request` or `ensure_anonymous_session`.
2. Render fields and submit token with `render_fields` and the form's widget definitions.
3. POST: parse submitted `FormValues`, read `csrf_token` and call `authenticate_browser_form` with `Method::POST`.
4. Validate via `ContactForm::from_form`.
5. Show validation errors again with status 422, or redirect to a named success URL.

These security steps are essential. The rendering and parsing snippets above are **components**, not an entire ready-to-run POST handler. The repository example is the authoritative runnable implementation until the Bookstore tutorial gains its own compiled form tests.

## 5. Model-bound forms and uploaded files

Model form-binding uses explicit allowed columns, field contracts and submission modes. The framework supports `SubmitMode::Create`, `Replace` and `Patch`; a missing PATCH field remains unchanged rather than being overwritten automatically.

Uploads need separate input-size, content-type and storage checks. Fexu provides upload inspection and `LocalStorage` integration; storing bytes and linking them to a model is an application responsibility. A browser file input alone does not provide a secure upload handler.

### Staged uploads and replacement failures

In the v0.3 development version, the `Storage` contract rejects a staging deadline by default unless the backend implements expiring uploads. A backend that supports staged media must implement both `save_expiring` and `retain`.

When using the staged media workflow (including `fexu::forms::finish_media`), abort staged uploads if the database transaction rolls back. After a successful database commit, retain the new files before removing previous files. If any retain operation fails, Fexu defers all previous-file removals for that batch. Inspect `CommitReport::retain_failed` and `CommitReport::removal_failed`; the latter also includes deferred removals. The database transaction has already committed, so reconcile its references and retention before running orphan cleanup. The previous bytes remain available, but the new file may still expire.

Local storage remains the default. The development-only optional S3 backend is available through explicit construction; see the [storage guide](storage.md). Content checks do not replace malware scanning or full image decoding.

## Next steps

Read [views and URL routing](views-and-urls.md), [models](models.md), [authentication and admin](auth-and-admin.md), and [the complete first-project tutorial](../tutorial/first-project.md).
