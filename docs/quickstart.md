# Quickstart

This page lists the commands. A step is marked as run only when this repository's tests actually ran it.

`fexu dev` and `fexu doctor` do not apply migrations. `fexu migrate up` is the command that writes schema changes, and it does not reset a database. There is no default admin password.

## Published CLI

Use this when the crates.io packages are already published. This working tree did not run `cargo install fexu-cli` against crates.io.

```powershell
cargo install fexu-cli --version 0.1.2 --locked
fexu new project demo
Set-Location demo
fexu doctor
fexu migrate up
fexu admin create-user --email admin@example.com --staff --superuser
fexu dev
```

`fexu new project` writes `fexu = { version = "=0.1.2", features = ["admin-ui"] }` for the fullstack template. That dependency comes from crates.io. The automated test `clean_room_project_has_no_local_fexu_path` checks the generated files and `fexu doctor` without a local framework checkout. It does not download the published crate or start the server.

The same published install on Linux and macOS uses a POSIX shell. This tree did not run these commands against crates.io either.

```bash
cargo install fexu-cli --version 0.1.2 --locked
fexu new project demo
cd demo
fexu doctor
fexu migrate up
fexu admin create-user --email admin@example.com --staff --superuser
fexu dev
```

## Local checkout

Use this while changing Fexu itself. The generator tests in this repository do run this path, including compile, `migrate up`, app and model generation, and a short `fexu dev`.

```powershell
cargo install --path crates/fexu-cli
$repo = (Get-Location).Path
Set-Location ..
fexu new project demo --template fullstack --fexu-path $repo
Set-Location demo
fexu migrate up
fexu dev
```

`--fexu-path` must be the workspace root or `crates/fexu`. Omit it to use crates.io.

## SQLite

New projects set `[database] url` to `sqlite://db.sqlite3?mode=rwc`. No database server is required. `fexu doctor` does not create a missing SQLite file. Create it by running `fexu migrate up` when you intend to write the schema.

## PostgreSQL

Set `DATABASE_URL` or `[database] url` to a `postgres://` URL only for an application that needs PostgreSQL. The value is not printed by `doctor` or `check`. This quickstart did not start a PostgreSQL server. Workspace CI runs the existing ignored PostgreSQL tests against its own disposable service. Do not point these commands at a production database.

## Development and production

`fexu dev` is the development process. It rebuilds after `.rs`, `.toml`, `.json`, `.html` and `.css` changes, stops the previous process before starting another one, and does not migrate. `fexu check --deploy` is read-only and expects production settings such as a 64-character hex `FEXU_CSRF_SECRET`. It does not print that secret. `fexu build` writes a release binary and does not start it.

## When a command fails

| Symptom | Cause and fix |
|---|---|
| `No fexu.toml found` | The working directory is not a project. Run `fexu new project` or `cd` into one. |
| `project.file.missing` | `Cargo.toml`, `src/main.rs` or `src/lib.rs` is gone. Restore the generated file. |
| `migrations.missing` | The migrations directory is gone. Recreate it. `doctor` will not migrate. |
| `templates.missing` | `templates.enabled` is true and the directory is absent. Create it or disable templates. |
| `migration.database.missing` | The SQLite file is not there. Run `fexu migrate up` only when you want it created. |
| `migration.database.unavailable` | The configured database did not answer. Fix the existing URL. The URL is not shown. |
| `Port ... is already in use` | Another process holds `server.port`. Stop it or change the port. `dev` does not start a second server. |
| Invalid `FEXU_CSRF_SECRET` | The value is not 64 hexadecimal characters. Replace it. The current value is not shown. |
