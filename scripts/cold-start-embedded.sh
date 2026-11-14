#!/usr/bin/env bash
# Runs the embedded-migrate binary from a temporary directory while the
# auth and job migration JSON directories are renamed out of the source tree.
# The process must not open CARGO_MANIFEST_DIR. A PostgreSQL URL is accepted
# only when the decoded database name starts with fexu_test_ and the host is
# loopback. The URL user must already be a disposable test role. Without an
# argument the script creates a fresh SQLite file and does not read DATABASE_URL.
# Exit 0 also requires login=ok and a reported job. A killed process cannot
# restore the renamed directories; move migrations.cold-hidden back by hand.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck source=cold-start-url.sh
source "$root/scripts/cold-start-url.sh"
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
else
  reason=$(cold_start_database_rejection "$database_url")
  if [ -n "$reason" ]; then
    printf '%s\n' "$reason" >&2
    exit 2
  fi
fi

auth_src="$root/crates/fexu-auth/migrations"
jobs_src="$root/crates/fexu-jobs/migrations"
auth_hide="$root/crates/fexu-auth/migrations.cold-hidden"
jobs_hide="$root/crates/fexu-jobs/migrations.cold-hidden"
if [ -e "$auth_hide" ] || [ -e "$jobs_hide" ]; then
  echo "a previous cold start left migrations.cold-hidden in place" >&2
  exit 1
fi

restore_one() {
  local hide="$1"
  local src="$2"
  if [ -d "$hide" ] && [ ! -e "$src" ]; then
    if ! mv "$hide" "$src"; then
      echo "Could not restore $src. Move '$hide' back to '$src' by hand." >&2
      echo "A killed process does not run this cleanup. If migrations.cold-hidden remains, move it back before the next build." >&2
      return 1
    fi
  elif [ -e "$hide" ] && [ -e "$src" ]; then
    echo "Both $src and $hide exist. Leave both until you choose which copy to keep." >&2
    echo "A killed process does not run this cleanup." >&2
    return 1
  elif [ ! -e "$hide" ] && [ ! -e "$src" ]; then
    echo "Neither $src nor $hide exists. The migration directory was not restored." >&2
    echo "A killed process does not run this cleanup." >&2
    return 1
  fi
  return 0
}

restore() {
  local saved=$?
  local restore_failed=0
  restore_one "$auth_hide" "$auth_src" || restore_failed=1
  restore_one "$jobs_hide" "$jobs_src" || restore_failed=1
  if [ "$restore_failed" -ne 0 ]; then
    return 1
  fi
  return "$saved"
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
