# Upgrade from Fexu 0.1.2 to the 0.2.0 development source

!!! warning "Source upgrade — not automatic"
    Published crates.io is still **0.1.2** until authorize publish. Workspace **0.2.0** requires source edits or path dependencies; `fexu upgrade` does **not** fully rewrite application code or guarantee production migration.

## Before you upgrade

Fexu is experimental. A pre-1.0 minor release may require edits to application Rust sources. Do **not** run the steps against a production database. Use an isolated copy of the existing application with a **disposable** database and take and verify backups of its data, schema/migration history, configured session secrets, and uploaded media. Keep the old binary and exact source revision available for a rollback test.

Do not delete migration files that were already applied. Do not use `migrate reseal` or `migrate baseline` to hide differences between historical migration files and recorded checksums. Never assume that a binary rollback will reverse an incompatible schema change.

## What has actually been tested

The repository includes `scripts/test-published-v012-upgrade.sh`, which:

1. Installs **published** `fexu-cli 0.1.2` from crates.io in an isolated temporary directory.
2. Generates an API project with the **published** CLI, creates a `catalog` app, generates a `Product` model and applies its initial SQLite migration.
3. Inserts an actual persisted product row, switches the generated application's `fexu` dependency to the current **source checkout**, and explicitly adapts a generated Rust API call.
4. Runs `cargo check`, confirms that `fexu upgrade --check` reports tracked-file modifications without writing files, verifies that `fexu migrate plan` does not mutate the database, and applies a new `Customer` model migration.
5. Checks that the original product row, historical migration chain and the new table remain present.

Additional acceptance scripts on branch `release/020-final-acceptance` (CI green @ `d76dab7`) cover:

- Published 0.1.2 **auth** consumer → source on SQLite and disposable PostgreSQL.
- Published 0.1.2 **fullstack** consumer → source on SQLite and disposable PostgreSQL (HTTP login, sessions, roles, catalog data, failed migration guard).

These are **regression fixtures**, not a promise that every production app upgrades without manual Rust edits. The API SQLite script above remains the minimal catalog/product proof.

## Compare a published project with your source checkout

First check the **old** generated project's `Cargo.toml`. Published CLI 0.1.2 generates a registry dependency pinned to `fexu = "=0.1.2"` for API projects (and an `admin-ui` feature dependency for fullstack projects). It must not be silently interpreted as source `main`.

To assess unreleased changes, obtain a local checkout of the framework and use a **disposable project copy**. Replace the project's existing Fexu dependency with an explicit local path:

```toml
# Example for an API-only project; adapt the path to your source checkout.
fexu = { path = "../fexu-rs/crates/fexu" }
```

For a fullstack project, preserve the existing `admin-ui` feature:

```toml
fexu = { path = "../fexu-rs/crates/fexu", features = ["admin-ui"] }
```

These are source-checkout examples, not instructions for installing `0.2.0` from crates.io. Use a real relative or absolute path to your local framework checkout. The path must resolve from your **application's** `Cargo.toml`.

## One confirmed source-level compatibility change

In the published 0.1.2 generated API project, `settings.resolved_database_url()` was used as though it directly returned a string. The development implementation returns a `Result` that must be handled.

**File:** `src/project/mod.rs` inside the copied generated application.

Old published generator code:

```rust
let db = fexu::db::connect(&settings.resolved_database_url()).await?;
```

Verified adaptation in the repository's SQLite upgrade regression:

```rust
let database_url = settings.resolved_database_url()?;
let db = fexu::db::connect(&database_url).await?;
```

This is one **known example**, not an exhaustive source-compatibility list for every feature or template. Compile your own application and review any further errors against the matching development source. Do not assume `fexu upgrade` edits these Rust files for you: that CLI command inspects legacy project layout/manifest state.

## Read-only checks before schema changes

Inside the disposable, adapted project:

```sh
cargo check
fexu upgrade --check
fexu migrate plan
```

Use the CLI built from the same Fexu checkout as the source dependency if you are evaluating development behavior. A `--check` warning about modified generated files can be **expected** after manual source edits; inspect it rather than attempting to overwrite your work.

The repository test explicitly verifies that the SQLite database checksum is unchanged after its `migrate plan` command. That check applies to the tested fixture, not to arbitrary third-party projects or every migration.

After reviewing the plan and verifying a backup, apply intended migrations **only in a disposable environment**:

```sh
fexu migrate up
fexu migrate status
cargo check --all-targets
```

Do not use destructive migration options merely to make a failed plan pass.

## Required release acceptance before recommending 0.2.0 upgrades

The following are **not yet proven** by the existing published-to-source SQLite regression:

- A published 0.1.2 **fullstack** project's prior user and staff/admin accounts still authenticate against the exact candidate build.
- Existing session identities, role assignments, permissions and login state survive a real old-binary-to-new-binary transition with the intended session-secret configuration.
- A **separate PostgreSQL consumer** migrates its real historical schema and data without drift or data loss.
- Rejected and accepted schema evolutions, rollback, browser login, admin access and CSRF behavior pass on the final candidate SHA.
- Windows authentication stability is understood sufficiently to approve the candidate.

The release team must run these tests with disposable data on the **final pinned candidate**, record exact versions and results, and obtain explicit maintainer sign-off. Until then, the release remains **NO-GO**.

See [documentation versions](../versions.md), [migrations](../topics/migrations.md), [authentication and admin](../topics/auth-and-admin.md), and the repository [release candidate gate](https://github.com/Iam-Sanja/fexu-rs/blob/main/docs/release-candidate-0.2.md).
