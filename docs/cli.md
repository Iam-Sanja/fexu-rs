# Fexu CLI reference

The `fexu` executable is installed by the `fexu-cli` crate. Commands below are cross-checked against `crates/fexu-cli/src/main.rs` on the development branch. For a published version, choose that version in the documentation selector: **0.2.0 is published; 0.3.0 development is unreleased**.

```sh
cargo install fexu-cli --version 0.2.0 --locked
fexu --version
fexu --help
```

Run `fexu new project` from a directory in which you want a new project. Run project commands inside the directory containing `fexu.toml`. `fexu <command> --help` shows the options supported by your installed binary. An installed 0.2.0 CLI may differ from unreleased development source.

## Command overview

| Command | Purpose | Writes? |
| --- | --- | --- |
| `fexu new <name>` | Shorthand for `new project <name>` | Creates project files |
| `fexu new project <name>` | Generate a standalone API or fullstack project | Creates project files |
| `fexu new app <name>` | Create and register an app in a layout-2 project | Creates app files |
| `fexu doctor` | Diagnose tools, files and configuration | No migrations |
| `fexu check` | Check the project, migrations and compilation | No migrations |
| `fexu upgrade` | Inspect or add a missing legacy manifest | Only with `--yes` |
| `fexu dev` | Start development server with source watching | No migrations |
| `fexu build` | Build the release application | Build artifacts |
| `fexu generate model` | Create and register model source | Creates files |
| `fexu generate worker` | Generate worker entry point | Creates files |
| `fexu migrate` | Make, inspect and execute versioned schema migrations | Depends on subcommand |
| `fexu admin` | Create staff/admin users | Writes account data |
| `fexu auth` | Manage users, roles and permissions | Depends on subcommand |
| `fexu worker` | Run the persistent-jobs worker | Processes jobs |
| `fexu jobs` | Inspect, retry and cancel jobs | Depends on subcommand |
| `fexu dumpdata` | Export database rows as JSON | No database changes |
| `fexu loaddata` | Validate or import fixture rows | Except `--dry-run` |
| `fexu manage` | Execute project-defined management commands | Command-specific |

## Projects, apps and development

```sh
fexu new project shop --template fullstack
fexu new project service --template api
fexu new project shop --apps-dir apps
fexu new project shop --apps-dir .
fexu new app catalog --templates local
fexu new app catalog --templates shared
fexu generate model Product --app catalog --timestamps
fexu generate worker
fexu doctor
fexu check
fexu check --strict
fexu check --deploy
fexu dev
fexu build
```

Options: `new project` accepts `--template api|fullstack` (default `fullstack`), `--apps-dir apps|.` (default `apps`), and `--fexu-path <workspace-root-or-crates/fexu>` for development against a source checkout. Without `--fexu-path`, the generator uses a registry dependency. `new app` accepts `--templates local|shared` (default `local`). `generate model` accepts `--app <app>` and `--timestamps`. In layout-2 projects, create and register an app first.

`fexu new help project` displays the project help. `fexu generate app` is a retired command and does not generate files; use `fexu new app`.

`doctor`, `check`, and `dev` do **not** apply migrations. `check --strict` treats warnings as errors; `check --deploy` validates deployment configuration without editing it. `dev` watches relevant source/config/template files and restarts its child process; it is not a production supervisor. `build` compiles a release binary but does not start it.

`upgrade` concerns supported legacy **project file layouts**, not a published-Fexu-version-to-new-Fexu-version source or database migration:

```sh
fexu upgrade --check
fexu upgrade --dry-run
fexu upgrade --yes
```

`--check` and `--dry-run` are read-only; `--yes` may write a missing manifest but does not overwrite an existing one.

## Migrations

Run `fexu migrate plan` and verify a restorable database backup before changing an existing schema.

```sh
fexu migrate make add_stock --app catalog
fexu migrate make add_stock --app catalog --backfill products.stock:i64=0
fexu migrate plan
fexu migrate status
fexu migrate up
fexu migrate down --dry-run
fexu migrate down --app catalog --dry-run
fexu migrate rename-column --table products --from old_name --to new_name
fexu migrate rename-table --from old_products --to products
fexu migrate reseal migrations/0002_add_stock.json
fexu migrate baseline
```

| Subcommand | Supported options | Effect |
| --- | --- | --- |
| `make [name]` | `--app`, `--schema-file`, `--backfill` (repeatable), `--migrations-dir` | Generate migration file |
| `up` | `--database-url`, `--migrations-dir`, `--allow-destructive` | Apply pending migrations |
| `down` | `--database-url`, `--app`, `--migrations-dir`, `--allow-destructive`, `--dry-run` | Undo latest supported migration |
| `plan` | `--database-url`, `--migrations-dir`, `--schema-file` | Read-only schema/history comparison |
| `status` | `--database-url`, `--migrations-dir`, `--schema-file` | Read-only migration status |
| `rename-column` | `--table`, `--from`, `--to`, `--name`, `--schema-file`, `--migrations-dir` | Generate explicit column-rename migration |
| `rename-table` | `--from`, `--to`, `--name`, `--schema-file`, `--migrations-dir` | Generate explicit table-rename migration |
| `reseal <path>` | `--database-url` | Recompute checksum of **unapplied** migration |
| `baseline` | `--database-url`, `--migrations-dir`, `--confirm` | Verify existing SQLite schema; with `--confirm`, write history |

`make --app` cannot be combined with `--schema-file` or `--migrations-dir`. `down --app` cannot be combined with `--migrations-dir`. Destructive schema steps require explicit `--allow-destructive` where supported. `baseline` is SQLite-only and writes only with `--confirm`; it must not be used to conceal mismatched schema history. See [migrations](migrations.md) for detailed workflows.

## Users, roles and permissions

```sh
fexu admin create-user --email admin@example.com --name "Site Administrator" --staff --superuser
fexu auth list-users
fexu auth create-role editor
fexu auth grant-permission products.read --role editor
fexu auth grant-permission products.update --role editor
fexu auth revoke-permission products.update --role editor
fexu auth assign-role --email editor@example.com --role editor
fexu auth revoke-role --email editor@example.com --role editor
```

Each `admin create-user` and `auth` subcommand accepts `--database-url`. `create-user` prompts securely for a password (and email if omitted); it has no default password. `--superuser` is not a substitute for `--staff` when admin access is required. Role permission strings use a resource plus `.read`, `.create`, `.update` or `.delete`. `auth list-users` does not reveal password hashes.

## Background jobs

```sh
fexu generate worker
fexu worker --concurrency 2 --poll-ms 1000
fexu jobs list
fexu jobs failed
fexu jobs inspect JOB_ID
fexu jobs inspect JOB_ID --show-payload
fexu jobs retry JOB_ID --yes
fexu jobs cancel JOB_ID --yes
```

`worker` accepts `--database-url`, `--concurrency` and `--poll-ms`. Every `jobs` subcommand accepts `--database-url`. `inspect` additionally accepts `--show-payload` (potentially sensitive data); `retry` and `cancel` accept `--yes`. The built-in CLI worker runs its supported mail handler; application-specific handlers require a generated and configured worker. See [jobs](jobs.md).

## Fixtures: dumpdata and loaddata

```sh
fexu dumpdata --model products --output fixtures/products.json
fexu dumpdata --app catalog --output fixtures/catalog.json
fexu loaddata fixtures/catalog.json --dry-run
fexu loaddata fixtures/catalog.json
fexu loaddata fixtures/catalog.json --upsert
fexu loaddata fixtures/catalog.json --truncate --yes
```

`dumpdata` accepts `--model`, `--app`, `--schema-file`, `--database-url`, `--output`. If `--output` is omitted it writes to standard output. `loaddata <file>` accepts `--dry-run`, `--upsert`, `--truncate`, `--yes`, `--model`, `--app`, `--schema-file`, `--database-url`. `--truncate` deletes selected data and requires `--yes`: perform it only against an intentionally disposable database or after explicit backup and approval. See [fixtures](fixtures.md).

## Project-defined management commands

```sh
fexu manage
fexu manage --help
fexu manage seed-shop
```

The available management commands are defined by the application's `fexu-manage` binary; `seed-shop` is an example, **not** a guaranteed built-in command in every project. Additional arguments are passed to the project command. See [management commands](management-commands.md).

## Configuration and troubleshooting

SQLite uses a local `sqlite://db.sqlite3?mode=rwc` URL by default in generated projects. PostgreSQL uses a `postgres://` URL supplied through the project configuration or `DATABASE_URL`. Avoid putting credentials into command history; prefer scoped environment variables and protected config.

For command-specific usage on the exact installed CLI:

```sh
fexu --help
fexu migrate --help
fexu migrate make --help
fexu auth --help
fexu jobs --help
fexu loaddata --help
```

See [quickstart](quickstart.md), [configuration](configuration.md), [CLI diagnostics](cli-doctor.md), [authentication](auth.md), and [deployment](deployment.md).
