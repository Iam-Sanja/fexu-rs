# Storage

Local storage is the default in generated applications. The development `s3-storage` feature exposes a private S3-compatible backend; it is not in published Fexu 0.2.0. It does not automatically replace local upload routes or settings.

Enable `features = ["s3-storage"]` on a matching development source dependency on Fexu. Provision a private bucket and restrict credentials to its Fexu media prefix. Set `FEXU_S3_ACCESS_KEY` and `FEXU_S3_SECRET_KEY` in the environment.

```rust
use fexu::storage::{
    credentials_from_env, prepare_s3, S3Limits, S3Storage, Storage,
    ACCESS_KEY_ENV, SECRET_KEY_ENV,
};

let plan = prepare_s3(
    "https://storage.example",
    "us-east-1",
    "app-media",
    ACCESS_KEY_ENV,
    SECRET_KEY_ENV,
)?;
let storage = S3Storage::new(
    &plan,
    credentials_from_env(&plan)?,
    S3Limits::default(),
    &["text/plain", "image/png"],
    None,
)?;
let file = storage.save(owner_id, "notes.txt", b"hello").await?;
let (_metadata, bytes) = storage.read_for(owner_id, owner_id, file.id).await?;
```

The example assumes a verified owner ID from the application session. It uses path-style S3 addressing and a regional endpoint. Private CA PEM bytes may be supplied in the final constructor argument; certificate checking remains enabled. No redirect or implicit HTTP proxy is followed. Configure the Rustls provider before construction if your application requires a different one.

The backend stores one versioned envelope per immutable owner/file UUID, including metadata and a SHA-256 digest. Do not serve raw bucket objects: they contain metadata as well as file bytes. Application downloads must enforce verified ownership and safe response headers. Existing signed-download tokens are application tokens, not S3 presigned URLs. The local HTTP routes are not automatically remote-enabled.

The default file cap is 10 MiB, up to 100 MiB; each request has a bounded timeout. A failed write may have reached storage even if its reply was lost. Automatic retries are disabled. Staged expiry is honored for reads and retain, but expired objects require explicit cleanup. Do not deploy remote staging without a reconciliation policy. No distributed owner quota, automatic orphan scan, multipart support, IAM role refresh or live AWS acceptance is claimed. The initial TLS MinIO contracts passed in PR #271. Additional CI coverage checks explicit concurrent retain/delete operations and persistence across a restart of the same disposable MinIO container. A restart is not a backup/restore or failover guarantee. Automatic expiry cleanup, distributed quotas and production lifecycle acceptance remain separate work.

### Explicit expiry reconciliation

The development backend exposes `cleanup_unreferenced_expired(owner, id)` for an operator or application reconciliation service. Call it only after establishing that no database reference exists and preventing new references during reconciliation. It skips missing, retained and not-yet-expired objects. A conditional tombstone write fences a stale retain before physical deletion; a failed cleanup can be retried using the same immutable ID. There is no scheduled sweep, automatic account purge or database transaction. Do not write or recreate raw keys outside Fexu. Keep server clocks synchronized; bucket versioning may retain historical bytes and requires separate retention policy. The targeted cleanup contract passed in PR #273. Additional deterministic tests exercise a stale ETag after retain and resuming a cleanup from a persisted deletion marker; both tests passed in PR #274. They simulate interruption between requests, not a network fault or server crash.

### Reference-aware reconciliation

Use `reconcile_expired` with an application implementation of `ReferenceCheck` and the S3 backend. The check returns `Referenced`, `Unknown`, or `Unreferenced(guard)`. Only the last decision permits cleanup; lookup errors propagate. The guard must exclude new database references across every writer for the entire cleanup operation. This API keeps the guard alive across storage I/O but cannot prove that an application implements distributed exclusion correctly. No default database adapter, scheduled scan or automatic local cleanup is enabled. Existing local `cleanup_orphans` is not reference-aware; do not run it on live referenced media. The reference-aware contract passed in PR #275.

### Opt-in reference ledger

`ReferenceLedger` provides a durable admission/retirement gate shared by SQLite and PostgreSQL writers. Explicitly initialize its table with `initialize` during controlled setup, then enroll only newly allocated media IDs with `register`. Every model reference addition and removal must call `attach` or `detach` in the same database transaction as the model write. The ledger stores a count, not a discovery scan. Missing rows are unknown; retirement is irreversible and requires zero references. `ReferenceCheck` commits retirement before allowing storage cleanup, so a lost connection cannot reopen admission. A retired key remains retired even if storage cleanup fails or finds a permanent or future-dated file. Only submit expired candidates that will never be reused; keep the ledger and media backups consistent. Do not enroll existing media without a separately verified reference migration. Direct writers, legacy form deletion, automatic enumeration and production integration are not covered. The ledger runtime contracts passed on both databases in PR #277.

### Reference-aware model save

After validating and staging an upload, use `StagedMedia::commit_referenced(&storage, &db, save)` instead of the legacy `commit` or `finish_media` path. The async save closure receives the transaction, previous field bindings and replacement bindings. It must compare the current authorized model fields against the previous values under a row lock or an atomic conditional write, persist exactly the replacement values, and return an error on conflict. Do not start or commit a separate transaction. The coordinator enrolls and attaches new IDs, detaches replaced IDs, retains new bytes before committing, and never deletes old bytes. Detached IDs are candidates only, not deletion permission. On any failure, new candidate IDs are reported and no file is deleted; retained but unreferenced bytes can require manual recovery. Cancellation may leave staged or permanent orphans. Do not treat a commit error as proof of rollback. All writers must use the ledger; legacy forms/admin and automatic cleanup remain unintegrated. The SQLite and PostgreSQL model-row contract tests passed in PR #278.

### Explicit orphan discovery and cleanup

`ReferenceLedger::orphan_candidates(after, limit)` returns up to 1,000 enrolled zero-reference IDs in deterministic owner/file order. Pass the last ID as the next cursor, and restart from the beginning on later scans. Results are not a snapshot or permission to delete; newly registered or temporarily unreferenced files can be present. Retired IDs remain visible for repeatable recovery. `cleanup_abandoned(&storage, media)` is an explicit operator operation for permanently abandoned IDs only, including retained files. It commits the irreversible zero-reference retirement before calling the backend delete operation. Unknown and referenced IDs are skipped. Missing and removed bytes are distinguished according to the backend's delete result; errors leave the retirement intact. Do not run this on unexpired staged uploads, in-flight saves, legacy files, or storage shared with nonparticipating writers. No background worker, automatic eligibility policy, physical inventory, durable pre-upload registration, or recovery of unknown IDs is provided. The new SQLite/PostgreSQL tests require CI verification.
