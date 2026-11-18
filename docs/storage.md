# Storage

**Milestone:** v0.12, implemented for local storage. Remote object storage is a checked configuration, not a client. The Cargo version is `0.1.2`. See [storage-remote.md](storage-remote.md).

## Responsibility

`fexu-storage` stores bytes under a root that the application sets. The client filename never becomes the path.

## Crates and files

- Trait and types: `crates/fexu-storage/src/lib.rs`
- Local backend: `crates/fexu-storage/src/local.rs`
- HTTP, only after explicit attachment: `crates/fexu-storage/src/http.rs`

## Public API

`Storage` is asynchronous: `save`, `read`, `delete`, `exists`, `metadata`. `LocalStorage::open(root, max_bytes, allow)` creates the root. `detect_type` recognizes PNG, JPEG, GIF, WEBP, PDF and simple text based on the bytes. `routes` provides `POST /uploads`, `GET /uploads/{id}` and `DELETE /uploads/{id}`.

The local backend performs brief blocking `std::fs` operations. This is a known limitation of the current storage implementation.

## Data flow

`save` checks size and content, creates a UUID and writes `{root}/{owner}/{id}.partial`. Only then will the file be moved exclusively to `{id}`. Metadata is next to it in `{id}.json`. If any step fails, the partial file, the data file and the sidecar are removed. `read` returns the bytes only when their length matches `size`. A stored `sha256` must match too. An older sidecar without `sha256` stays readable when the length still matches. There is no new SQL migration. The existing checksums remain valid.

## Configuration

```toml
[storage]
backend = "local"
directory = "media"
max_upload_size_mb = 10
```

`backend` only accepts `local`. `directory` is relative without `..`. The size is between 1 and 100 MiB. `fexu::web::prepare` sets the body limit at least to this size. myshop opens `examples/myshop/media` with 1 MiB and the allowlist `text/plain`, `image/png`, `image/jpeg`, `application/pdf`.

## Security

Paths are canonicalized and must stay inside the owner’s directory. A symlinked owner directory is refused, and a symlinked file is not opened. Cleanup unlinks those symlinks without deleting their targets. `create_new` prevents silent overwrites. The original filename is sanitized to ASCII letters, digits, periods, hyphens and underscores before it is stored and again when the download header is built. A name that is only dots is rejected. Quotes and line breaks cannot add a second HTTP header. When the name has an extension, it must match the type detected from the bytes. A client `Content-Type` is not accepted as proof. HTML, SVG, scripts, `MZ` and `ELF` are rejected as text uploads. `OwnerAccess` allows an action only when the actor is the owner. HTTP requests from another owner return 404. Downloads use `Content-Disposition: attachment`, `application/octet-stream`, `nosniff` and `Content-Security-Policy: sandbox; default-src 'none'`. `save_until` sets an expiry. `cleanup_orphans` removes partial files, mismatched sidecars and expired files without following symlinks. `purge_owner` removes one owner's directory. Quotas are per process. Signed URLs and the S3 plan are described in [storage-remote.md](storage-remote.md).

## Error cases

`upload.too_large` returns 413, `upload.type` returns 415 and `resource.not_found` returns 404. Storage failures return `request.rejected` with status 500 and expose neither paths nor system error text.

## Example

```rust
let stored = storage.save(user_id, "notes.txt", b"hello").await?;
let (_meta, bytes) = storage.read(user_id, stored.id).await?;
```

## Tests

`cargo test -p fexu-storage` stores, reads, deletes, isolates owners, rejects size, HTML and path names, and checks anonymous and third-party HTTP accesses.

## Image checks

`detect_type` only recognizes a signature. A short `GIF89a` or a PNG prefix is enough for that function, and `save` still accepts those bytes when the allowlist contains the detected type. That is not image decoding. `inspect_image` additionally checks PNG chunk framing or JPEG markers, the declared width and height, and `ImageLimits` (`max_bytes`, `max_width`, `max_height`, `max_pixels`). It does not inflate PNG pixels or decode a JPEG scan, and it rejects GIF and PDF bytes. A later decoder can still allocate more memory than the declared size. `LocalStorage::with_check` runs a `BeforeSave` hook before creating a file. `ImageHeaderCheck` uses `inspect_image`. Files that fail the hook are not written. Uploads without a hook keep the signature check.

## Field lifecycle

`prepare_media` checks the actor, the field name, the filename, the declared MIME type, the signature, the extension, the file size and the combined request size. It writes nothing. An image field then calls `validate_image`. A PNG is inflated and its scanlines are reconstructed under `ImageLimits`, and the pixel buffer is dropped. A JPEG is still only a marker and dimension check; `ImageValidation::JpegContainer` means the pixels were not decoded. GIF, WebP and PDF are not accepted as image fields. `inspect_image` and `detect_type` remain signature or container checks and are not a decode.

`UploadPlan::stage` stores each new file with `save_expiring` and a lifetime of at most 24 hours. The previous id stays. If a later file in the same batch cannot be written, the ids created by that call are deleted. `StagedMedia::abort` deletes only the staged ids. Call it when form validation fails or the database transaction rolls back. `StagedMedia::commit` runs after that transaction has committed: `retain` clears the expiry, then the previous ids are deleted. `CommitReport::removal_failed` lists previous files that are still readable. `retain_failed` lists staged files whose expiry could not be cleared; `cleanup_orphans` can remove them after the deadline even though a database row points at them. A database rollback does not call the storage backend.

`MediaRef` is the owner id plus the file id. The client filename is stored only as a sanitized download label and is never a path. A readonly field rejects posted bytes and a clear flag. An empty body counts as no upload. Duplicate field names and duplicate basenames, including names that differ only by case, are `validation.duplicate`.

## Limitations

S3 has no network client. See [storage-remote.md](storage-remote.md). Malware scanning is missing. Content recognition does not replace a virus scanner. Header checks do not decode image pixels. JPEG image fields are not pixel-decoded. PNG decoding accepts 8-bit gray, RGB and RGBA without interlacing and rejects other PNG forms. A mismatched `sha256` or length makes `read` return `Failed` and leaves the file on disk. Metadata is Sidecar JSON, not a database table. `StoredFile` accepts older sidecars that omit `created_at`, `expires_at` and `sha256`.

Account close and admin record deletion do not call `purge_owner` or `delete`. Auth does not depend on this crate. `FieldSpec::image_limits` overrides the policy for one image field. `fexu_forms::finish_media` is the app helper for stage, database save, abort and commit, in that order. It does not make the database and the storage backend one transaction.

Admin image widgets still store bytes in `_fexu_record_files` inside the record transaction. They validate through `prepare_media` first. PNG bytes are inflated. JPEG bytes stay a container check. GIF, WebP and ICO are not moved to `LocalStorage`. A failed record save rolls the blob back with the row. `fexu-forms::store_upload` remains the immediate single-file helper and can still store a PNG signature prefix. `media_field_errors` copies the lifecycle error codes.

## Extension

Other backends can implement the same `Storage` methods. A future metadata table would require a new migration; existing `0001` files must remain unchanged.
