# Custom admin pages

[Overview](admin-extension-api.md)

## Route

A custom page is served at `/admin/pages/{key}`. `key` must match `[a-z][a-z0-9_]{0,39}`. Reserved names are `admin`, `api`, `assets`, `audit`, `directory`, `login`, `logout`, `models`, `pages` and `preferences`. Passing a duplicate or reserved name to `view` returns an `ExtensionError`. An unknown page returns 404 for an authenticated staff user; a request without a session redirects to login.

## Handler

```rust
struct OrderReport;

#[fexu::async_trait::async_trait]
impl fexu::admin::ViewHandler for OrderReport {
    async fn handle(
        &self,
        request: &fexu::admin::ViewRequest<'_>,
    ) -> Result<fexu::admin::ViewOutcome, fexu::admin::ExtensionError> {
        let date = request.query.date("date")?;
        let listed = Order::list(request.db).await.map_err(|_| {
            fexu::admin::ExtensionError::new("extension.view", "orders could not be read")
        })?;
        let mut data = serde_json::Map::new();
        data.insert("rows".to_owned(), serde_json::json!(listed.data.len()));
        data.insert(
            "date".to_owned(),
            serde_json::json!(date.map(|value| value.to_string())),
        );
        Ok(fexu::admin::ViewPage::new(data).into())
    }
}
```

Before calling `handle`, Fexu checks the session, `is_staff` and `AuthService::holds` for the registered permission code. Superusers satisfy `holds` but still require `is_staff`. A POST without valid CSRF returns 403 without calling the handler. A page without `on_post` returns 405 for POST. Those checks run before the query is parsed, so an anonymous request is sent to login and the login `next` value stays the admin path.

`ViewRequest.query` is the GET query after those checks. Keys are lowercase `name` tokens. Values are short visible ASCII tokens such as `2026-01-02`. Encoded `.`, `/` and `\`, duplicate keys, controls and off-site text are rejected with 422 and the handler is not called. `query.date` accepts only `YYYY-MM-DD`. A missing key is empty. A value that is present but not a real date becomes 422 inside the handler, before a database read, when the handler uses `?`. Parsing the query does not write.

`handle` returns `ViewOutcome`:

| Variant | HTTP |
|---|---|
| `Page` | 200 and the registered template. `notice` and `alert` are shortened and escaped by Tera. |
| `redirect` | 303 to a canonical `/admin` path. The `Location` header is that path only. |
| `not_found` | 404 |
| `conflict` | 409 |
| `invalid` | 422 |

`ViewOutcome::redirect` rejects schemes, protocol-relative paths, backslashes, fragments, encoded separators and any path outside `/admin`. The response then has no `Location` header and does not repeat the rejected target. An existing handler that returned `ViewPage` now returns `Ok(page.into())`.

The template is chosen by the developer during registration, not the request. `custom.html` prints `date` and `body` as escaped text.

The sidebar includes pages registered with `navigation(true)` only when the current user holds the required permission. This applies to the dashboard, lists, forms and custom pages.

A custom page POST may use `multipart/form-data`. Text parts become `ViewRequest.form`. File parts become `ViewRequest.uploads` (`UploadedFile`: field, filename, declared Content-Type, bytes, `size()`). The declared type is not trusted. Duplicate field names and filenames containing a separator, `:`, `.` or `..` are rejected with 400 after authentication. Bodies larger than `MULTIPART_BODY_LIMIT` (1 MiB) are rejected with 413 before the handler runs. CSRF, staff and the page permission are still required; a rejected request does not call the handler, so the framework does not store the file. URL-encoded forms are unchanged and have an empty `uploads` slice. The handler decides whether to call storage.
