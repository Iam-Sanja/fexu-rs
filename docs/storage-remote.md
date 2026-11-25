# Remote storage

[Overview](index.md) · Local storage: [storage.md](storage.md)

**Development feature:** `s3-storage` enables `S3Storage`. `prepare_s3` checks an HTTPS origin (no userinfo, non-root paths, query, fragment or port zero), a region and a bucket name. It does not open a socket. `credentials_from_env` reads the two environment names stored on the plan at call time. The names are `FEXU_S3_ACCESS_KEY` and `FEXU_S3_SECRET_KEY` when the application chooses those constants. Credentials are not compiled into the binary. `Debug` for `S3Credentials` prints `[redacted]`.

LocalStorage remains the default. S3Storage implements explicit network operations against a pre-provisioned private bucket. `Storage` is the backend-neutral contract (`save`, `read`, `delete`, `exists`, `metadata`, `save_expiring`, `retain`). The default `save_expiring` rejects a supplied deadline and the default `retain` fails, so a backend has to override both before it can stage files. The optional client uses Apache object_store 0.14.2 and a dedicated HTTPS-only reqwest transport with no automatic redirects or proxy discovery. Ring signs requests and supplies the default Rustls provider if the application has not installed one.

Signed downloads are separate from cookies. `sign_download` builds `v1.{owner}.{id}.{exp}.{mac}` with HMAC-SHA256. The key is at least 32 bytes and comes from the caller. `sign_download_for` accepts a lifetime from 1 second through 15 minutes. A bad MAC is `NotFound`. An expired signature is `Expired`. The signature does not grant another owner's file. The local download response uses `Content-Disposition: attachment`, `application/octet-stream`, `nosniff` and `Content-Security-Policy: sandbox; default-src 'none'`. The filename is the sanitized basename.

`save_for` and `read_for` require the actor id to equal the owner id. Staff is not implied.

Quotas are best-effort inside one process. `0` means unlimited. A second process does not share the counter.

`cleanup_orphans` removes `*.partial` files, JSON without bytes, bytes without JSON, and expired files. It does not follow symbolic links. `purge_owner` removes one owner's directory. Authentication does not call it, because auth does not depend on storage.

## Remote contract

Each object at `fexu-media/v1/{owner}/{id}` contains a bounded FXS1 envelope: magic, big-endian JSON length, StoredFile JSON and the original bytes. Create is conditional (If-None-Match); retain uses the observed ETag (If-Match) and cannot recreate a deleted object. Metadata and bytes publish together. Reads stream with a size cap, validate owner/id, name, type, length and SHA-256, and reject expiry. Metadata/exists currently read the whole object; objects are not directly downloadable as original bytes. A successful PUT can be ambiguous if the connection fails; automatic retries are disabled. Do not retry blindly. S3 has no cross-backend database transaction.

The constructor revalidates the public plan and requires explicit credentials, an HTTPS origin, pre-existing private bucket and a nonempty allowlist. The optional PEM CA bundle supplements system trust. The timeout is bounded to 60 seconds per request; max file size 100 MiB, default 10 MiB. Local upload validation and optional BeforeSave hook are reused. Use separate buckets for environments; restrict credentials to the Fexu prefix. No public ACL, direct presigned URLs, IAM session-token/role refresh, multipart uploads, global quotas or automatic remote orphan cleanup are provided in this slice. Expired objects cannot be read or retained, but are not physically removed until explicit deletion/reconciliation. Do not enable staged production forms until a cleanup/reconciliation policy exists. HTTP routes remain local-only; applications must enforce authorization before calling the trait. An isolated TLS MinIO contract test is required before merge; real AWS service, restore, quota enforcement and failure drills remain separate acceptance work.
