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

The default file cap is 10 MiB, up to 100 MiB; each request has a bounded timeout. A failed write may have reached storage even if its reply was lost. Automatic retries are disabled. Staged expiry is honored for reads and retain, but expired objects require explicit cleanup. Do not deploy remote staging without a reconciliation policy. No distributed owner quota, automatic orphan scan, multipart support, IAM role refresh or live AWS acceptance is claimed. MinIO contract verification is required before merging this development feature; restart/restore and production lifecycle acceptance are separate work.
