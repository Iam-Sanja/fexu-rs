# Remote storage plan

[Overview](index.md) · Local storage: [storage.md](storage.md)

**Not a client.** `prepare_s3` checks an HTTPS origin (no userinfo, non-root paths, query, fragment or port zero), a region and a bucket name. It does not open a socket. `credentials_from_env` reads the two environment names stored on the plan at call time. The names are `FEXU_S3_ACCESS_KEY` and `FEXU_S3_SECRET_KEY` when the application chooses those constants. Credentials are not compiled into the binary. `Debug` for `S3Credentials` prints `[redacted]`.

There is no success path that pretends an upload reached a bucket. Callers that need bytes today use `LocalStorage`. `Storage` is the backend-neutral contract (`save`, `read`, `delete`, `exists`, `metadata`, `save_expiring`, `retain`). The default `save_expiring` rejects a supplied deadline and the default `retain` fails, so a backend has to override both before it can stage files. An S3 client is not part of this crate. A client that cannot be tested without a live bucket would report success it cannot prove.

Signed downloads are separate from cookies. `sign_download` builds `v1.{owner}.{id}.{exp}.{mac}` with HMAC-SHA256. The key is at least 32 bytes and comes from the caller. `sign_download_for` accepts a lifetime from 1 second through 15 minutes. A bad MAC is `NotFound`. An expired signature is `Expired`. The signature does not grant another owner's file. The local download response uses `Content-Disposition: attachment`, `application/octet-stream`, `nosniff` and `Content-Security-Policy: sandbox; default-src 'none'`. The filename is the sanitized basename.

`save_for` and `read_for` require the actor id to equal the owner id. Staff is not implied.

Quotas are best-effort inside one process. `0` means unlimited. A second process does not share the counter.

`cleanup_orphans` removes `*.partial` files, JSON without bytes, bytes without JSON, and expired files. It does not follow symbolic links. `purge_owner` removes one owner's directory. Authentication does not call it, because auth does not depend on storage.

## Remote implementation design (planned)

The next slice should use an established maintained S3-compatible client with Rust 1.89 support and an optional feature, not a custom signature implementation. Revalidate the public plan on construction and use HTTPS with certificate verification. No request should be redirected to an untrusted destination. Use a dedicated private bucket or scoped prefix and credentials restricted to the necessary operations. The object key must derive solely from owner UUID and immutable media UUID, never from the original filename. Preserve the same validated metadata and SHA-256 contract as local storage; publish metadata and bytes as a single object if supported by the selected client. For staging and retain, choose an atomic metadata transition or a conditional write; do not silently ignore a deadline or overwrite a newer object. A failed retain defers previous-file removals. Define an explicit bounded read, quota and reconciliation policy; do not pretend process-local counters are global quotas. Before enabling the backend in generated projects, test against an isolated TLS S3-compatible service, including missing objects, corruption, denied credentials, concurrent uploads, staged expiry, failed retention, delete and restart/restore. AWS-specific acceptance requires a separate fixture; no live bucket or production credentials in CI.
