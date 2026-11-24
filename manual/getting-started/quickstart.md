# Quickstart

Create a Fexu **0.2.0** application using the published CLI, apply migrations, create an admin user, and start the development server. For unreleased 0.3.0 development, follow the source instructions in [Installation](installation.md).

Fexu requires Rust **1.89.0+**. New projects default to SQLite (`sqlite://db.sqlite3?mode=rwc`).

## Install the published CLI

```bash
cargo install fexu-cli --version 0.2.0 --locked
fexu --version
```

## Published 0.2.0 generator compatibility

If you use the published 0.2.0 CLI, inspect `src/project/mod.rs` in the generated project. The tagged registry generator omits the `?` from a fallible database URL resolver. Before compiling, use:

```rust
let database_url = settings.resolved_database_url()?;
```

The development generator emits this form directly. The published 0.2.0 packages are unchanged.

## New fullstack project

```bash
mkdir -p ~/fexu-apps && cd ~/fexu-apps
fexu new project demo --template fullstack
cd demo
fexu doctor
fexu check
fexu migrate plan
fexu migrate up
fexu admin create-user --email admin@example.com --staff --superuser
fexu dev
```

On Windows PowerShell, replace paths accordingly:

```powershell
fexu new project demo --template fullstack --fexu-path D:\path\to\fexu-rs
Set-Location demo
fexu migrate up
fexu admin create-user --email admin@example.com --staff --superuser
fexu dev
```

Stop the dev server with Ctrl+C. `fexu dev` reloads supported source and templates; it does **not** run migrations automatically.

## First app and model

```sh
fexu new app catalog
fexu generate model Product --app catalog --timestamps
fexu migrate make add_products --app catalog
fexu migrate plan
fexu migrate up
```

Review generated migrations before `migrate up`. See [models](../topics/models.md) and [migrations](../topics/migrations.md).

## API template

```sh
fexu new project api-demo --template api
```

API projects pin `fexu = "=0.2.0"` without the `admin-ui` feature set.

## PostgreSQL

Set `database.url` in `fexu.toml` or export `DATABASE_URL` with a **disposable** database. Never use production or shared shop databases for migration experiments.

## Next steps

- [First-project tutorial](../tutorial/first-project.md)
- [CLI reference](../reference/cli.md)
- [Upgrade from 0.1.2](../how-to/upgrade-0.1.2-to-0.2.md)
- [Deployment](../how-to/deployment.md)