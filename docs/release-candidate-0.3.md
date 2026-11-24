# Fexu 0.3.0 — development and release acceptance

**Planning baseline:** `8ac4b50` (2026-11-24). Published version: 0.2.0. Development version: 0.3.0-dev.1. This is a plan, not a release approval or test report.

## Development gates

- [ ] Validate the coordinated 16-crate version and 23 workspace package lock entries with Cargo; leave third-party resolutions unchanged. Keep the default registry generator pinned to published 0.2.0.
- [ ] Run `cargo fmt --all -- --check`, `cargo check --workspace --all-targets --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, `cargo +1.89.0 check --workspace --all-targets --locked`, and `cargo audit`.
- [ ] Pass Windows/Linux, PostgreSQL ignored tests, browser tests and documentation gates; review actual results, not only source presence.

## New published 0.2.0 upgrade acceptance (#92, #93)

- [ ] Install the exact published 0.2.0 CLI in a disposable clean-room directory; generate API and fullstack projects with registry dependencies. Do not substitute a source-only fixture for the published baseline.
- [ ] Seed real rows, relationships, migration history, users, roles, password hashes, sessions and representative media on isolated SQLite and PostgreSQL databases.
- [ ] Switch only the disposable copies to matching candidate source via an explicit path; retain required feature flags, secrets and recorded migration files. Verify the read-only upgrade and migration plan before applying intended migrations.
- [ ] Confirm data, authorization, existing passwords and sessions survive; test admin login, CSRF, failed migration and rollback/recovery boundaries. Record candidate SHA and exact versions; never modify production data.
- [ ] Keep the existing 0.1.2 regression suite in addition to the new 0.2.0 acceptance.

## Production foundations

- [ ] #94: readiness, shutdown, redacted logging, least privilege, isolated backup/restore drills.
- [ ] #96: local and optional S3 storage lifecycle, authorization, quotas and recovery.
- [ ] #87/#88: worker restart, at-least-once delivery, optional shared cache and consistency tests.
- [ ] #85/#172–174: opt-in OIDC, explicit account linking and mandatory negative/mock/browser tests.
- [ ] #86/#148: minimum reusable-app and locale contracts with runnable examples.
- [ ] #80/#84/#91: acceptance of existing admin/forms and clean-room CLI workflow.

## Final release gates

- [ ] Review all remaining milestone issues and explicitly defer unfinished scope; do not equate an implementation with complete acceptance.
- [ ] Update the upgrade manual, README, changelog, API documentation and tagged-version documentation plan. Verify examples and distinguish development from registry availability.
- [ ] Coordinate all packages at final stable 0.3.0 and run the existing tokenless release preflight plus the complete CI on the final SHA. The stable-only publication planner remains unchanged.
- [ ] Obtain explicit maintainer approval before creating or publishing a release. Preserve the v0.2.0 tag. No release/tag is created by this plan.

See [roadmap](roadmap.md), [release policy](crates-io-release.md) and [documentation maintenance](documentation-maintenance.md).
