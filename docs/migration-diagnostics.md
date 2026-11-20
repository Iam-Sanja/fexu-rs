# Migration diagnostics

[Overview](index.md) · Migrations: [migrations.md](migrations.md) · CLI: [cli.md](cli.md)

**Implemented:** `crates/fexu-migrations/src/inspect.rs`. The Cargo version is `0.1.2`.

## Purpose

`fexu migrate status`, `fexu check`, `fexu doctor`, `fexu dev` and `fexu upgrade --check` or `--dry-run` share read-only diagnostics. These checks apply no migrations and create neither history tables nor SQLite files.

Auth and job migrations are embedded in the binary. Their history tables are `_fexu_auth_migrations` and `_fexu_job_migrations`. The diagnostic loads those resources from the process, not from `CARGO_MANIFEST_DIR`. Project migrations use `migrations_dir` and `_fexu_migrations`. An absent or empty application directory is optional and is not treated as a successful auth or jobs migration. Declared `[app.<name>]` packages share the application history table with namespaced IDs such as `shop:0001_initial`. The CLI reads TOML rather than `registry.migrations` and reports this limit as `app.migration.runtime`; startup compares both declarations.

## Report

Each deviation is a `Diagnostic` with range `Auth`, `Jobs` or `App`, migration ID, level `Ok`, `Warn` or `Error`, stable code and text. Applied, pending and conflicted files remain separate.

Stable codes:

| Code | Level | Meaning |
|---|---|---|
| `migration.auth.pending` | Warning |Auth migration is not in `_fexu_auth_migrations`|
| `migration.jobs.pending` | Warning |Job migration is not in `_fexu_job_migrations`|
| `migration.app.pending` | Warning |App file is not in `_fexu_migrations`|
| `migration.history.missing` | Warning | The history table is missing; diagnostics do not create it |
| `migration.checksum_mismatch` | Error |Saved checksum and file do not match|
| `migration.unknown` | Error |An applied ID no longer has a file|
| `migration.order` | Error |The order of files and history is wrong|
| `migration.schema_conflict` | Warning |The loaded model does not correspond to the migration target|
| `migration.auth.table_missing` | Error |An applied auth migration's table is missing, including `_fexu_users`. This is not reported as success and the table is not recreated.|
| `migration.auth.column_missing` | Error |A column required by the latest applied auth snapshot is missing, including `operation_id` after `0003_audit_operation`. The column is not added back.|
| `migration.jobs.table_missing` | Error |An applied job migration's table is missing, including `_fexu_jobs`. This is not reported as success and the table is not recreated.|
| `migration.jobs.column_missing` | Error |A column required by the latest applied job snapshot is missing, including `cron_expression` or `cron_zone` after `0002_job_schedule`. The column is not added back.|
| `migration.database.missing` |Warning in Check, Doctor, Dev and Upgrade. Error in `migrate status`|The SQLite file does not exist|
| `migration.database.unavailable` | Error |The existing file is not a readable database. This is not a success|

`0002_admin_journal` relates to `_fexu_admin_audit` and `_fexu_admin_drafts`. `0003_audit_operation` relates to the column `operation_id`. `0002_job_schedule` relates to `cron_expression` and `cron_zone` on `_fexu_jobs`. The latest applied migration is chosen by migration number, including when the file list is unsorted. Only that snapshot's tables and columns are required. A pending column-only migration does not make its columns required, and a checksum conflict stays visible beside a missing column. As long as these files are only pending, this is a warning and not a silent creation. The check does not repair the schema.

## Reading without writing

SQLite diagnoses open an existing file with `mode=ro`. A missing file does not lead to a `connect`. PostgreSQL is connected to the same URL, the queries are `SELECT`. `ensure_history` is reserved for writing commands.

The URL is `--database-url` on commands that accept it, otherwise `DATABASE_URL`, otherwise `database.url` from `fexu.toml`. `check` and `doctor` have no database override flag. The generated program uses the same environment and file resolution. A relative SQLite path from `fexu.toml` is resolved from the project root. A URL that is calculated only in the application code does not see the CLI. The selected target is printed redacted; `check` and `doctor` do not apply migrations.

## Component readiness in `fexu check`

`fexu check` and `fexu check --deploy` print separate `INFO` summaries after inspecting the existing database. The summaries are read-only and do not create migration history tables, apply migrations, or repair missing tables:

```text
[INFO] migration.readiness.auth state=current pending=0 conflicts=0
[INFO] migration.readiness.jobs state=pending pending=2 conflicts=0
[INFO] migration.readiness.app state=optional-empty pending=0 conflicts=0
```

Each declared application package also has a separate `migration.readiness.<package-name>` line. Counts belong to that component, rather than mixing auth, jobs and app migration histories.

| State | Interpretation |
|---|---|
| `current` | All listed migrations for the component are applied and no conflicts were reported. |
| `pending` | One or more migrations have not been applied. |
| `conflict` | The read-only inspection found an inconsistency, such as missing tables, changed checksums or an invalid history. |
| `missing` | A required component has no migration entries. This must not be confused with a valid optional application directory. |
| `optional-empty` | The application migration set is intentionally empty; this does **not** mean framework auth or jobs migrations have succeeded. |

The readiness lines describe the already validated migration reports. A missing or corrupt embedded framework resource makes inspection fail explicitly instead of producing a successful readiness line. A separate job-status inspection error is reported as `job.migration.unavailable` with an error exit, never as `current`. Other existing diagnostic codes and warnings remain authoritative; an informational readiness line is not a replacement for the final command exit code.

For deployment, run `fexu check --deploy` with access to the **existing** database before bringing the application online. Any pending required migrations or inspection errors block the deploy check. Run `fexu migrate up` separately and explicitly if migrations are needed, then re-run the check. For SQLite, diagnostics use `mode=ro` and do not create the database file; PostgreSQL diagnostics issue read-only queries. A read-only status must never be used as an implicit schema initializer.

## Read-only plan

`fexu migrate plan` keeps the application-directory comparison and, unless `--migrations-dir` selects another directory, also reports:

- dependency errors from declared `[app.<name>]` packages: a missing app, a self-dependency, or a cycle. The message states that no migration was applied when the graph itself says so. No migration file is created.
- pending, checksum, order, and live-schema findings for the embedded auth and job packages. Pending IDs are prefixed `Auth:` and `Job:`. A checksum or order conflict stays an error and skips the live comparison. Extra or missing columns on tables that belong to the latest applied framework snapshot are drift. Unrelated application tables are not treated as framework drift.

`plan_embedded` and `package_dependency_findings` do not call `ensure_history` and do not execute schema SQL. An empty embedded package is an error. A duplicate resource ID or an ID outside the expected auth or jobs list is an error that names the component and the ID; those resources are not applied. An absent or empty application directory remains an optional empty set.

## Runtime contract for issues 74, 75, and 83

Fulfilled:

- Auth migrations `0001_auth_schema` through `0006_account_security` and job migrations `0001_jobs` and `0002_job_schedule` are compiled into the binary. Runtime apply and status do not open `CARGO_MANIFEST_DIR`.
- A relocated `embedded-migrate` binary applies those resources, then calls the public auth and jobs APIs. Linux CI runs `scripts/cold-start-embedded.sh`. Windows CI runs `scripts/cold-start-embedded.ps1`. The official multi-stage example is `examples/deploy/Dockerfile`; the runtime stage copies only the binary.
- Missing, corrupt, incomplete, duplicate, and unexpected required framework resources fail before success. Optional application directories stay empty without that error.
- Fresh and already-migrated SQLite and disposable PostgreSQL databases are covered by library tests. PostgreSQL tests are ignored unless `DATABASE_URL` points at an admin connection that can create a `fexu_test_` database. They drop only that disposable database.
- `migrate plan` is read-only for application files, embedded auth and jobs, and declared app dependencies. Drift is reported. Checksum conflicts are not rewritten as drift.

Additional acceptance evidence on current `main` (unreleased after 0.1.2):

- `crates/fexu-migrations/tests/schema_evolution.rs` exercises a populated initial schema, a read-only plan of the pending next migration, explicit application, schema rename, dry-run and actual down/up, failed unique constraint with unchanged history, recovery using a corrected index migration, and guarded destructive rollback. The SQLite test and the ignored PostgreSQL test use separate disposable databases; PostgreSQL CI explicitly runs the ignored test.
- Both backends verify that `plan_database` identifies an unapplied migration on an older populated schema without inserting history, and detects an unmanaged live column after migrations without repairing it. Read-only inspection does not change existing row counts or history counts.
- A destructive down migration is rejected without `allow_destructive`; on PostgreSQL an explicitly approved rollback drops the selected column while preserving unrelated rows. This is **not** a promise that dropped column values can be recovered after approval.

Still open under issue 83:

- Generated migration operations have focused contract tests for index add/replace/drop, foreign-key add/drop with reversible metadata, supported column conversions, and refused unsafe conversions or new required fields. These are planning-contract tests, **not** proof that every type transition succeeds on both databases. Rename-like column changes are intentionally refused by automatic diffing: use the explicit `fexu migrate rename-column` workflow rather than treating drop + add as lossless.
- The dedicated SQLite and disposable PostgreSQL type-conversion integration tests start with populated string data, reject an invalid string-to-integer conversion without committing history or rewriting rows, then validate the corrected conversion, idempotent reapply and a reversible down migration. PostgreSQL CI explicitly runs its ignored test.
- The SQLite and PostgreSQL FK rollback suite also verifies real populated-row referential behavior: switching a parent/child key from `RESTRICT` to `CASCADE` deletes a dependent row only when its parent is deleted; reverting the FK migration restores `RESTRICT` and protects an existing parent/child pair. The same suite checks that a refused destructive rollback leaves the original row untouched.
- Complete any remaining runtime matrix for other foreign-key changes and supported type conversions beyond string/integer; unsupported transitions must remain explicit rather than guessed.
- The source-tree compatibility suite uses the **exact** `examples/myshop/migrations/0001_initial.json` artifact from the `v0.1.2` Git tag (checked identical to `main`), applies it on fresh SQLite and disposable PostgreSQL, inserts two products, plans/applies a newly generated index migration, verifies no drift and preserved values, then rolls that index migration back without data loss. PostgreSQL CI explicitly runs this additional test. This verifies the historical **migration-file format and data upgrade**, not that the published v0.1.2 CLI or crates can upgrade a generated external application.
- A clean-room published-consumer regression (`scripts/test-published-v012-upgrade.sh`) now installs **fexu-cli 0.1.2 from crates.io** on Ubuntu, generates an independent API project with a catalog model and migration, writes an actual business row, switches the project library dependency to the current Fexu checkout, explicitly ports the v0.1.2-generated `settings.resolved_database_url()` call to the new fallible return type in the temporary source, builds the project, asserts that read-only `upgrade --check` reports the expected `project.file.modified` conflicts with exit code 1 (without changing tracked project files), verifies read-only `migrate plan`, adds a second model/migration with the current CLI, and asserts preserved data and migration history. The Ubuntu CI job requires this test; PostgreSQL runtime evolution remains tested independently by the PostgreSQL job. **This tests a registry-created v0.1.2 SQLite project against the current unreleased checkout, not a published v0.2.0 release candidate.**
- Release sign-off must still repeat a clean-room consumer test against the actual published v0.2.0 packages/CLI, including PostgreSQL where applicable and existing authentication/session data. No published release or production database is modified by this test.
- Keep historical issue acceptance criteria open until fresh Windows/Linux and SQLite/PostgreSQL results are verified against a release candidate. These checks do not move data between databases or silently baseline existing databases.

## Schema

`fexu migrate status` loads the model via `--schema-file` or `schema_command`. `fexu check` does the same after the database file is present. `FEXU_SCHEMA_FILE` replaces the command. `FEXU_CHECK_SKIP_SCHEMA=1` skips it in `check`. `doctor`, `dev` and `upgrade` do not start `schema_command`. Without a loaded schema, the diagnosis explicitly reports this and claims no match.

## Commands

`migrate status` remains successful for pending migrations and names `fexu migrate up`. Conflicts, a missing file and an unreadable database terminate it with Exitcode 1.

`check` terminates warnings only with `--strict` and exit code 2. Without `--strict`, the exit code remains 0 as long as there is no error. `cargo check` then runs. `FEXU_CHECK_SKIP_CARGO=1` omits this step and is intended for testing.

`doctor` explains the same situation and proposes `fexu migrate up`. Warnings do not change its exit code. An unreadable database is an error. An uncreated SQLite file is a warning. Storage routes are not guessed from the source code. A missing storage directory is only a warning if the directory itself is missing.

`dev` prints migration warnings before building. Pending migrations do not prevent startup. If status cannot be read, it reports that failure rather than claiming the database was checked.

## Example

```text
Backend: sqlite

Auth-Migrationen:
  0001_auth_schema  angewendet  checksum …
  0002_admin_journal  ausstehend  checksum …
  0003_audit_operation  ausstehend  checksum …

App-Migrationen:
  0001_initial  angewendet  checksum …

Konflikte: keine
Ausstehende Migrationen: 2
  migration.auth.pending 0002_admin_journal ist ausstehend.
  migration.auth.pending 0003_audit_operation ist ausstehend.

Nächster Schritt:
  fexu migrate up
```

`FEXU_LOCALE=en` switches the headings in English. The codes remain English.

## PowerShell

```powershell
$env:FEXU_LOCALE = "de"
fexu migrate status
fexu check
fexu check --strict
fexu doctor
```
