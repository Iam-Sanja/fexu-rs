# Fexu 0.2.0 release candidate: controlled preflight

Issue [#92](https://github.com/Iam-Sanja/fexu-rs/issues/92) is the publication tracker; [#93](https://github.com/Iam-Sanja/fexu-rs/issues/93) covers independent acceptance. **This document is a proposed gate, not a release authorization.** The currently published crates.io version is **0.1.2**. The unpublished `main` checkout declares **`0.2.0-dev.1`**, not an approved `0.2.0` release. Do not call it released 0.2.0.

## Release contract

- All **16** publishable workspace crates are versioned and released together. Every internal path dependency must also declare the candidate registry version. The dependency-order planner is `scripts/publish-workspace.py --version VERSION --plan`; it refuses mismatched versions, invalid dependency requirements and cycles. This does **not** upload anything.
- Pre-1.0 minor releases can have breaking source/API changes. Upgrading from 0.1.2 may require explicit edits to generated application Rust sources (for example the now-fallible `resolved_database_url()?`). Never advertise that `fexu upgrade` automatically rewrites application sources, resets databases or preserves an incompatible binary format.
- Migration histories, user data, passwords and sessions must remain intact unless a separately reviewed, explicitly documented migration is necessary. Schema/file checksums must not be silently resealed or baselined.
- A release of crates to crates.io is **irreversible**: published versions cannot be replaced. Rolling back an application deploy means restoring a previously approved binary and compatible data/configuration; it does not mean unpublishing crates or reversing arbitrary destructive migrations. Take and verify backups before any migration.

## Candidate workflow (no publishing)

1. Create a focused final release PR that promotes `0.2.0-dev.1` to `0.2.0` across the workspace version and *all* internal crate dependency requirements, updates `Cargo.lock`, the CLI generator pins, generated examples and changelog. Review the version diff separately from feature development.
2. On that candidate branch, manually run **Release candidate preflight (no publishing)** from GitHub Actions with the exact Cargo version (e.g. `0.2.0`). The workflow validates dependency order and package metadata, checks formatting/lints/tests, builds Linux release binaries, and lists the actual packaged files for all 16 crates. No crates.io token, tag or GitHub Release is used.
3. Require the regular **10 green checks** on the release PR, including PostgreSQL, Windows, browser and Cargo audit. Verify the final candidate commit exactly matches the commit that received those checks.
4. Run independent clean-room generated consumer upgrades from the **published 0.1.2 CLI and crates** to the candidate checkout on both SQLite and disposable PostgreSQL. Exercise real prior application migration files and rows, successful *and rejected* schema evolutions, and preserved existing auth accounts, passwords, sessions and admin access. Assert no unnoticed history/row changes from read-only commands. Current `scripts/test-published-v012-upgrade.sh` covers the SQLite CLI/source/migration side, and `scripts/test-published-v012-auth-upgrade.sh` validates historical auth/session continuity on SQLite. The new `scripts/test-published-v012-postgres-auth-upgrade.sh` exercises published-to-source auth/session continuity on isolated PostgreSQL, but it must pass CI and does **not** yet cover a full generated PostgreSQL project, HTTP admin/browser behavior, or the exact final candidate.
5. Validate relocation and a minimal multi-stage Docker cold start, release package inventory, SQLx/SQLite dependencies and security advisories, private asset/secret review, platform installation instructions, and reproducible Linux build instructions. Record limitations and actual commands/results with candidate SHA, tool versions and test database provenance.
6. Obtain **explicit maintainer approval** for version/tag and publishing. Only then use the existing `.github/workflows/publish.yml` with an approved release tag pointing to the current main HEAD. The publishing workflow uses short-lived OIDC credentials and uploads in dependency order; environment reviewers, if needed, must be configured separately.

## Evidence checklist

| Gate | Current evidence | Release status |
|---|---|---|
| 16-crate publication graph, metadata and planner | `docs/crates-io-release.md`, `scripts/publish-workspace.py` | Implemented for 0.1.2; rerun for candidate |
| Main Linux, Windows, PostgreSQL, browser, audit | `.github/workflows/ci.yml` | Require new candidate SHA |
| Published 0.1.2 CLI -> checkout (SQLite) | `scripts/test-published-v012-upgrade.sh` | Existing, partial |
| Historical migration artifact -> checkout (SQLite + PostgreSQL) | `crates/fexu-migrations/tests/schema_evolution.rs` | Existing, not external sessions |
| Published 0.1.2 consumer -> 0.2.0 (PostgreSQL) | Not yet established | **Blocked** |
| Persisted auth/admin identity, role and session continuity across reconnect (current migrations) | `crates/fexu-auth/src/session_upgrade_tests.rs` (SQLite and disposable PostgreSQL) | Required CI regression; **not a historical binary upgrade** |
| Existing user/admin login and session continuity across actual published 0.1.2 -> candidate upgrade | `scripts/test-published-v012-auth-upgrade.sh`: separate published auth/db 0.1.2 and development-source binaries on disposable SQLite; first CI pass pending | **Partial — PostgreSQL, final candidate and fullstack still blocked** |
| Candidate version bump, locked dependencies, packaged contents | Manual preflight workflow | Not yet run for candidate |
| Verified final changelog, install instructions, rollback plan | This document and release checklist | Maintainer sign-off pending |
| Approved tag/GitHub Release, crates.io publication | Never authorized by this document | **Not started** |

## Public upgrade documentation

The current [development-source upgrade guide](../manual/how-to/upgrade-0.1.2-to-0.2.md) explains the verified published 0.1.2 SQLite API-project regression, its exact source edit, and missing PostgreSQL/fullstack/auth-session evidence. It is not a proof that a complete production migration is supported. A version-specific guide must be rechecked against the final candidate before release.

## Installation instructions to verify with actual released version

Windows PowerShell and Linux/macOS shell share the Cargo commands, provided a supported Rust toolchain and native build prerequisites are installed:

```sh
cargo install fexu-cli --version 0.1.2 --locked
cargo add fexu@0.1.2
```

Those commands explicitly refer to the **existing published 0.1.2** version. Do **not** use `cargo install fexu-cli --version 0.2.0` or `cargo add fexu@0.2.0` in instructions until that version has been published and independently installed from crates.io. Source-checkout testing is a separate workflow.

## Rollback constraints

Before applying migrations to any existing application, save a tested DB backup, schema/migration history, session-secret material, environment configuration and uploaded media. A pre-release candidate must exercise restore in a disposable environment. Do not change TestShop, live PostgreSQL, or deployment services to perform release acceptance. Destructive database migrations need separate acknowledgement and can make an old binary incompatible even if the old image is available.

The `publish.yml` workflow is intentionally **not** triggered by the preflight. Finishing the preflight never publishes, tags or edits a release.
