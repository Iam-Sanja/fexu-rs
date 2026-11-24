# Cache

The local `MemoryCache` is available without Redis or another service. The optional Redis backend is development functionality, not part of published Fexu 0.2.0.

## Local cache

Use `TypedCache::new(MemoryCache::new(MemoryLimits::default())?, 1)` for bounded process-local storage. A separate instance does not see the entries. Typed values include a serialization version. Use explicit namespace invalidation after changes to authoritative data.

## Optional Redis cache

Enable the facade feature in your development project:

```toml
fexu = { version = "0.3.0-dev.1", features = ["redis-cache"] }
```

Use a matching development source checkout until the candidate is published. Set `FEXU_REDIS_URL` in the environment (the application reads it explicitly). For example:

```rust
use fexu::cache::{RedisCache, RedisLimits, TypedCache};
use std::time::Duration;

let url = std::env::var("FEXU_REDIS_URL")?;
let backend = RedisCache::new(&url, "myshop", RedisLimits::default())?;
let cache = TypedCache::new(backend, 1);
cache.set("tenant.42", "catalog", &vec!["book"], Some(Duration::from_secs(30))).await?;
let value: Option<Vec<String>> = cache.get("tenant.42", "catalog").await?;
cache.invalidate_namespace("tenant.42").await?;
```

Use the same application prefix across instances, a different prefix across environments, and a separate namespace for each tenant or consistency domain. The backend accepts standalone Redis URLs (`redis://` on an isolated trusted network or certificate-verified `rediss://`); insecure TLS fragments and URL queries are refused. Credentials belong in the environment, not source code. Keep Redis unpublished, use an ACL limited to your cache prefix and required commands, and configure server memory and eviction limits. No connection is made until the first operation.

Every Redis value has a TTL: by default 300 seconds, with a maximum of 24 hours. `None` selects the default, rather than permanent storage. Values are limited to 1 MiB by default (including typed serialization headers). Each operation opens a connection and has a bounded total timeout (default 2 seconds). A later operation may reconnect after a failure; failed mutations are not retried automatically because they might already have executed. Errors are reported as `CacheError::Backend`, not as misses. The caller decides whether to recompute from authoritative data. Do not use Redis cache failures to bypass authorization.

## Consistency and security

Redis uses server-side atomic scripts and an exact namespace generation. Namespace invalidation makes the old generation unreachable immediately; the old values expire at their original TTL. A producer with an outdated token cannot publish after a mutation. For bounded metadata, all writes within the namespace advance a shared fence, so unrelated writes can conservatively return `Superseded`. Different namespaces do not share fences. Redis expiry or eviction is a miss; values can disappear at any time. Metadata loss fails closed through fresh random generations/tokens. Restoring an older Redis snapshot or asynchronous failover is not a supported consistency guarantee: use disposable cache data and invalidate after recovery. The first implementation is standalone Redis only, not Cluster or Sentinel.

`get_or_set` coalesces concurrent producers within a single TypedCache instance only. Distributed instances can compute the same value more than once. Neither backend guarantees consistency with an external database transaction; invalidate after commit and handle stale data according to your application. Do not cache passwords, tokens, CSRF values or sensitive session-specific HTML. View and fragment caching remain explicit opt-ins.

The repository's Redis CI uses a disposable local container, independent clients, and a stopped-server test. It does not certify a live production Redis deployment, TLS infrastructure or cluster failover.
