# Quickstart

Create a Fexu application, initialize a database, sign in to its admin and start the development server. These commands use the **published Fexu 0.1.2** release. For changes introduced in the upcoming 0.2.0 release, use the matching versioned documentation when it becomes available.

Fexu requires a supported Rust toolchain (the current workspace declares Rust 1.89.0 as its minimum). New projects use SQLite by default; you do not need Docker or a PostgreSQL service for the first application.

## Windows PowerShell

```powershell
cargo install fexu-cli --version 0.1.2 --locked
fexu --version

# Choose a directory outside the Fexu framework repository.
fexu new project demo --template fullstack
Set-Location demo

# Inspect the generated files and configuration.
fexu doctor
fexu check

# Review migrations before applying them.
fexu migrate plan
fexu migrate up

# Prompts for a password; no default credentials are supplied.
fexu admin create-user --email admin@example.com --staff --superuser

# Start the local development server.
fexu dev
```

## Linux or macOS

```bash
cargo install fexu-cli --version 0.1.2 --locked
fexu --version
fexu new project demo --template fullstack
cd demo
fexu doctor
fexu check
fexu migrate plan
fexu migrate up
fexu admin create-user --email admin@example.com --staff --superuser
fexu dev
```

Stop the development process with Ctrl+C. The dev server watches supported source, configuration and template changes and restarts the application. It does not apply migrations automatically.

## Create your first app and model

Inside the generated project:

```sh
fexu new app catalog
fexu generate model Product --app catalog --timestamps
fexu doctor
fexu migrate make add_products --app catalog
fexu migrate plan
```

Review the generated model and migration before running `fexu migrate up`. If a non-nullable field is added to a table containing existing rows, a safe backfill or an explicit migration decision may be needed. See [creating apps](create-app.md), [models](models.md) and [migrations](migrations.md).

## Working against a Fexu source checkout

The CLI normally writes a Cargo registry dependency. To test unpublished source features, use `--fexu-path` with a path to the Fexu workspace root (or its `crates/fexu` directory).

```powershell
Set-Location "D:\04_Projekte_Arbeit\Projekte\Fexu\main"
cargo install --path crates/fexu-cli
$framework = (Get-Location).Path
Set-Location ..
fexu new project demo-main --template fullstack --fexu-path $framework
Set-Location demo-main
fexu doctor
fexu migrate plan
```

A local checkout of `main` contains unreleased APIs and is not equivalent to installing the latest crates.io version. Do not advertise its behavior as published until a corresponding release is available.

## Database selection

The generated SQLite URL is `sqlite://db.sqlite3?mode=rwc`. The database file is created as part of the intended migration/application workflow, not by `fexu doctor`.

For PostgreSQL, configure `[database] url` in `fexu.toml` or supply `DATABASE_URL` in a protected environment. Use a disposable database while evaluating migration behavior. Keep database credentials out of source control and command history.

## Next steps

- [CLI reference](cli.md) — every command, subcommand and important option
- [Project structure](project-structure.md) — files and directories
- [Configuration](configuration.md) — settings and environment variables
- [Models](models.md) — fields, relationships and queries
- [Admin](admin.md) — accounts and management views
- [Deployment](deployment.md) — server and production configuration
