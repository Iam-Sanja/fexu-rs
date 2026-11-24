#!/usr/bin/env bash
# No PostgreSQL connection: mock the two clients and prove argument guards.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
export FEXU_TEST_CLIENT_LOG="$work/calls"
mkdir "$work/bin"
cat > "$work/bin/pg_dump" <<'MOCK'
#!/bin/sh
set -eu
test -z "${PGHOSTADDR+x}"
test -z "${PGSERVICE+x}"
printf 'dump\n' >> "$FEXU_TEST_CLIENT_LOG"
MOCK
cat > "$work/bin/pg_restore" <<'MOCK'
#!/bin/sh
set -eu
test -z "${PGHOSTADDR+x}"
test -z "${PGSERVICE+x}"
case " $* " in *" --single-transaction "*) ;; *) exit 1 ;; esac
printf 'restore\n' >> "$FEXU_TEST_CLIENT_LOG"
MOCK
chmod +x "$work/bin/"*
export PATH="$work/bin:$PATH"
export PGHOSTADDR=203.0.113.10 PGSERVICE=untrusted
source_url="postgres://tester:password@127.0.0.1:5432/fexu_test_source"
target_url="postgresql://tester:password@[::1]:5432/fexu_test_target"
reject() {
  : > "$FEXU_TEST_CLIENT_LOG"
  if sh "$root/deploy/backup-restore.sh" "$1" "$2" >/dev/null 2>&1; then
    echo "unsafe backup URL accepted" >&2
    exit 1
  fi
  test ! -s "$FEXU_TEST_CLIENT_LOG"
}
for url in \
  'postgres://fexu_test:password@127.0.0.1:5432/production' \
  'postgres://tester:fexu_test@127.0.0.1:5432/production' \
  'postgres://tester@203.0.113.10:5432/fexu_test_source' \
  'postgres://tester@127.0.0.1:5432/fexu_test_source?dbname=production' \
  'postgres://tester@127.0.0.1:5432/fexu_test_source?hostaddr=203.0.113.10' \
  'postgres://tester@127.0.0.1:5432/fexu_test_source#fragment' \
  'postgres://tester@127.0.0.1:5432/fexu_test' \
  'postgres://tester@127.0.0.1:5432/fexu_test_' \
  'postgres://tester@127.0.0.1:5432/fexu_test_source/extra' \
  'postgres://tester@127.0.0.1:0/fexu_test_source' \
  'postgres://tester@127.0.0.1/fexu_test_source' \
  'postgres://tester@127.0.0.1:5432/fexu_test_%00' \
  'postgres://tester@127.0.0.1:5432,203.0.113.10:5432/fexu_test_source' \
  'sqlite:///fexu_test_source'; do
  reject "$url" "$target_url"
  reject "$source_url" "$url"
done
reject "$source_url" 'postgresql://other@127.0.0.1:5433/fexu_test_source'
: > "$FEXU_TEST_CLIENT_LOG"
sh "$root/deploy/backup-restore.sh" "$source_url" "$target_url"
test "$(cat "$FEXU_TEST_CLIENT_LOG")" = "$(printf 'dump\nrestore')"
echo "backup/restore URL policy passed without database access"
