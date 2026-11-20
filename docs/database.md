# Fexu-db – A repository layer for SQLite and PostgreSQL

[Overview](index.md) · **Implemented files:** `crates/fexu-db/src/lib.rs`, `crates/fexu-db/src/query.rs`, `crates/fexu-db/src/sql.rs`. The repository implementation generates `crates/fexu-macros/src/lib.rs`.

## Auth tables (v0.7)

`_fexu_users`, `_fexu_sessions`, `_fexu_roles`, `_fexu_permissions`, `_fexu_user_roles` and `_fexu_role_permissions` are in `fexu-auth` and use SeaQuery on the same connection. They are not `FexuModel` CRUD models so that `password_hash` does not appear as a REST resource. Time stamps are `i64`. UUID is the UUID type in SQLite Text and PostgreSQL. The introspection passes table names with prefix `_fexu_`.

SQLite uses `max_connections(1)`. During a write transaction, no call may request a second connection from the same pool. `conclude` and the `*_in` methods are in [transactions.md](transactions.md).

## Responsibility (Implemented)

- `connect(url)` opens the Fexu database driver. SQLite uses a dedicated worker-thread connection through bundled rusqlite, with `PRAGMA foreign_keys=ON`; PostgreSQL uses the SQLx PostgreSQL driver. See [database driver](database-driver.md).
- Without set `DATABASE_URL`, myshop uses `DEFAULT_SQLITE_URL`: `sqlite://db.sqlite3?mode=rwc`. The path is relative to the working directory of the process.
- `ModelRepository` is the public interface for `list`, `get`, `create`, `update` and `delete`. Each method also has a `_scoped` variant accepting a `QueryScope`. Handlers do not need to know the SQL dialect. See [query-scopes.md](query-scopes.md).
- There is a query implementation. SeaQuery renders SQLite or PostgreSQL. MySQL is rejected.

```rust
#[async_trait::async_trait]
pub trait ModelRepository: FexuModel + Serialize + DeserializeOwned {
    type Create: DeserializeOwned + Send;
    type Update: DeserializeOwned + Send;
    async fn list(db: &DatabaseConnection) -> Result<ListPage<Self>, FexuError>;
    async fn get(db: &DatabaseConnection, id: uuid::Uuid) -> Result<Option<Self>, FexuError>;
    async fn create(db: &DatabaseConnection, input: Self::Create) -> Result<Self, FexuError>;
    async fn update(db: &DatabaseConnection, id: uuid::Uuid, input: Self::Update) -> Result<Self, FexuError>;
    async fn delete(db: &DatabaseConnection, id: uuid::Uuid) -> Result<DeleteOutcome, FexuError>;
}
```

`ListPage<T>` contains `data: Vec<T>`, `limit: usize`, `truncated: bool`. `DeleteOutcome` is `Deleted` or `Missing`. Public methods provide the model, not `serde_json::Value`. The JSON image remains within `fexu-db`.

## Dialect and Types (Implemented)

`DatabaseKind` is `Sqlite` or `Postgres`. `ColumnSpec` describes the name, `ColumnKind`, nullability, primary-key status, optional Boolean default, decimal precision/scale and choice values. `ColumnKind` includes `Uuid`, `Text`, `BigInt`, `Boolean`, `Date`, `Time`, `DateTime`, `Decimal` and `Choice`. SQLite stores all three temporal types as text; PostgreSQL uses `DATE`, `TIME` and `TIMESTAMPTZ`. Decimal values are 16-byte order-preserving coefficients (`BLOB` / `BYTEA`), not `REAL` and not lexicographic text. Choice values are text plus a `CHECK` constraint. The macro supplies this metadata, and `render_schema` uses it to build `CREATE TABLE IF NOT EXISTS`. See [date-time-types.md](date-time-types.md) and [models.md](models.md).

| Model type | PostgreSQL | SQLite | JSON-API |
|---|---|---|---|
| `Uuid` | `uuid` | `text` | String |
| `String` | `text` | `text` | String |
| `i64` | `bigint` | `bigint` | Number |
| `bool` | `bool`, Default `TRUE`/`FALSE` | `integer`, Default `1`/`0` | `true`/`false` |
| `Decimal` | `bytea` coefficient | `blob` coefficient | Decimal string |
| Choice enum | `text` and `CHECK` | `text` and `CHECK` | Stored value string |
| `Option<T>` |Column without `NOT NULL`|Column without `NOT NULL`|Value or `null`|

`i64` remains the minor-unit integer. Decimal filters and `ORDER BY` use the coefficient bytes, so `10` sorts after `9`. A JSON number is accepted only when `rust_decimal` can read its lexical token as an exact decimal. Details and the no-rounding rule are in [models.md](models.md).

SeaQuery binds values using PostgreSQL `$1`, `$2` or SQLite `?` placeholders. Identifiers come only from validated macro attributes and are quoted. Inserts and updates use `RETURNING`, supported by sqlx’s bundled SQLite library. Rows are read column by column and deserialized into models. An empty PATCH reads the current row without updating it; a non-empty PATCH writes only changed columns.

## Query behavior (implemented)

### Create

1. DTO from HTTP or direct Rust call.
2. Generated `required`, Length and `min` test.
3. UUID with `Uuid::new_v4()`.
4. SeaQuery-`INSERT` with bound values and `RETURNING`.
5. Internal deserialization to the model. Errors become `FexuError::Database`.

### Update

1. `PatchField::Missing` leaves the column unchanged.
2. `PatchField::Null` is a validation error for `NOT NULL` and writes SQL `NULL` for nullable columns.
3. `PatchField::Value` checks and stores the original value.
4. With no changed columns, call `get`; otherwise, execute `UPDATE ... WHERE "id" = ... RETURNING`.
5. A missing row returns `NotFound`. There is no version field; the last write wins.

### Delete, Get and List

`DELETE` filters by the bound UUID. Zero affected rows produce `Missing`; `get` returns `Ok(None)` when no row exists. Repository `list` orders by `"id"` and reads 101 rows, returning at most 100 objects and setting `truncated` if more exist. UUID ordering does not represent creation time. The generated REST list is a cursor page (`data`, `limit`, `next`, `has_more`) on the same keyset engine as `Model::query()`. `truncated` stays on the repository `ListPage` and is not an HTTP field.

## Setup

SQLite is the local standard. An installation and Docker are not necessary for this. sqlx brings SQLite. Schema changes for applications run through [migrations.md](migrations.md). `render_schema` remains the legacy bootstrap with `IF NOT EXISTS` for empty test databases. Migrations use `CREATE TABLE` without `IF NOT EXISTS`.

```text
sqlite://db.sqlite3?mode=rwc
sqlite::memory:
postgres://user:password@localhost:5432/fexu
```

`sqlite::memory:` is a separate database for each connection. Fexu limits SQLite pools to one connection so that tests and the application see the same database. The file `db.sqlite3` is created in the working directory and is contained in `.gitignore`. PostgreSQL remains a supported backend and is dialed via `DATABASE_URL`. `compose.yaml` can optionally start the server. Bootstrap only runs `CREATE TABLE IF NOT EXISTS`, does not delete existing rows and does not change existing tables. This is not a migration engine.

## Error and security (Implemented)

Deserialization errors and unknown SQL errors become `FexuError::Database`. The HTTP layer then responds with 500 and a generic text. The database detail goes to the tracing log, not the answer. `DATABASE_URL` is not logged. `sqlx::ErrorKind::UniqueViolation` and `ForeignKeyViolation` are `Validation` and thus 422. SQLite reports `ON DELETE RESTRICT` as extended code `1811` with the fixed message `FOREIGN KEY constraint failed`. This will also be 422. Missing lines remain 404, unreleased routes 403. SQLite foreign keys are on. Models can generate unique indexes, secondary indexes and foreign keys. Relationship methods are in [relations.md](relations.md). `find_by_column` loads typed child lines. SQLite checks types weaker than PostgreSQL. Fexu checks the supported values before writing in Rust.

## Composable queries

`Product::query()` returns a `Select<Product>`. Import `ModelQuery` so the method is in scope. The query accepts `&DatabaseConnection` and any other `&impl ConnectionTrait`, including an open `DatabaseTransaction`. It does not take a second connection from the pool.

`list` and `list_scoped` are unchanged.

```rust
use fexu::core::QueryScope;
use fexu::db::{any_of, col, not, Direction, ModelQuery};

// 1. Active products with stock greater than zero, ordered by price.
let available = Product::query()
    .filter(col("active").eq(true))
    .filter(col("stock").gt(0_i64))
    .order_by("price_cents", Direction::Asc)
    .all(&db)
    .await?;

// 2. The same order with the stable id tie-break appended by the query.
let by_price = Product::query()
    .order_by("price_cents", Direction::Asc)
    .order_by("name", Direction::Asc)
    .all(&db)
    .await?;

// 3. AND, OR and NOT. Values are bound parameters, not SQL text.
// The scope comes from the object policy; this shows an equality scope.
let matched = Product::query()
    .filter(col("active").eq(true))
    .filter(any_of([
        col("name").eq("bolt"),
        col("price_cents").lte(500_i64),
    ]))
    .filter(not(col("name").is_null()))
    .scope(QueryScope::Equals {
        column: "category_id",
        value: fexu::core::ScopeValue::Uuid(category_id),
    })
    .all(&db)
    .await?;

// 4. Walk more than 100 rows. Each call returns at most 100.
let mut offset = 0_u64;
loop {
    let page = Product::query()
        .order_by("price_cents", Direction::Asc)
        .limit(40)
        .offset(offset)
        .page(&db)
        .await?;
    // `page.data` is this window. `page.total` counts the filtered rows.
    if !page.has_more {
        break;
    }
    offset += page.data.len() as u64;
}
```

For correctness-sensitive checks (for example a blocking reservation after the 100th row), do not use `.all()` as if it were exhaustive. Choose a checked single window or a bounded complete scan:\n\n```rust\n// One window only, but fail rather than silently omit matching rows.\nlet one_window = Product::query().filter(col("active").eq(true))\n    .all_checked(&db).await?;\n\n// Read every matching row up to an explicit safe maximum (including row 101).\n// Cancellation and exceeding the maximum are errors, not partial success.\nlet all_available = Product::query().filter(col("active").eq(true))\n    .order_by("price_cents", Direction::Asc)\n    .collect_bounded(&db, 2_000, || false).await?;\n\n// If the result may exceed 10_000, process it page by page instead.\nlet mut scan = Product::query().scan()?;\nwhile !scan.is_complete() {\n    for product in scan.next_batch(&db).await? {\n        // Process the current batch without accumulating unbounded memory.\n    }\n}\n```\n\nThese are database scans, **not** concurrency-safe reservation locks. For availability decisions that can race with writes, use the transaction/locking contract in addition to a complete query. `scan_after` resumes from a saved cursor but is not a snapshot of mutations between requests.\n\n`policy_scope` is a `QueryScope` from the object policy. The default scope is `QueryScope::All`, the same default as `list`. `QueryScope::None` matches no row. `QueryScope::Equals` is AND-ed with the filters. A query does not invent a policy.

| Call | Window | Result |
|---|---|---|
| `all` | `limit` and `offset` | At most `limit` models. Default limit 100. |
| `all_checked` | `limit`, offset must be zero | All matching rows if they fit in one window; otherwise `Validation` (`query truncated`) rather than silently returning an incomplete result. |
| `page` | same window, plus a count | `QueryPage` with `data`, `limit`, `offset`, `total`, `has_more`. |
| `count` | ignored | Filtered row count, including rows outside the page. |
| `exists` | ignored | Whether the filter matches at least one row. |
| `scan` / `next_batch` | one keyset page | The next page only. Earlier pages are not kept. An empty page ends the scan. |
| `Select::collect_bounded` / `RowScan::collect_bounded` | up to the caller bound, at most 10_000 | Every matching row via keyset batches, or `Validation` when more rows exist, the caller cancels, or the bound is outside 1..=10_000. A shortened list is not returned. |

`count` and `exists` still reject an invalid `limit` or `offset` on that builder.

Supported comparisons are `eq`, `ne`, `gt`, `gte`, `lt`, `lte`, `is_in`, `is_not_in`, `is_null` and `is_not_null`. Text and choice columns also accept `contains`, `starts_with` and `ends_with`. Those three are case-sensitive: PostgreSQL uses `LIKE ... ESCAPE '\'`, and SQLite uses `GLOB` because SQLite `LIKE` is case-insensitive for ASCII. `contains_ignore_case`, `starts_with_ignore_case` and `ends_with_ignore_case` use `LOWER(column) LIKE ? ESCAPE '\'` with a bound pattern. The pattern is folded in Rust before binding: ASCII lowercasing on SQLite to match SQLite `LOWER`, Unicode lowercasing on PostgreSQL to match PostgreSQL `LOWER`. `%`, `_` and `\` in the term are escaped. A term longer than 200 characters is `Validation`. Neither backend uses Unicode casefolding; SQLite still does not match `MÜNCHEN` to `München` when only non-ASCII case differs.

`where_related` keeps rows whose foreign-key column points at a related model row. `where_has` keeps rows that a related model references. `where_linked` keeps rows joined through `ManyToMany`. `Related::filter` and `Related::scope` apply inside `EXISTS`, so `count`, `page`, `keyset` and the parent `QueryScope` see the same constraint. `Related::negated` emits `NOT EXISTS`. The foreign-key metadata must name the related table. A many-to-many join table must use UUID primary keys and is not itself a model.

`filter` combines with AND. `all_of`, `any_of` and `not` nest AND, OR and NOT. An empty OR matches nothing. `NamedFilter` stores one of those predicates under a safe name. `and`, `or` and `negated` combine stored filters and keep the receiver's name; only the predicate is sent. The same `NamedFilter` can be passed to `filter_named` on a list and on `aggregate`. An empty `IN` matches nothing, including a `NULL` column. An empty `NOT IN` matches every row, including `NULL`, and is still AND-ed with `QueryScope` so a tenant or object scope is not widened. Boolean and UUID columns accept equality, inequality, `IN`, `NOT IN` and null checks. `i64`, text, date, time and datetime also accept ranges. A comparison whose value type does not match the column is `Validation`.

Date, time and datetime filters accept the canonical strings from [date-time-types.md](date-time-types.md) and the matching `chrono` values: `NaiveDate`, `NaiveTime` and `DateTime<Utc>`. A string and the typed value for the same instant bind the same parameter. Invalid calendar dates, times and non-UTC offsets are `Validation` before a statement is sent. PostgreSQL binds `date`, `time` and `timestamptz`. SQLite keeps text columns: a date is `YYYY-MM-DD`, a time is `HH:MM:SS`, and a timestamp is the RFC 3339 text the SQLite driver writes for `DateTime<Utc>` (`+00:00`, with fractional seconds when the instant has them). `DateTime` is an absolute UTC instant. `Z` and `+00:00` are the same instant; any other offset is rejected and is not converted. Direct filters, `where_related`, `where_has` and keyset continuation use this binder. A whole-second JSON value stays `2026-11-07T13:30:00Z`. A non-zero fraction is kept, for example `2026-11-07T13:30:00.123456Z`.

Column names must be columns of that model and must match `[A-Za-z_][A-Za-z0-9_]*`. Unknown or unsafe names are `Validation` and are not sent to the database. Values go through SeaQuery parameters (`?` on SQLite, `$1` on PostgreSQL). `limit` and `offset` are validated integers; SeaQuery may emit them as numeric literals.

Ordering can list several columns. If `id` is absent, the query appends `id ASC`. Pages of an unchanged filtered set do not overlap or skip when walked with a constant page size. An insert, delete or sort-key update between two requests can move offset pages. This is not keyset pagination and not a snapshot across requests. Read both pages inside one transaction when the set must stay still.

### Limits

- Page size is 1..=100 (`PAGE_LIMIT`). Offset is at most 1_000_000 (`OFFSET_LIMIT`).
- A nonempty `IN` or `NOT IN` list holds at most 100 bound values (`IN_LIMIT`). An empty `IN` is false and an empty `NOT IN` is true. Neither form emits `IN ()`. A longer list is `Validation`, not a database error.
- `all` is one window of at most 100 rows and does not say that further rows exist. For decisions requiring complete results, prefer `all_checked` (fails if that window is incomplete) or `Select::collect_bounded` (walks every match with keyset pages, caller sets a maximum, fails on truncation/cancellation). `page` reports `has_more`; `scan`/`next_batch` is the memory-bounded alternative for arbitrarily large results. All full-scan APIs require offset zero and a keyset-compatible order; they do not guarantee a snapshot if rows change between batches. `collect_bounded` caps collection at 10_000 (`SCAN_COLLECT_LIMIT`). Generated REST lists remain bounded to 100 rows.
- `load_by_ids`, `load_children`, `load_one_to_one` and `load_many_to_many` send at most 100 identifiers per statement. `load_children` does not cap the child rows it returns; page those children with `scan` when the set can be large.
- `annotate` adds one related `COUNT` or `SUM`.
- `Select::aggregate` builds a grouped report. `group_by` accepts text, choice, bigint, uuid, date, time and datetime columns. Boolean, decimal and JSON columns are rejected. `group_by_month` accepts only a date or datetime column and groups by `YYYY-MM`. A null column stays a null group. Datetime months are UTC: PostgreSQL uses `to_char(... AT TIME ZONE 'UTC', 'YYYY-MM')`, and SQLite uses the first seven characters of the canonical text. Several `measure` calls are one query. Measures are `count`, `count_column`, bigint `sum`, bigint `average`, and `min`/`max` of bigint, text, choice, date, time or datetime. Decimal `sum`, `average`, `min` and `max` are rejected before a statement is sent. `average` is `sum * 1_000_000 / count`, truncated toward zero to six decimal places. A product outside `i64` is `Validation` (`average is outside the portable range`). SQL `AVG` is not used, because SQLite returns a float and PostgreSQL returns numeric. `HAVING` accepts only integer measures and a bound `i64`. Ordering by `average` is rejected. The report returns at most 100 groups and has no offset; it does not walk groups past that window. Filters, named filters, relation predicates and `QueryScope` stay in `WHERE`. Explicit order and the remaining group columns use `NULLS LAST`, then ascending group keys, so equal measures keep a stable order on SQLite and PostgreSQL. `character_length_eq` is a bound length check for text and choice only: PostgreSQL `char_length`, SQLite `length`. The length is 0..=200. `annotate` remains one related `COUNT` or bigint `SUM` and applies the child `QueryScope` inside that aggregate.
- JSON path operators, arbitrary SQL expressions and MySQL are not part of this API.
- MySQL is rejected.
- SQLite behavior is covered by `cargo test -p fexu-db`. PostgreSQL SQL generation is asserted in the same crate. `postgres_filters_and_pages_runtime`, `postgres_temporal_filters_and_keyset`, `postgres_empty_in_and_bounded_scan` and `postgres_grouped_report_scope_json_and_one_to_one` are `#[ignore]` and run in the GitHub PostgreSQL CI job. A local workspace run does not mark those tests as passed.

## Fixtures

`fexu dumpdata` and `fexu loaddata` read and write versioned JSON through the schema snapshot. Decimal values travel as strings and are stored as the same 16-byte coefficient used by model inserts. The default load inserts new primary keys and rolls back the whole file when any row conflicts. Details, the confirmed `--truncate` flag and the cycle limit are in [fixtures.md](fixtures.md).

## Filtered writes

`Select::update_matching`, `delete_matching` and `lock_existing` are in `crates/fexu-db/src/write.rs`. `ModelQuery::insert` is the typed insert. Values and arithmetic amounts are bound parameters. Column names must be model fields; a name such as `stock = 0; DROP TABLE` is rejected before SQL is built.

`update_matching` and `delete_matching` refuse a query that has no filter, relation predicate or restricting scope. `update_all` and `delete_all` are the explicit forms for an intentional full-table write. `QueryScope::None` still matches nothing. The page limit does not cap a bulk delete.

`set` writes one bound value. `increment` and `decrement` are one SQL update of the form `column = column + amount`, with `amount > 0` and an integer column. The statement also requires the current value to stay inside `i64` and any numeric bound. A row outside that window is left unchanged and is not counted in `rows_affected`. There is no multiply or divide helper.

`lock_existing` must run on `DatabaseTransaction`. PostgreSQL emits `SELECT ... FOR UPDATE` and locks only the matched rows. SQLite has no row lock. It runs an `UPDATE` whose predicate matches nothing, which takes the database reserved lock until commit or rollback and does not change a stored value. `AFTER UPDATE` triggers and `ON UPDATE CASCADE` do not run for that statement. Another writer on the same SQLite file waits, including a writer of a different row. Zero locked rows do not reserve a row that does not exist yet. A first insert is protected by a real unique constraint, including `on_conflict_do_nothing` on that constraint. Conflict handling does not overwrite an existing row.

`update_all` and `delete_all` keep the `QueryScope`. `QueryScope::All` is the explicit whole-table request. `QueryScope::Equals` still limits the rows, and that scope column cannot be assigned by the bulk update. A bulk update is not `ModelRepository::update`. When the model declares a field contract with `in_update = false`, the bulk builder rejects that column. Text length, email and slug rules are not applied. Declared numeric bounds are applied to `set`, `increment` and `decrement`.

Text, email and slug rules from `update_record` are not repeated here. `rows_affected == 0` means the predicate matched nothing, including a failed stock or price condition. `rows_affected == 1` means that one row changed. SQLite connections set `busy_timeout` to five seconds; a lock that SQLite reports immediately as busy still surfaces as a database error.

`examples/desk` is a small application on this API: models, one generated migration, a URL, a template and the shop operations. `sqlite_same_operations` runs in the workspace tests. `postgres_same_operations` is ignored and listed in the PostgreSQL 16 job. The same Rust source talks to either database through `connect`. SQLite `LOWER` folds ASCII only, so `MÜNCHEN` does not find `München`. A PostgreSQL cluster whose `LOWER` follows a UTF-8 locale can find that row. SQLite `BINARY` order places `Muenchen` before `München`. PostgreSQL order follows the database collation. The capability table is [orm-capability-matrix.md](orm-capability-matrix.md). The write-path audit is [orm-write-security-audit.md](orm-write-security-audit.md).

## Not implemented (planned)

- PostgreSQL baseline. Explicit renames, SQLite table rebuilds, SQLite baselining and `migrate down` are described in [migrations.md](migrations.md). Multi-model transactions are described in [transactions.md](transactions.md).
- Own error codes apart from the two fixed validation texts for Unique and Foreign Key.
- Automatic eager loading and MySQL. JSON documents are stored, not queried with path operators. Explicit many-to-many loads, one-to-one loads and batched foreign-key reads are in [relations.md](relations.md).
- Pool sizes and timeouts as an application configuration except for the fixed SQLite limit of a connection.
- Optimistic locking.

## Tests

`cargo test --workspace` runs `sqlite_http_roundtrip` against `sqlite::memory:` without changing `db.sqlite3`. `postgres_http_roundtrip` remains `#[ignore]` and requires `DATABASE_URL` plus a running PostgreSQL server. `.github/workflows/ci.yml` provides a separate PostgreSQL job for the ignored tests; a local workspace test run does not start that server. See [testing.md](testing.md).

## UUID parameters

`render_schema` stores a Fexu UUID as SQLite `TEXT` and PostgreSQL `UUID`. Do not change that. A column that an application created as `BLOB` is different: `sea_orm::Value::Uuid` matches the bytes SQLite stored for a typed UUID, while `Value::String` of the hyphenated text does not. `Statement::from_sql_and_values` with `Value::Uuid` is the raw-SQL form. Prefer `Item::query()` so the column type comes from the model. `sqlite_uuid_text_does_not_match_a_blob_parameter` demonstrates the mismatch. Case-insensitive text search uses `LOWER`, which on SQLite folds ASCII only; `München` does not match `MÜNCHEN` there.

`QueryCounter` counts model queries on the current thread. Call `QueryCounter::record` after a direct statement you want included. Direct `ConnectionTrait` calls and failed direct statements stay uncounted unless explicitly recorded. Independent threads do not share the counter.

## Admin lists

`query_admin_page` in `crates/fexu-db/src/sql.rs` uses the same SeaQuery layer. Column names come from the descriptor and must be on the search allowlist. The search is `LOWER(column) LIKE ? ESCAPE '\'` on SQLite and `LOWER(column) LIKE $1 ESCAPE '\'` on PostgreSQL. The pattern is bound, not concatenated, and `%`, `_` and `\` in the term are literals. The pattern is folded in Rust with the same ASCII-versus-Unicode rule as `contains_ignore_case`. An empty search adds no predicate. Scope, count and the page window stay in force. The public list limit remains 100. The admin page is at most 100 lines, plus one line to detect `has_more`.
