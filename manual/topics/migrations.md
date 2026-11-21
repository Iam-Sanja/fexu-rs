# Migrations

Schema changes are explicit. Generating a model does **not** silently create tables in an existing database.

```sh
fexu generate model Book --app catalog
fexu migrate make initial --app catalog
fexu migrate plan
fexu migrate up
fexu migrate status
```

**Files:** app-specific migrations appear under `apps/catalog/migrations/`; framework migrations are handled by the framework migration workflow.

## Everyday workflow

1. Edit the model in `apps/catalog/models/book.rs`.
2. Generate a migration with `fexu migrate make name --app catalog`.
3. Inspect the generated schema change and `fexu migrate plan`.
4. Back up an existing database before applying changes.
5. Apply with `fexu migrate up` and inspect `fexu migrate status`.

Adding a required column to a populated table requires a deliberate backfill:

```sh
fexu migrate make add_stock --app catalog --backfill books.stock:i64=0
fexu migrate plan
fexu migrate up
```

Rename operations use `fexu migrate rename-column` and `fexu migrate rename-table`. A rollback may require special flags depending on the migration; inspect `fexu migrate down --dry-run` before changing persisted data. For all supported flags, see [CLI reference](../reference/cli.md).
