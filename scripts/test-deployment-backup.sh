#!/usr/bin/env bash
# Self-contained Docker PostgreSQL recovery rehearsal; no supplied database URLs.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
name="fexu-backup-$(python3 -c 'import secrets; print(secrets.token_hex(8))')"
password="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
container=""
cleanup() {
  if [[ -n "$container" ]]; then docker rm -f -v "$container" >/dev/null; fi
}
trap cleanup EXIT
container="$(docker run --rm -d --name "$name" -e POSTGRES_USER=fexu -e POSTGRES_PASSWORD="$password" -e POSTGRES_DB=postgres -p 127.0.0.1::5432 postgres:16)"
for _ in $(seq 1 90); do
  if docker exec "$container" pg_isready -h 127.0.0.1 -U fexu -d postgres >/dev/null 2>&1; then break; fi
  sleep 1
done
port="$(docker port "$container" 5432/tcp | sed -n 's/^127\.0\.0\.1://p')"
test -n "$port"
base="postgres://fexu:$password@127.0.0.1:$port"
psql -X -v ON_ERROR_STOP=1 "$base/postgres" -c 'CREATE DATABASE fexu_test_source'
psql -X -v ON_ERROR_STOP=1 "$base/postgres" -c 'CREATE DATABASE fexu_test_target'
psql -X -v ON_ERROR_STOP=1 "$base/fexu_test_source" <<'SQL'
CREATE TABLE recovery_probe (id integer PRIMARY KEY, payload text NOT NULL);
INSERT INTO recovery_probe VALUES (1, 'backup fixture'), (2, 'second fixture');
SQL
sh "$root/deploy/backup-restore.sh" "$base/fexu_test_source" "$base/fexu_test_target"
expected="$(psql -X -At -v ON_ERROR_STOP=1 "$base/fexu_test_source" -c 'SELECT id, payload FROM recovery_probe ORDER BY id')"
actual="$(psql -X -At -v ON_ERROR_STOP=1 "$base/fexu_test_target" -c 'SELECT id, payload FROM recovery_probe ORDER BY id')"
test "$expected" = "$actual"
test "$(printf '%s\n' "$actual" | wc -l)" -eq 2
# Restore into a non-empty target must fail without changing the existing rows.
if sh "$root/deploy/backup-restore.sh" "$base/fexu_test_source" "$base/fexu_test_target"; then
  echo "restore unexpectedly succeeded into non-empty target" >&2
  exit 1
fi
test "$(psql -X -At -v ON_ERROR_STOP=1 "$base/fexu_test_target" -c 'SELECT id, payload FROM recovery_probe ORDER BY id')" = "$actual"
echo "PASS: real PostgreSQL dump/restore and failed-restore preservation"
