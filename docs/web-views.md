# HTML views, flash messages and web helpers

[Overview](index.md) · Responses: [views.md](views.md) · Forms: [forms.md](forms.md) · Routes: [urls.md](urls.md)

**Status:** the helpers are on `main` and are ordinary Rust functions, not class-based views. `crud_urls_for` mounts a chosen subset. Issue #89 stays open for the items under [Open requirements](#open-requirements).

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

`crud_urls` and `crud_urls_with` still mount all five parts. `crud_urls_for` mounts only the requested `CrudPart` values. An empty list or a repeated part is rejected. Omitted parts have no route. List, detail and delete pages leave the corresponding link empty, and the board templates render those links only when the URL is present. `crud_urls_with` is `crud_urls_for` with every part, so existing callers keep the same routes.

The example is `examples/board`. `lib.rs` holds the `Note` model, an `ObjectPolicy`, a `RecordStamp`, the `HtmlResource` and the `crud_urls_with` registration. Queries go through `ModelRepository` and `query_admin_page`. `forms.rs` and `project.rs` sit beside those routes. They are not part of `crud_urls`.

HTML CRUD pages set `locale` with `locale_of` in `crates/fexu-api/src/generic.rs`. `Accept-Language` that starts with `en` selects English. Every other value, including a missing header, selects German. That rule does not read `?lang=`, `q` weights or `project.default_locale`. `negotiate_locale` in `pages.rs` is a different function. `locale_from` in `views.rs` uses the same `en` prefix for HTML error pages.

### Measured application code

`examples/board/src/lib.rs` is 187 lines. That file holds the `Note` model, `NotePolicy`, `NoteStamp`, the `HtmlResource`, the one `crud_urls_with` registration, and the robots and sitemap handlers. The four templates are 82 lines together. Each sets `lang` from `locale`. Fixed titles and links branch on `locale == "de"` (`New note` / `Neue Notiz`, and the same pattern on the list, form and delete pages). The detail title is the stored record title. `examples/board/src/forms.rs` is 135 lines and includes model forms plus a required media field. `examples/board/src/project.rs` is 265 lines and saves a project and its tasks in one transaction. `examples/myshop/src/contact.rs` is 182 lines and implements one public form: it parses the body, checks CSRF, renders errors and redirects. It has no list, no object scope, no update and no delete. The five CRUD handlers, the scope checks and the flash redirect stay in `fexu-api`.

### Open requirements

These are not part of this API:

- Choice columns with a non-empty static list render as a `<select>` from `model_widget`. The option label is `label_de` when the form locale is `de`, and `label_en` otherwise. A choice column with no options stays a text input.
- The generic routes do not upload files, render admin screens, or mount inlines. The board media field and the project/task formset are the separate modules above.
- The generic routes do not register the JSON API.
- `_fexu_flashes` is created only by `create_flash_table`.
- The PostgreSQL board test stays ignored unless `DATABASE_URL` points at an approved server that can create a disposable database. It was not run for this change.
- Sitemap, robots and conditional responses already exist and were not replaced.

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

Choice columns with a non-empty static list are validated and drawn as a `<select>`. The option label is `label_de` when the form locale is `de`, and `label_en` otherwise. A choice column with no options stays a text input. Those routes do not upload files or render admin screens. The generic routes do not register the JSON API; call `register` or `registry.model` only when that API should exist, and bind an object policy there too. `_fexu_flashes` is created only by `create_flash_table`.

## Flash messages

`FlashLevel` is `success`, `info`, `warning` or `error`. `push_flash` rejects an empty line, a line longer than 240 characters, and control characters. `take_flashes` returns the session rows and deletes them in the same transaction. A second take is empty. Messages for another session stay put.

HTML CRUD pages put the consumed messages in `flashes` as `{ level, text }`. Render them as text:

```html
{% for flash in flashes %}
  <li data-level="{{ flash.level }}">{{ flash.text }}</li>
{% endfor %}
```

`render_flashes` remains for a caller that wants one escaped HTML fragment.

`redirect_with_flash(db, session_id, level, text, target)` checks the same local-path rules as `redirect_to` before it stores anything. A rejected target, a control character, or a missing `_fexu_flashes` table produces an error and does not set `Location`. A stored message is still removed by the next `take_flashes` for that session only. This helper does not change session cookies or list pagination.

## Sitemap, robots, redirects and conditional responses

| Helper | Behavior |
|---|---|
| `canonical_url(origin, path)` | Joins a configured `http`/`https` origin and a local path. The origin cannot contain a user, a path or a query. `Host` is not read. |
| `sitemap_xml(origin, paths)` | At most 1000 local paths. Locations are escaped. |
| `robots_txt(origin, allow, disallow, sitemap)` | `User-agent: *` only. Paths must be local. |
| `redirect_to` | Allows one local `/` path. Encoded `.`, `/` and `\`, including repeated encodings, plus schemes and backslashes, are rejected. |
| `redirect_next(next, fallback, prefixes)` | Uses `next` only when it stays under an allowed prefix. `/` means any local path. An empty `next` uses the fallback. |
| `page_links` | Builds previous and next links with `pagination_href`. |
| `conditional_response` | Sets a SHA-256 ETag. A matching `If-None-Match` on a 200 response is 304 with an empty body. The request's validator is not copied into the response. HTML CRUD uses `private, no-cache`. |

Static files keep their own ETag logic in `fexu-static`. These helpers are for generated documents.

## Shared interfaces

`fexu-api` now depends on `fexu-forms` and `sha2`. `FlashLevel` gained `Warning`. `redirect_to` normalizes percent-encoding before it accepts a target. Auth, the cookie CSRF middleware, OpenAPI and the admin UI are unchanged.

Other work on `fexu-api/Cargo.toml` needs to keep the `fexu-forms` dependency. Do not point the generic views at a different session table or skip `authorize`.

## Tests

`examples/board/tests/crud.rs` covers the mounted HTML flow on SQLite: anonymous 401, missing permission 403, a rejected `scope` query, missing and forged CSRF on create, update and delete, validation, escaped titles, owner isolation, a foreign id on read, update and delete, a missing id, `page=0`, a POST to the list, pagination under `/board`, canonical output, `robots.txt`, `sitemap.xml`, a 304 and delete. A second test mounts only list and detail under `/reading` and checks that create, update and delete are absent. The same full test is ignored for PostgreSQL unless `DATABASE_URL` points at an approved server that can create a disposable database.

Library tests cover redirect decoding, pagination links, flash escaping and the sitemap/robots/ETag helpers.
