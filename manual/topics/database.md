# SQLite and PostgreSQL

Fexu supports SQLite for simple local projects and PostgreSQL for database-backed deployments. SQLite uses bundled `rusqlite`; PostgreSQL uses the SQLx PostgreSQL driver.

**SQLx version note:** Published v0.2.0 uses SQLx 0.8.6 for PostgreSQL. The ongoing v0.3.0 development work evaluates SQLx core and PostgreSQL 0.9.1 together; this is not part of the immutable v0.2.0 release. Database connections, migrations, query handling and compatibility with existing installations require CI acceptance.

**Version note:** Published v0.2.0 uses `rusqlite` 0.42.0. The v0.3.0 development branch is evaluating `rusqlite` 0.43.0; this is not part of the v0.2.0 tag. SQLite schema and migration behavior must pass CI before adoption.

## SQLite

The generated project has:

```toml
[database]
url = "sqlite://db.sqlite3?mode=rwc"
```

Create models and review migrations before applying them:

```sh
fexu migrate plan
fexu migrate up
fexu migrate status
```

## PostgreSQL

Set the `DATABASE_URL` environment variable to a protected PostgreSQL connection string or configure `[database] url` in your private deployment config. For example, use the *shape* `postgres://user:password@host:5432/database`, but do not paste real passwords into shared logs or shells.

Do not assume that custom SQLite DDL and PostgreSQL DDL are identical; generate/apply migrations using the selected backend.

## Data operations

Model registration exposes the generated REST endpoints and Rust-side model APIs according to its declaration. Permissions are enforced according to the model and app registration, not automatically granted to anonymous users.

For concrete query-builder examples, see [ORM queries and pagination](orm.md).

For a guided creation of the Book model and migration see [the first-project tutorial](../tutorial/first-project.md); for evolving database schema see [migrations](migrations.md).
