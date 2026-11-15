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

`CacheError` carries no stored bytes. `Serialization`, `Corrupt`, `VersionMismatch`, `Backend`, `AuthenticatedContent`, `UnauthenticatedViewer`, `Superseded`, `InvalidKey`, `InvalidTtl`, `InvalidLimits` and `TooLarge` keep the payload out of `Display` and `Debug`. `Serialization` is a JSON failure, including `f64::NAN` and a custom serializer error. Zero `max_entries`, zero `max_bytes` and a zero TTL are rejected and nothing is stored. A TTL below one millisecond is stored as one millisecond, so it still expires on the next clock step. The clock is an injectable millisecond function. A clock that jumps backward delays expiry.

## Memory limits

`MemoryLimits` sets `max_entries` and `max_bytes`. The byte count includes the `FXC1` prefix and version. `Default` is 1024 entries and 8 MiB. Callers that do not need that much should pass a smaller limit. The notes example uses 16 entries and 8 KiB.

On insert, expired entries are dropped first. If the new value still does not fit, the backend evicts the earliest expiry, then the oldest insert time, then the key order. A value larger than `max_bytes` returns `TooLarge` and leaves the previous entry in place. Replacing a key frees the old bytes before the new value is counted. Invalidating a namespace frees its bytes for later inserts.

## Stampede, instances and stale data

`get_or_set` elects one producer per `TypedCache` and key. Waiters subscribe to `tokio::sync::Notify` before they read the result, so a producer that finishes immediately cannot drop the wakeup. A producer error is shared with those waiters and is not stored. Dropping the producer task wakes those waiters with `Backend` and stores nothing. The next call may try again. A second `TypedCache` on the same `MemoryCache` shares stored bytes and still runs its own producer. A second `MemoryCache`, or a second process, always runs its own producer.

`MemoryCache` fences a publish against `delete`, `invalidate_namespace` and `set` of the same key. `get_or_set` reads a `WriteToken` before the producer runs. After the producer returns, `put_raw_if_current` checks that token and writes under the same mutex as the delete and invalidation. A producer that started before the cut returns `Superseded`, stores nothing, and wakes its waiters with the same error. A later `get_or_set` reads a new token and may store. A delete or invalidation of a different key or namespace does not block the publish. The mutex is not held while the producer runs, so different keys still compute in parallel.

That fence is a `MemoryCache` guarantee. `CacheBackend` cannot police another backend: `current_token` and `put_raw_if_current` are separate trait methods, and only an implementation that checks and writes under the same exclusion as `delete_raw` and `invalidate_namespace` actually provides it. A Redis backend would need its own transaction or script. This crate does not claim that atomicity for a backend that has not implemented it.

The fence table is process-local. It records namespace and key marks and drops the oldest marks once the table exceeds eight times `max_entries`, at least 64. Dropping the newest mark for a name raises a floor, so an older token fails closed. That can reject an unrelated in-flight publish; the retry stores. It never lets the stale producer publish. A token of `u64::MAX` is rejected because a saturated counter can no longer tell a later invalidation apart. `set` still writes in that theoretical state. There is no cross-process lock, no lease and no broadcast invalidation. Eviction can also drop a live entry when the limit is reached.

## Views and templates

Nothing in `views::render`, Tera or `mount_pages` writes the cache. There is no site-wide cache and no automatic cache of authenticated responses.

`FragmentCache` stores one opted-in string under `view.{name}`. `ViewCache` does not use that namespace. Its anonymous namespace is `a{length}.{name}` and its user namespace is `u{length}.{name}`. The length is two digits. A view name longer than 60 characters is `InvalidKey`, because the derived namespace would pass 64 characters. Dots and underscores stay inside the name. `notes` and `notes.user` therefore never share a store, and an anonymous read cannot return another view's user entry by guessing its key.

- `ViewScope::Anonymous` with `viewer_is_authenticated == false` may store shared HTML.
- The same scope with `viewer_is_authenticated == true` returns `AuthenticatedContent` before lookup or the producer, and does not replace a public entry.
- `ViewScope::User { user_id }` with `viewer_is_authenticated == false` returns `UnauthenticatedViewer` before lookup or the producer. The boolean is not a session proof. The application must pass the real session and the user id that belongs to it. The cache does not read the session.
- `ViewScope::User { user_id }` stores under `u{length}.{name}` with key `u{length}:{user_id}:{key}`. A shared namespace does not isolate tenants; the tests lock that in.
- `invalidate` removes one entry. `invalidate_anonymous` clears only that view's anonymous namespace. `invalidate_users` clears only that view's user namespace. Neither clears `FragmentCache`, another view, or the other scope.

Pass HTML that `Site::render` or `views::render` already produced. Do not cache a response that contains a CSRF token, a flash message for one session, or a `Set-Cookie` value.

`examples/notes/src/cache_page.rs` is the working example. `notes.list` stays uncached. `NoteCache::public_list` and `NoteCache::user_list` opt in. `NoteCache::remember_summary` stores a tenant string only in the namespace the caller names.

## What not to store

The cache does not filter secrets. It will store a password, session token, TOTP seed, recovery code or signed-download key if the caller sends one. Do not send them. Tenant scope is the namespace the caller chooses. Two tenants in `shop` overwrite each other. `user.ada` and `user.bob` do not.

Error text does not repeat the value. That is not a reason to store the value.

## Shared backend

`CacheBackend` is the extension point: `lookup_raw`, `get_raw`, `put_raw`, `delete_raw`, `invalidate_namespace`, `current_token` and `put_raw_if_current`. `get_raw` has a default that collapses `Expired` and `Miss`. The other methods are required. Fexu 0.1 is experimental. Adding `lookup_raw`, `current_token` and `put_raw_if_current`, and adding `UnauthenticatedViewer` and `Superseded`, breaks an out-of-tree `CacheBackend` and an exhaustive `match` on `CacheError`. `get`, `set`, `delete` and `get_or_set` keep their signatures. Inside this workspace only `MemoryCache` implements the trait.

A later Redis crate would implement that trait, store the `FXC1` bytes unchanged, and address keys as a namespace plus a length-prefixed key. It would use the server TTL for `put_raw` and an exact namespace delete for `invalidate_namespace`. It would not treat `tenant` as a prefix of `tenant.a`. `lookup_raw` may return `Miss` for a key Redis has already deleted; `Expired` is guaranteed for `MemoryCache`, not for every backend. The stale-write fence is also not implied: that crate must check `WriteToken` and write in one Redis transaction or script.

That crate is not part of this tree. A Redis client would add a network service and a deployment requirement, and the process-local default would still be the one that must not be described as shared. No Redis dependency is declared.

## Known limits

- Two server processes do not share entries, invalidations or stampede protection.
- Two `TypedCache` values do not share stampede protection, even when they clone one `MemoryCache`.
- `get` collapses `Miss` and `Expired`. Use `lookup` when the difference matters.
- A version mismatch stays until an explicit delete, overwrite or expiry. It does not refresh itself.
- The cache does not encrypt values and does not refuse secret-shaped strings.
- Authenticated HTML stays out of the anonymous scope only when the caller uses `ViewCache` and passes `viewer_is_authenticated == true` for that viewer. A false flag on `ViewScope::User` is rejected, but a true flag is not proof that the session or the user id is real.
- User isolation is the encoded user id inside that view's user namespace. The cache does not check that the id belongs to the session.
- There is no per-namespace quota, only the backend's global entry and byte limits.
- Eviction drops live entries. TTL and invalidation are the caller's consistency tools.
- A backward clock jump keeps entries until the clock passes `expires_at`.
- A poisoned mutex is recovered. A panic inside the locked section can leave that process-local map inconsistent; the next process still starts empty.
- No HTTP `Cache-Control`, ETag or CDN behavior is implemented here. Static files set their own `max-age` in `fexu-static`.

## Tests

These tests do not open an application database. `cargo test -p fexu-cache` covers TTL boundaries, a one-millisecond expiry, zero TTL, zero limits, eviction, `TooLarge`, miss versus expiry, exact namespaces, key-pair collisions, user-key collisions, unsafe keys, version errors, corrupt payloads, serialization that must not echo the value, secrets left in the caller's namespace, a shared namespace mixing tenants, separate `MemoryCache` instances, cloned memory with two stampede locks, concurrent `get_or_set`, coalesced producer errors, a delete or namespace invalidation during a producer, an aborted producer, a newer `set` during a producer, fragment invalidation, dotted view names, and both invalid view-scope flags.

`cargo test -p notes --test cache_page` runs the notes example: tenant miss and hit, tenant invalidation, a public page cached once, an authenticated viewer refused, a user page isolated from another user and from the public page, and a second `NoteCache` that does not see the first.

`cargo test -p notes --test jobs` still checks that two separately constructed caches do not share a notes entry.
