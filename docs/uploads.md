# Uploads

**Milestone:** v0.12, Implemented for expressly registered routes. The Cargo version is `0.1.2`.

## Responsibility

The upload routes are not mounted by default. An application calls `fexu::storage::routes` and mounts the router with `App::merge`. Without auth service, the routes answer with 403.

## Crates and files

- `crates/fexu-storage/src/http.rs`
- Example: `examples/myshop/src/main.rs`
- Storage details: [storage.md](storage.md)

## Public API

`POST /uploads` takes either the raw body with `x-original-name` and `x-csrf-token` or `multipart/form-data` with the fields `csrf_token` and `file`. The JSON response contains `id`, `original_name`, `content_type` and `size`. It does not contain a file system path. `GET /uploads/{id}` only loads its own file. `DELETE /uploads/{id}` requires session and CSRF headers.

## Data flow

The body is read until `max_bytes + 1` and discarded when exceeded. The content determines the type, not the client header. After that, `authenticate_csrf` reviews the existing session. `save` writes the UUID file. Download and delete filter via the user ID from the session.

## Configuration

The size comes from `[storage]`. The allowed types are set by the caller of `LocalStorage::open`. They are not additional TOML keys so that an allowlist does not accidentally open everything.

## Security

Deny by default. Authentication is required. The file ID is a UUID, not a path. Foreign IDs are 404. Deletion without CSRF is 403. Incomplete files are removed. Uploaded files are not executed and are not shipped as HTML or SVG. `media/` is in the project `.gitignore`.

## Error cases

Responses are 401 for a missing session, 403 for missing CSRF or auth service, 404 for another owner’s or missing files, 413 for oversized uploads, 415 for disallowed content and 500 for storage failures without system details.

## Example

```rust
let storage = fexu::storage::LocalStorage::open("media", 1024 * 1024, &["text/plain"])?;
let app = app.merge(fexu::storage::routes(std::sync::Arc::new(storage)));
```

## Tests

The HTTP test creates two users in SQLite, loads a text file, denies anonymous access, delivers a second user 404, and deletes only with the owner’s CSRF token. The owner download sends `attachment`, `application/octet-stream`, `nosniff` and a sandbox CSP. A body labeled `image/png` is still rejected when the bytes are text.

## Limitations

There is no virus scanner, no image processing and no public upload URL. `read_multipart` accepts a quoted boundary, at most 16 parts and 1 MiB. Duplicate names are errors. `inspect_image` is a separate, optional container check. `decode_png` inflates PNG pixels under a limit and is the check an image field uses. See [storage.md](storage.md).

## Form integration

An application `forms.rs` uses `MediaField`, `read_media_parts`, `bind_media` and `finish_media`. `read_media_parts` parses `multipart/form-data` and writes nothing. `bind_media` checks required, optional, existing, replace, clear and readonly fields, including per-field image limits, and returns `FormErrors`. `finish_media` stages only after that check, polls the database future, and then calls `abort` or `commit`. Those storage calls are not inside the database transaction. A failed delete or retain stays on `CommitReport`. `store_upload` and `inspect_upload` stay the immediate single-file helpers.

Admin record images stay in `_fexu_record_files`. The form handler calls the same filename, MIME and PNG/JPEG checks before the record transaction. GIF, WebP and ICO stay signature checks. A clear checkbox deletes that blob only inside the same transaction as the record. A database rollback removes the new blob and keeps the previous one. That is not a `LocalStorage` commit, and it is not one transaction with app media files. The private image response still sends `nosniff` and `Content-Security-Policy: default-src 'none'`. CSRF stays on the admin POST.

## Extension

A virus scanner can be mounted after `detect_type` and before `save`. It is not included.
