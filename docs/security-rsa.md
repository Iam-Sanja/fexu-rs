# RSA advisory RUSTSEC-2023-0071

`RUSTSEC-2023-0071` matched `rsa` 0.9.10 because the `sqlx` 0.8.6 facade recorded optional `sqlx-mysql`. Fexu no longer depends on the `sqlx` facade or on `sqlx-mysql`. `Cargo.lock` does not contain `rsa`.

Checked again on 2026-11-17 in this worktree:

- `Cargo.lock` has no package named `rsa` and no `sqlx-mysql`. The workspace depends on `sqlx-core` 0.8.6, `sqlx-postgres` 0.8.6 and `rusqlite` 0.42.0. MySQL is not enabled.
- `cargo tree --workspace --target all --prefix none --locked --offline -i rsa` exits 101 with `package ID specification rsa did not match any packages`. The same command for `sqlx-mysql` and `sqlx` matches nothing. That is absence from the resolved graph, not a compiled feature that was switched off inside a present crate.
- The full workspace tree has no line matching `^rsa v`. The security workflow uses that grep and fails if a future target enables RSA.
- `cargo audit` 0.23.0 loaded 1527 advisories and scanned 360 lockfile crates. It did not report `RUSTSEC-2023-0071` and exited 0. It printed three allowed unmaintained warnings: `chrono` (`RUSTSEC-2026-0543`), `chrono-tz` (`RUSTSEC-2026-0555`) and `chrono-tz-build` (`RUSTSEC-2026-0559`). Those warnings are not this advisory.

SQLite and PostgreSQL stay. SQLx stays at 0.8.6. SQLx 0.9 is not the fix for this advisory.

The security workflow runs `cargo audit` 0.22.2 without an ignore for `RUSTSEC-2023-0071`. Do not add an ignore to make that check pass. If `rsa` returns, fix the dependency or record a new, reviewed exception. The old exception is not still in force.
