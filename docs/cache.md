# Cache

[Overview](index.md) · **Implemented files:** `crates/fexu-cache`, `examples/notes/src/cache_page.rs`.

**Implemented** as an optional, process-local cache. The facade re-exports it as `fexu::cache`. There is no Redis client and no required extra service. An in-process cache is not a distributed cache.

`MemoryCache::new` and `MemoryCache::with_now` each own a store. A second value does not see the first, and a second OS process does not either. `MemoryCache` is `Clone` because the store sits behind `Arc`: clones share bytes inside that one process only. Cloning the handle does not share the stampede lock. That lock lives on one `TypedCache`. Share `Arc<TypedCache<_>>` when concurrent `get_or_set` must run the producer once.

## API

`TypedCache::new(backend, version)` offers `lookup`, `get`, `set`, `delete`, `invalidate_namespace` and `get_or_set`.

| Call | Hit | Missing | Expired | Bad bytes |
|---|---|---|---|---|
| `lookup` | `Hit(value)` | `Miss` | `Expired`, then the entry is removed | `VersionMismatch` or `Corrupt` |
| `get` | `Some(value)` | `None` | `None` | same errors as `lookup` |

A second `lookup` after `Expired` is `Miss`. `get` does not say which of the two empty cases happened. Expiry is applied before JSON decoding, so an expired corrupt payload is `Expired`, not `Corrupt`. `delete` of a missing key succeeds. `invalidate_namespace` removes that exact namespace and frees its bytes. `tenant` does not clear `tenant.a`.

Keys are the pair `(namespace, key)`, not a concatenated string. `validate_namespace` allows 1 to 64 ASCII characters: a letter or digit, then letters, digits, `_`, `-` and `.`. `validate_key` allows 1 to 180 ASCII graphic characters except space, `\` and `..`. The same rules are public so a later backend does not invent a second spelling. A user-scoped view key is `u{length}:{user_id}:{key}`. The length makes `a.b` + `c` different from `a` + `b.c`.

Values are JSON with an `FXC1` prefix and a big-endian `u32` version. A version mismatch or corrupt bytes is an error. The cache does not recompute those entries. `get_or_set` returns the error and does not call the producer. Bump the version only together with `delete` or `invalidate_namespace`.

`CacheError` carries no stored bytes. `Serialization`, `Corrupt`, `VersionMismatch`, `Backend`, `AuthenticatedContent`, `InvalidKey`, `InvalidTtl`, `InvalidLimits` and `TooLarge` keep the payload out of `Display` and `Debug`. `Serialization` is a JSON failure, including `f64::NAN` and a custom serializer error. Zero `max_entries`, zero `max_bytes` and a zero TTL are rejected and nothing is stored. A TTL below one millisecond is stored as one millisecond, so it still expires on the next clock step. The clock is an injectable millisecond function. A clock that jumps backward delays expiry.

## Memory limits

`MemoryLimits` sets `max_entries` and `max_bytes`. The byte count includes the `FXC1` prefix and version. `Default` is 1024 entries and 8 MiB. Callers that do not need that much should pass a smaller limit. The notes example uses 16 entries and 8 KiB.

On insert, expired entries are dropped first. If the new value still does not fit, the backend evicts the earliest expiry, then the oldest insert time, then the key order. A value larger than `max_bytes` returns `TooLarge` and leaves the previous entry in place. Replacing a key frees the old bytes before the new value is counted. Invalidating a namespace frees its bytes for later inserts.

## Stampede, instances and stale data

`get_or_set` elects one producer per `TypedCache` and key. Waiters subscribe to `tokio::sync::Notify` before they read the result, so a producer that finishes immediately cannot drop the wakeup. A producer error is shared with those waiters and is not stored. The next call may try again. A second `TypedCache` on the same `MemoryCache` shares stored bytes and still runs its own producer. A second `MemoryCache`, or a second process, always runs its own producer.

There is no cross-process lock, no lease and no broadcast invalidation. Stale data is the caller's problem. Set a TTL or call `delete` / `invalidate_namespace` after a write. Eviction can also drop a live entry when the limit is reached.

## Views and templates

Nothing in `views::render`, Tera or `mount_pages` writes the cache. There is no site-wide cache and no automatic cache of authenticated responses.

`FragmentCache` stores one opted-in string under `view.{name}`. `ViewCache` is the view and template helper:

- `ViewScope::Anonymous` with `viewer_is_authenticated == false` may store shared HTML.
- The same scope with `viewer_is_authenticated == true` returns `AuthenticatedContent`, does not call the producer, and does not replace a public entry.
- `ViewScope::User { user_id }` stores under `view.{name}.user`. The caller must pass the authenticated user's id. The cache does not read the session. A shared namespace does not isolate tenants; the tests lock that in.
- `invalidate` removes one entry. `invalidate_anonymous` clears `view.{name}` and does not clear user entries. `invalidate_users` does the opposite.
- `FragmentCache` and anonymous `ViewCache` entries share `view.{name}` when the name matches. Invalidating one clears the other.

Pass HTML that `Site::render` or `views::render` already produced. Do not cache a response that contains a CSRF token, a flash message for one session, or a `Set-Cookie` value.

`examples/notes/src/cache_page.rs` is the working example. `notes.list` stays uncached. `NoteCache::public_list` and `NoteCache::user_list` opt in. `NoteCache::remember_summary` stores a tenant string only in the namespace the caller names.

## What not to store

The cache does not filter secrets. It will store a password, session token, TOTP seed, recovery code or signed-download key if the caller sends one. Do not send them. Tenant scope is the namespace the caller chooses. Two tenants in `shop` overwrite each other. `user.ada` and `user.bob` do not.

Error text does not repeat the value. That is not a reason to store the value.

## Shared backend

`CacheBackend` is the extension point: `lookup_raw`, `get_raw`, `put_raw`, `delete_raw` and `invalidate_namespace`. A later Redis crate would implement that trait, store the `FXC1` bytes unchanged, and address keys as a namespace plus a length-prefixed key. It would use the server TTL for `put_raw` and an exact namespace delete for `invalidate_namespace`. It would not treat `tenant` as a prefix of `tenant.a`. `lookup_raw` may return `Miss` for a key Redis has already deleted; `Expired` is guaranteed for `MemoryCache`, not for every backend.

That crate is not part of this tree. A Redis client would add a network service and a deployment requirement, and the process-local default would still be the one that must not be described as shared. No Redis dependency is declared.

## Known limits

- Two server processes do not share entries, invalidations or stampede protection.
- Two `TypedCache` values do not share stampede protection, even when they clone one `MemoryCache`.
- `get` collapses `Miss` and `Expired`. Use `lookup` when the difference matters.
- A version mismatch stays until an explicit delete, overwrite or expiry. It does not refresh itself.
- The cache does not encrypt values and does not refuse secret-shaped strings.
- Authenticated HTML is excluded only when the caller uses `ViewCache` and tells the truth in `viewer_is_authenticated` or `ViewScope::User`.
- User isolation is the encoded user id. The cache does not check that the id belongs to the session.
- There is no per-namespace quota, only the backend's global entry and byte limits.
- Eviction drops live entries. TTL and invalidation are the caller's consistency tools.
- A backward clock jump keeps entries until the clock passes `expires_at`.
- A poisoned mutex is recovered. A panic inside the locked section can leave that process-local map inconsistent; the next process still starts empty.
- No HTTP `Cache-Control`, ETag or CDN behavior is implemented here. Static files set their own `max-age` in `fexu-static`.

## Tests

These tests do not open an application database. `cargo test -p fexu-cache` covers TTL boundaries, a one-millisecond expiry, zero TTL, zero limits, eviction, `TooLarge`, miss versus expiry, exact namespaces, key-pair collisions, user-key collisions, unsafe keys, version errors, corrupt payloads, serialization that must not echo the value, secrets left in the caller's namespace, a shared namespace mixing tenants, separate `MemoryCache` instances, cloned memory with two stampede locks, concurrent `get_or_set`, coalesced producer errors, fragment invalidation, and the authenticated-view refusal.

`cargo test -p notes --test cache_page` runs the notes example: tenant miss and hit, tenant invalidation, a public page cached once, an authenticated viewer refused, a user page isolated from another user and from the public page, and a second `NoteCache` that does not see the first.

`cargo test -p notes --test jobs` still checks that two separately constructed caches do not share a notes entry.
