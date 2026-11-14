#!/usr/bin/env bash
# Runs the embedded-migrate binary from a temporary directory while the
# auth and job migration JSON directories are renamed out of the source tree.
# The process must not open CARGO_MANIFEST_DIR. Pass a PostgreSQL URL only when
# its database name contains fexu_test. Without an argument the script creates
# a fresh SQLite file and does not read DATABASE_URL.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

if [ -f target/debug/embedded-migrate.exe ]; then
  built=target/debug/embedded-migrate.exe
elif [ -f target/debug/embedded-migrate ]; then
  built=target/debug/embedded-migrate
else
  cargo build -p fexu-cli --bin embedded-migrate
  if [ -f target/debug/embedded-migrate.exe ]; then
    built=target/debug/embedded-migrate.exe
  else
    built=target/debug/embedded-migrate
  fi
fi

dest=$(mktemp -d)
work=$(mktemp -d)
if [[ "$built" == *.exe ]]; then
  bin="$dest/embedded-migrate.exe"
else
  bin="$dest/embedded-migrate"
fi
cp "$built" "$bin"
chmod +x "$bin" 2>/dev/null || true
case "$bin" in
  "$root"/*) echo "binary was not moved out of the source tree" >&2; exit 1 ;;
esac

database_url="${1:-}"
if [ -z "$database_url" ]; then
  db="$work/app.sqlite"
  if command -v cygpath >/dev/null 2>&1; then
    db=$(cygpath -m "$db")
  fi
  database_url="sqlite://${db}?mode=rwc"
elif [[ "$database_url" == postgres://* || "$database_url" == postgresql://* ]]; then
  case "$database_url" in
    *fexu_test*) ;;
    *)
      echo "refusing a PostgreSQL URL whose database name does not contain fexu_test" >&2
      exit 2
      ;;
  esac
fi

auth_src="$root/crates/fexu-auth/migrations"
jobs_src="$root/crates/fexu-jobs/migrations"
auth_hide="$root/crates/fexu-auth/migrations.cold-hidden"
jobs_hide="$root/crates/fexu-jobs/migrations.cold-hidden"
if [ -e "$auth_hide" ] || [ -e "$jobs_hide" ]; then
  echo "a previous cold start left migrations.cold-hidden in place" >&2
  exit 1
fi

restore() {
  if [ -d "$auth_hide" ] && [ ! -d "$auth_src" ]; then
    mv "$auth_hide" "$auth_src"
  fi
  if [ -d "$jobs_hide" ] && [ ! -d "$jobs_src" ]; then
    mv "$jobs_hide" "$jobs_src"
  fi
}
trap restore EXIT

mv "$auth_src" "$auth_hide"
mv "$jobs_src" "$jobs_hide"
if [ -d "$auth_src" ] || [ -d "$jobs_src" ]; then
  echo "migration source directories are still visible" >&2
  exit 1
fi
echo "cold-start source_migrations=hidden binary=$bin"

set +e
output=$(cd "$work" && DATABASE_URL="$database_url" "$bin" 2>&1)
status=$?
set -e
printf '%s\n' "$output"
if [ "$status" -ne 0 ]; then
  echo "embedded-migrate exited $status" >&2
  exit "$status"
fi
case "$output" in
  *login=ok*) ;;
  *) echo "cold start did not log in" >&2; exit 1 ;;
esac
case "$output" in
  *job=*) ;;
  *) echo "cold start did not enqueue a job" >&2; exit 1 ;;
esac
echo "cold-start login=ok job=enqueued"
