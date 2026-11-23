# Quickstart

Create a Fexu **0.2.0** application from a source-built CLI, apply migrations, create an admin user, and start the development server. These commands were verified against the workspace at the documentation review commit (see `docs/audits/fexu-020-final-documentation-review.md`).

**Registry note:** `fexu new` without `--fexu-path` pins `fexu = "=0.2.0"`. Until crates.io publishes 0.2.0, use `--fexu-path` as below or build dependencies from the framework checkout.

Fexu requires Rust **1.89.0+**. New projects default to SQLite (`sqlite://db.sqlite3?mode=rwc`).

## Install CLI from this repository

```bash
git clone https://github.com/Iam-Sanja/fexu-rs.git
cd fexu-rs
cargo install --path crates/fexu-cli --locked
fexu --version
```

## New fullstack project

```bash
mkdir -p ~/fexu-apps && cd ~/fexu-apps
fexu new project demo --template fullstack --fexu-path /path/to/fexu-rs
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
fexu new project api-demo --template api --fexu-path /path/to/fexu-rs
```

API projects pin `fexu = "=0.2.0"` without the `admin-ui` feature set.

## PostgreSQL

Set `database.url` in `fexu.toml` or export `DATABASE_URL` with a **disposable** database. Never use production or shared shop databases for migration experiments.

## Next steps

- [First-project tutorial](../tutorial/first-project.md)
- [CLI reference](../reference/cli.md)
- [Upgrade from 0.1.2](../how-to/upgrade-0.1.2-to-0.2.md)
- [Deployment](../how-to/deployment.md)