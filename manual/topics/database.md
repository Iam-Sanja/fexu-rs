# SQLite and PostgreSQL

Fexu supports SQLite for simple local projects and PostgreSQL for database-backed deployments. SQLite uses bundled `rusqlite`; PostgreSQL uses the SQLx PostgreSQL driver.

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
