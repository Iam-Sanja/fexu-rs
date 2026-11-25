# Fexu 0.3.0 — development and release acceptance

**Planning baseline:** `8ac4b50` (2026-11-24). Published version: 0.2.0. Development version: 0.3.0-dev.1. This is a plan, not a release approval or test report.

## Development gates

- [ ] Validate the coordinated 16-crate version and 23 workspace package lock entries with Cargo; leave third-party resolutions unchanged. Keep the default registry generator pinned to published 0.2.0.
- [ ] Run `cargo fmt --all -- --check`, `cargo check --workspace --all-targets --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, `cargo +1.89.0 check --workspace --all-targets --locked`, and `cargo audit`.
- [ ] Pass Windows/Linux, PostgreSQL ignored tests, browser tests and documentation gates; review actual results, not only source presence.

## New published 0.2.0 upgrade acceptance (#92, #93)

The first CI slice extends the existing fullstack fixture to the actual registry release 0.2.0 on both backends, with a documented one-line repair of the historical generator before baseline compilation. It also checks fresh API/fullstack registry scaffolding with the development CLI. Both new fullstack upgrade jobs passed on PR #262 (CI run 45380382399); the PR merged as `1703161`. Existing 0.1.2 regressions passed, including identity and privilege checks with the original session cookie. Media and broader API upgrade acceptance remain separate tasks.

- [ ] Install the exact published 0.2.0 CLI in a disposable clean-room directory; generate API and fullstack projects with registry dependencies. Do not substitute a source-only fixture for the published baseline.
- [ ] Seed real rows, relationships, migration history, users, roles, password hashes, sessions and representative media on isolated SQLite and PostgreSQL databases.
- [ ] Switch only the disposable copies to matching candidate source via an explicit path; retain required feature flags, secrets and recorded migration files. Verify the read-only upgrade and migration plan before applying intended migrations.
- [ ] Confirm data, authorization, existing passwords and sessions survive; test admin login, CSRF, failed migration and rollback/recovery boundaries. Record candidate SHA and exact versions; never modify production data.
- [x] Keep the existing 0.1.2 regression suite in addition to the new 0.2.0 acceptance (PR #262; rerun on final candidate).

## Phase 1 acceptance reviewed (2026-11-24)

- #74 completed: relocated binary, hidden source resources, authentication and job enqueue on SQLite and PostgreSQL; Linux/Windows CI; all eight migration JSON files identical to v0.2.0. Actual Docker deployment remains #78/#94.
- #77 completed: direct strict native admin login, cookie-state and CSRF/origin matrix on SQLite and PostgreSQL; documented in admin-security.md.
- #19 closed as not planned: pre-Fexu naming aliases and database conversion were deliberately retired, not implemented. Published Fexu upgrades remain #92/#93.

## Production foundations

- [ ] #94: readiness, shutdown, redacted logging, least privilege, isolated backup/restore drills. PR #264 passed actual Compose HTTPS, runtime role, DB outage, normal SIGTERM exit, volume/session persistence and real PostgreSQL restore. The next slice adds stricter deployment checks and structured access-log regression coverage; these new results remain pending. Worker outage, media restore, checkout recovery and live LAN/NPM remain separate.
- [ ] #96: local and optional S3 storage lifecycle, authorization, quotas and recovery. First slice shares upload validation, rejects unsupported staging deadlines, preserves old files after retain failures and validates HTTPS S3 origins. #270 passed all 13 checks. #271 passed all 14 checks, including optional S3 network client, atomic envelope, conditional retain, TLS MinIO, dependency lock/MSRV and security. #272 passed explicit retain/delete race and persistent-container restart tests. Targeted fenced expiry reconciliation is the next slice (pending CI); quotas, reference coordination, outage and backup/restore remain follow-up work.
- [ ] #87/#88: #266 and #267 passed lease-expiry, attempt-budget, Unix SQLite process crash/restart/SIGTERM and PostgreSQL handler overlap acceptance. #268 passed all 13 checks, including Redis shared/outage tests on Rust 1.89 and security/lockfile verification. #269 passed all 13 checks, including connection reuse, same-handle restart, concurrent callers and both TLS trust cases. #88 closed; #87 remains open for remaining acceptance.
- [ ] #85/#172–174: opt-in OIDC, explicit account linking and mandatory negative/mock/browser tests.
- [ ] #86/#148: minimum reusable-app and locale contracts with runnable examples.
- [ ] #80/#84/#91: acceptance of existing admin/forms and clean-room CLI workflow.

## Final release gates

- [ ] Review all remaining milestone issues and explicitly defer unfinished scope; do not equate an implementation with complete acceptance.
- [ ] Update the upgrade manual, README, changelog, API documentation and tagged-version documentation plan. Verify examples and distinguish development from registry availability.
- [ ] Coordinate all packages at final stable 0.3.0 and run the existing tokenless release preflight plus the complete CI on the final SHA. The stable-only publication planner remains unchanged.
- [ ] Obtain explicit maintainer approval before creating or publishing a release. Preserve the v0.2.0 tag. No release/tag is created by this plan.

See [roadmap](roadmap.md), [release policy](crates-io-release.md) and [documentation maintenance](documentation-maintenance.md).
