#!/usr/bin/env bash
# Disposable CI stack only. Never adopt an existing Compose project.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
chmod 700 "$work"
project="fexu-smoke-$(python3 -c 'import secrets; print(secrets.token_hex(8))')"
compose=(docker compose --project-name "$project" --file "$root/deploy/compose.yaml")
export POSTGRES_PASSWORD="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
export FEXU_APP_PASSWORD="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
export FEXU_CSRF_SECRET="$(python3 -c 'import secrets; print(secrets.token_hex(32))')"
export FEXU_MFA_KEY="$(python3 -c 'import secrets; print(secrets.token_hex(32))')"
export FEXU_PASSWORD="$(python3 -c 'import secrets; print("Fixture!" + secrets.token_hex(24))')"
export NO_PROXY=localhost,127.0.0.1
cleanup() {
  local status=$?
  if [[ "$status" -ne 0 ]]; then
    "${compose[@]}" ps || true
    "${compose[@]}" logs --tail=60 app proxy || true
  fi
  "${compose[@]}" down --volumes --remove-orphans || true
  rm -rf "$work"
}
trap cleanup EXIT
"${compose[@]}" config --quiet
"${compose[@]}" build app
"${compose[@]}" up -d --wait db
# The owner is used only for the explicit migration and fixture user setup.
owner_url="postgres://fexu:${POSTGRES_PASSWORD}@db:5432/fexu_test_deploy"
"${compose[@]}" run --rm --no-deps -e DATABASE_URL="$owner_url" app \
  fexu migrate up --migrations-dir /home/fexu/migrations
"${compose[@]}" run --rm --no-deps -e DATABASE_URL="$owner_url" -e FEXU_PASSWORD app \
  fexu admin create-user --email deploy-admin@example.test --staff --superuser
"${compose[@]}" up -d --wait app proxy

# Ensure neither application nor database has a host-published port.
for service in db app; do
  container="$("${compose[@]}" ps -q "$service")"
  test -n "$container"
  docker inspect "$container" --format '{{json .HostConfig.PortBindings}}' |
    python3 -c 'import json,sys; assert not json.load(sys.stdin), "unexpected host port binding"'
done
# Test the actual runtime role, not the migration owner.
"${compose[@]}" exec -T -e PGPASSWORD="$FEXU_APP_PASSWORD" db \
  psql -h 127.0.0.1 -U fexu_app -d fexu_test_deploy -v ON_ERROR_STOP=1 -Atc \
  "SELECT count(*) FROM _fexu_users WHERE email = 'deploy-admin@example.test';" | grep -Fx 1
if "${compose[@]}" exec -T -e PGPASSWORD="$FEXU_APP_PASSWORD" db \
  psql -h 127.0.0.1 -U fexu_app -d fexu_test_deploy -v ON_ERROR_STOP=1 \
  -c 'CREATE TABLE fexu_runtime_must_not_create (id integer);' >"$work/ddl.log" 2>&1; then
  echo "runtime role can create tables" >&2
  exit 1
fi
grep -qi 'permission denied' "$work/ddl.log"

base="https://localhost:8443"
# Trust only the test proxy's CA; never use curl -k.
for _ in $(seq 1 60); do
  if "${compose[@]}" exec -T proxy cat /data/caddy/pki/authorities/local/root.crt >"$work/ca.pem" 2>/dev/null &&
      curl --noproxy '*' --cacert "$work/ca.pem" -fsS --max-time 5 "$base/readyz" >/dev/null; then
    break
  fi
  sleep 1
done
export SSL_CERT_FILE="$work/ca.pem"
curl --noproxy '*' --cacert "$SSL_CERT_FILE" -fsS --max-time 10 "$base/healthz"
curl --noproxy '*' --cacert "$SSL_CERT_FILE" -fsS --max-time 10 "$base/readyz"
curl --noproxy '*' --cacert "$SSL_CERT_FILE" -fsS --max-time 10 "$base/static/css/app.css" >/dev/null

# Verify actual structured access events, not only a helper's return value.
log_probe="fexu-log-probe-$(python3 -c 'import secrets; print(secrets.token_hex(8))')"
curl --noproxy '*' --cacert "$SSL_CERT_FILE" -fsS --max-time 10 \
  -H "x-request-id: $log_probe" "$base/healthz?token=fixture-query-secret" >/dev/null
curl --noproxy '*' --cacert "$SSL_CERT_FILE" -sS --max-time 10 \
  -H "x-request-id: $log_probe" "$base/fixture-path-secret?token=fixture-query-secret" >/dev/null
curl --noproxy '*' --cacert "$SSL_CERT_FILE" -sS --max-time 10 \
  -H "x-request-id: $log_probe" "$base/static/fixture-path-secret?token=fixture-query-secret" >/dev/null
"${compose[@]}" logs --no-color --no-log-prefix app >"$work/access.log"
python3 - "$work/access.log" "$log_probe" <<'PY'
import json
import sys
from pathlib import Path
events = []
for line in Path(sys.argv[1]).read_text(encoding="utf-8").splitlines():
    try:
        event = json.loads(line)
    except ValueError:
        continue
    fields = event.get("fields", {})
    if fields.get("request_id") == sys.argv[2] and fields.get("message") == "http request":
        events.append(fields)
assert any(e.get("path") == "/healthz" and e.get("status") == 200 for e in events), "missing health access event"
assert any(e.get("path") == "<unmatched>" and e.get("status") == 404 for e in events), "missing redacted unmatched access event"
assert any(e.get("path") == "/static/{*path}" and e.get("status") == 404 for e in events), "missing redacted dynamic route event"
assert not any("fixture-query-secret" in str(e) or "fixture-path-secret" in str(e) for e in events), "raw request values leaked"
print("PASS: structured request logs redact unmatched paths and query values")
PY

python3 - "$root/scripts" "$base" "$work/session" <<'PY'
import json
import os
import sys
import urllib.parse
from pathlib import Path
sys.path.insert(0, sys.argv[1])
from v012_fullstack_upgrade_lib import HttpClient

base = sys.argv[2]
email = "deploy-admin@example.test"
password = os.environ["FEXU_PASSWORD"]
body = json.dumps({"email": email, "password": password}).encode()
client = HttpClient(base)
status, _, _ = client.request("POST", "/auth/login", headers={
    "Content-Type": "application/json", "Origin": "https://foreign.invalid",
    "X-Forwarded-Host": "localhost:8443", "X-Forwarded-Proto": "https",
}, body=body)
assert status == 403 and not client.cookie("fexu_session"), "foreign origin accepted"
client = HttpClient(base)
status, _, _ = client.request("GET", "/admin/login")
assert status == 200 and client.cookie("fexu_login_csrf"), "missing native login proof"
invalid_body = urllib.parse.urlencode({
    "csrf_token": "incorrect", "email": email, "password": password, "next": "/admin",
}).encode()
status, _, _ = client.request("POST", "/admin/login", headers={
    "Content-Type": "application/x-www-form-urlencoded", "Origin": base, "Referer": base + "/admin/login",
}, body=invalid_body)
assert status == 403 and not client.cookie("fexu_session"), "incorrect login CSRF accepted"
client = HttpClient(base)
session = client.login(email, password, base)
assert any(c.name == "fexu_session" and c.secure for c in client.jar), "session cookie not Secure"
Path(sys.argv[3]).write_text(session, encoding="utf-8")
PY
python3 "$root/scripts/v012_fullstack_upgrade_runner.py" session-check "$base" "$(cat "$work/session")" deploy-admin@example.test
# A writable media volume must survive application container recreation.
"${compose[@]}" exec -T app sh -c 'test "$(id -u)" -ne 0 && printf fixture > /home/fexu/media/.deployment-probe'
"${compose[@]}" stop db
curl --noproxy '*' --cacert "$SSL_CERT_FILE" -fsS --max-time 10 "$base/healthz"
status="$(curl --noproxy '*' --cacert "$SSL_CERT_FILE" -sS --max-time 60 -o /dev/null -w '%{http_code}' "$base/readyz")"
test "$status" = 503
"${compose[@]}" up -d --wait db
# SIGTERM should yield a normal exit, rather than Docker's forced SIGKILL.
app_id="$("${compose[@]}" ps -q app)"
"${compose[@]}" stop -t 30 app
test "$(docker inspect "$app_id" --format '{{.State.ExitCode}}')" = 0
"${compose[@]}" up -d --no-deps --force-recreate --wait app
# Recreate proxy to avoid any stale upstream connection or DNS cache.
"${compose[@]}" up -d --no-deps --force-recreate --wait proxy
for _ in $(seq 1 60); do
  if curl --noproxy '*' --cacert "$SSL_CERT_FILE" -fsS --max-time 5 "$base/readyz" >/dev/null; then
    break
  fi
  sleep 1
done
curl --noproxy '*' --cacert "$SSL_CERT_FILE" -fsS --max-time 10 "$base/readyz"
"${compose[@]}" exec -T app sh -c 'test "$(cat /home/fexu/media/.deployment-probe)" = fixture'
python3 "$root/scripts/v012_fullstack_upgrade_runner.py" session-check "$base" "$(cat "$work/session")" deploy-admin@example.test
echo "PASS: disposable Compose TLS, runtime role, database outage, graceful exit and volume/session persistence"
