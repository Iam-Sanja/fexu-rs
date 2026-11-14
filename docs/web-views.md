# HTML views, flash messages and web helpers

[Overview](index.md) · Responses: [views.md](views.md) · Forms: [forms.md](forms.md) · Routes: [urls.md](urls.md)

**Status:** implemented on this branch for issue #89. Not merged. The helpers are Rust functions, not class-based views.

## What already existed

`fexu::views` already rendered templates, rejected external redirects, checked a session and a permission, and parsed small form bodies. `scoped_list` and `scoped_get` already applied a `QueryScope`. `push_flash` / `take_flashes` already stored one-shot messages in `_fexu_flashes`. Static files already answer `If-None-Match`.

The new layer connects those pieces for ordinary HTML CRUD. It does not add a second session store, a second CSRF check, or handwritten SQL.

## CRUD

`crud_urls` registers five named routes for one model:

| Name | Path shape | Methods | Permission action |
|---|---|---|---|
| list | `/items` | GET | `read` |
| create | `/items/new` | GET, POST | `create` |
| detail | `/items/{id}` | GET | `read` |
| update | `/items/{id}/edit` | GET, POST | `update` |
| delete | `/items/{id}/delete` | GET, POST | `delete` |

Paths are local to the app. `Registration::routes` adds the app prefix, and the handlers reverse the mounted path from the route map. A list link for an app mounted at `/board` is `/board/items`, not `/items`.

```rust
registry.routes(fexu::views::crud_urls_with::<Note, NotePolicy, NoteStamp>(resource())?)?;
```

The example is `examples/board`. The application code is the model, an `ObjectPolicy`, a `RecordStamp`, the `HtmlResource` and four templates. Queries go through `ModelRepository` and `query_admin_page`.

### Security rules

- `permission: None` denies every caller. `Some("authenticated")` requires an active session. Any other value uses `{permission}.read|create|update|delete` through `AuthService::authorize`.
- POST calls `authenticate_csrf` with the `csrf_token` field. The existing cookie middleware still runs first.
- `QueryScope::None` yields an empty list or a 404. A row outside `can_read`, `can_update` or `can_delete` is a 404.
- A superuser receives `QueryScope::All` and passes the object check, matching the JSON API. CSRF and `authorize` still run.
- `omit` names are rejected if the browser sends them. `RecordStamp` may set those columns after binding, so an owner id comes from the session.
- Unknown fields, readonly fields and model validation errors re-render the form with status 422. Field text is escaped by `render_fields`.
- Success and delete respond with 303 to a reversed local path. The flash is shown on the next GET and then deleted.
- Page queries accept only `page` and `page_size`. `scope`, tokens and external paths are rejected.
- Templates receive record JSON. Tera escapes `{{ }}`. `form_html` is already escaped and is inserted with `| safe`. Generated local URLs are also inserted with `| safe`. The URL helpers reject quotes, apostrophes, angle brackets and ampersands in the path, and they percent-encode query values.

`KeepRecord` is the stamp that changes nothing. Use it through `crud_urls`.

### Limits

Choice columns are validated and drawn as text inputs. A `<select>` still needs the existing form renderer when the option list is `'static`. File uploads, inlines and admin screens are unchanged. The generic routes do not register the JSON API; call `register` or `registry.model` only when that API should exist, and bind an object policy there too. `_fexu_flashes` is created only by `create_flash_table`.

## Flash messages

`FlashLevel` is `success`, `info`, `warning` or `error`. `push_flash` rejects an empty line, a line longer than 240 characters, and control characters. `take_flashes` returns the session rows and deletes them in the same transaction. A second take is empty. Messages for another session stay put.

HTML CRUD pages put the consumed messages in `flashes` as `{ level, text }`. Render them as text:

```html
{% for flash in flashes %}
  <li data-level="{{ flash.level }}">{{ flash.text }}</li>
{% endfor %}
```

`render_flashes` remains for a caller that wants one escaped HTML fragment.

## Sitemap, robots, redirects and conditional responses

| Helper | Behavior |
|---|---|
| `canonical_url(origin, path)` | Joins a configured `http`/`https` origin and a local path. The origin cannot contain a user, a path or a query. `Host` is not read. |
| `sitemap_xml(origin, paths)` | At most 1000 local paths. Locations are escaped. |
| `robots_txt(origin, allow, disallow, sitemap)` | `User-agent: *` only. Paths must be local. |
| `redirect_to` | Decodes the path once and allows a single local `/` path. Encoded `..`, `//`, backslashes and schemes are rejected. |
| `redirect_next(next, fallback, prefixes)` | Uses `next` only when it stays under an allowed prefix. `/` means any local path. An empty `next` uses the fallback. |
| `page_links` | Builds previous and next links with `pagination_href`. |
| `conditional_response` | Sets a SHA-256 ETag. A matching `If-None-Match` on a 200 response is 304 with an empty body. The request's validator is not copied into the response. HTML CRUD uses `private, no-cache`. |

Static files keep their own ETag logic in `fexu-static`. These helpers are for generated documents.

## Shared interfaces

`fexu-api` now depends on `fexu-forms` and `sha2`. `FlashLevel` gained `Warning`. `redirect_to` normalizes percent-encoding before it accepts a target. Auth, the cookie CSRF middleware, OpenAPI and the admin UI are unchanged.

Other work on `fexu-api/Cargo.toml` needs to keep the `fexu-forms` dependency. Do not point the generic views at a different session table or skip `authorize`.

## Tests

`examples/board/tests/crud.rs` covers the mounted HTML flow on SQLite: anonymous 401, missing permission 403, a rejected `scope` query, missing and forged CSRF, validation, escaped titles, owner isolation, pagination under `/board`, canonical output, `robots.txt`, `sitemap.xml`, a 304 and delete. The same test is ignored for PostgreSQL unless `DATABASE_URL` points at an approved server that can create a disposable database.

Library tests cover redirect decoding, pagination links, flash escaping and the sitemap/robots/ETag helpers.
