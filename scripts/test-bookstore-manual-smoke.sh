#!/usr/bin/env bash
# Isolated smoke test of the unreleased fullstack generator used by the manual.
# Does not touch project databases or publish crates.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cli="$repo/target/debug/fexu"
if [[ ! -x "$cli" ]]; then
  echo "Build the development CLI first: cargo build -p fexu-cli --bin fexu" >&2
  exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export CARGO_TARGET_DIR="$repo/target"
export DATABASE_URL="sqlite://$work/bookstore.sqlite3?mode=rwc"
cd "$work"

"$cli" new project bookstore --template fullstack --fexu-path "$repo"
cd bookstore
grep -Fq 'features = ["admin-ui"]' Cargo.toml
grep -Fq 'path = ' Cargo.toml
"$cli" doctor
"$cli" new app catalog
"$cli" generate model Book --app catalog
test -f apps/catalog/models/book.rs
test -f apps/catalog/views.rs
test -f apps/catalog/urls.rs

# A fresh project must be able to compile its registered app and schema.
cargo check --all-targets
"$cli" migrate make initial --app catalog

# The planner is intentionally read-only and will not create a missing SQLite
# database. Create only this disposable test fixture before planning.
python3 - "$work/bookstore.sqlite3" <<'PY'
import sqlite3
import sys

with sqlite3.connect(sys.argv[1]):
    pass
PY

"$cli" migrate plan
"$cli" migrate up
"$cli" migrate status
"$cli" check

# Check actual SQLite data and migration history, not just exit statuses.
python3 - "$work/bookstore.sqlite3" <<'PY'
import sqlite3
import sys
from pathlib import Path

db = Path(sys.argv[1])
assert db.is_file(), f"migration did not create SQLite file: {db}"
with sqlite3.connect(db) as connection:
    assert connection.execute("PRAGMA quick_check").fetchone()[0] == "ok"
    tables = {row[0] for row in connection.execute(
        "SELECT name FROM sqlite_master WHERE type='table'"
    )}
    assert "books" in tables, f"Book table missing: {tables}"
    count = connection.execute(
        "SELECT COUNT(*) FROM _fexu_migrations WHERE id LIKE 'catalog:%'"
    ).fetchone()[0]
    assert count >= 1, "catalog migration not recorded"
PY

echo "PASS: unreleased fullstack Bookstore generated, compiled and migrated in isolation"
