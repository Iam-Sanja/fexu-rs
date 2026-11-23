# ORM: queries, filters and pagination

Fexu generates typed repositories for structs deriving `FexuModel`, and also exposes a composable query builder with `ModelQuery`.

**Internal query-builder version:** Published v0.2.0 uses SeaQuery 0.32.7. The upcoming v0.3.0 development source is evaluating SeaQuery 1.0.2. Fexu's public `ModelQuery` API should remain stable; its SQLite and PostgreSQL SQL generation, migration operations and parameter binding must pass CI before the new dependency is accepted.

This guide assumes the `Book` model introduced in [the first-project tutorial](../tutorial/first-project.md), already registered in the `catalog` app.

## Use a model query from a view or service

```rust
use fexu::db::{col, Direction, ModelQuery};

// db is a previously opened &DatabaseConnection.
let books = Book::query()
    .filter(col("name").starts_with("Rust"))
    .order_by("name", Direction::Asc)
    .limit(20)
    .all(&db)
    .await?;
```

The `Book` type must be in scope, and the `ModelQuery` trait import enables `Book::query()`. Columns are validated against model metadata. Fexu binds values rather than inserting user input directly into SQL.

## Filter and count

```rust
use fexu::db::{any_of, col, ModelQuery};

let matches = Book::query()
    .filter(any_of([
        col("name").eq("Rust by Example"),
        col("name").eq("Rust Cookbook"),
    ]))
    .count(&db)
    .await?;
```

Predicates support equality and inequality, comparisons where field types permit, null checks, membership, and appropriate text matching. For any custom client-controlled filters, validate input and ensure the required authorization scope is applied.

## Page rather than silently truncate

A call to `.all()` returns a **single bounded window** (default: 100 rows). Do not assume it traverses the entire result set:

```rust
use fexu::db::{Direction, ModelQuery};

let page = Book::query()
    .order_by("name", Direction::Asc)
    .limit(25)
    .offset(0)
    .page(&db)
    .await?;

// page.data: rows in this window
// page.total: count of matching rows
// page.has_more: whether another page exists
```

When a decision requires the complete matching set, `all_checked` rejects a truncated one-window result, while `collect_bounded` reads multiple keyset batches up to an explicit safe limit:

```rust
let books = Book::query()
    .collect_bounded(&db, 2_000, || false)
    .await?;
```

These reads do **not** make concurrent business changes safe. Use the appropriate transaction and locking contract for inventory or reservation decisions.

## Permission scopes and transactions

An unscoped Rust query does not infer an object policy. For authenticated or multitenant data, apply the same `QueryScope` that a protected route needs; do not treat `Book::query()` as permission-aware automatically. Model reads and writes can also run against an explicit transaction where the query method accepts `ConnectionTrait`.

For schema and migrations see [Models](models.md), [Database](database.md), and [Migrations](migrations.md). To set up the application from scratch see the [Bookstore tutorial](../tutorial/first-project.md).
