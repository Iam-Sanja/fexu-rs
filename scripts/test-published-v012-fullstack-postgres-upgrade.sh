#!/usr/bin/env bash
# Published fexu-cli 0.1.2 fullstack -> current-checkout upgrade on disposable PostgreSQL 16.
set -euo pipefail

baseline="${FEXU_UPGRADE_BASELINE:-0.1.2}"
case "$baseline" in
  0.1.2|0.2.0) ;;
  *) echo "Unsupported published upgrade baseline: $baseline" >&2; exit 2 ;;
esac

python3() {
  if command -v py >/dev/null 2>&1 && py -3 -c 'pass' >/dev/null 2>&1; then
    py -3 "$@"
  elif command -v python3 >/dev/null 2>&1 && command python3 -c 'pass' >/dev/null 2>&1; then
    command python3 "$@"
  else
    echo "python3 or py -3 is required" >&2
    return 127
  fi
}

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if repo_native="$(cd "$repo" && pwd -W 2>/dev/null)"; then
  repo_native="${repo_native//\\//}"
else
  repo_native="$repo"
fi
runner="$repo_native/scripts/v012_fullstack_upgrade_runner.py"
python3 "$runner" check-session-arguments

docker_container=""
ensure_postgres_admin() {
  if [[ -n "${FEXU_POSTGRES_ADMIN:-}" ]]; then
    python3 - "$FEXU_POSTGRES_ADMIN" <<'PY'
from urllib.parse import urlsplit
import sys
url = urlsplit(sys.argv[1])
if url.scheme not in ('postgres', 'postgresql'):
    raise SystemExit('FEXU_POSTGRES_ADMIN must be a postgres URL')
if url.hostname not in ('127.0.0.1', 'localhost'):
    raise SystemExit('Refusing non-local PostgreSQL admin URL')
PY
    return 0
  fi
  if ! command -v docker >/dev/null 2>&1; then
    echo "BLOCKED: set FEXU_POSTGRES_ADMIN or install Docker for a disposable PostgreSQL 16" >&2
    exit 3
  fi
  if ! docker info >/dev/null 2>&1; then
    echo "BLOCKED: Docker daemon is not running" >&2
    exit 3
  fi
  docker_container="fexu-pg-v012-fullstack-$(python3 -c 'import secrets; print(secrets.token_hex(6))')"
  docker run --rm -d --name "$docker_container" \
    -e POSTGRES_USER=fexu \
    -e POSTGRES_PASSWORD=fexu_dev_only \
    -e POSTGRES_DB=fexu \
    -p 127.0.0.1:0:5432 \
    postgres:16 >/dev/null
  host_port="$(docker port "$docker_container" 5432 | head -n1 | awk -F: '{print $NF}')"
  export FEXU_POSTGRES_ADMIN="postgres://fexu:fexu_dev_only@127.0.0.1:${host_port}/fexu"
  for _ in $(seq 1 60); do
    if docker exec "$docker_container" pg_isready -U fexu -d fexu >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.5
  done
  echo "BLOCKED: disposable PostgreSQL 16 did not become ready" >&2
  exit 3
}

work="$(mktemp -d)"
chmod 700 "$work"
dbname="fexu_fullstack_$(python3 -c 'import secrets; print(secrets.token_hex(16))')"
created=0
cleanup() {
  if [[ -n "${server_pid:-}" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  if [[ "$created" -eq 1 && -n "${FEXU_POSTGRES_ADMIN:-}" ]]; then
    psql "$FEXU_POSTGRES_ADMIN" -v ON_ERROR_STOP=1 -c "DROP DATABASE IF EXISTS \"$dbname\" WITH (FORCE);" \
      || echo "ERROR: temporary PostgreSQL database $dbname could not be dropped" >&2
  fi
  if [[ -n "${docker_container:-}" ]]; then
    docker rm -f "$docker_container" >/dev/null 2>&1 || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT

ensure_postgres_admin
python3 "$runner" require-psql

psql "$FEXU_POSTGRES_ADMIN" -v ON_ERROR_STOP=1 -c "CREATE DATABASE \"$dbname\" TEMPLATE template0;"
created=1
admin_base="${FEXU_POSTGRES_ADMIN%/*}"
export DATABASE_URL="${admin_base}/${dbname}"

old_root="$work/published-cli"
new_cli="$repo/target/debug/fexu"
[[ -x "$new_cli" ]] || new_cli="$repo/target/debug/fexu.exe"
mkdir -p "$old_root"
if [[ ! -x "$new_cli" && ! -f "$new_cli" ]]; then
  echo "Build the current CLI first: cargo build -p fexu-cli --bin fexu" >&2
  exit 1
fi

v012_root="$repo/target/v012-cli"
resolve_v012_cli() {
  if [[ -x "$v012_root/bin/fexu" ]]; then
    printf '%s\n' "$v012_root/bin/fexu"
  elif [[ -f "$v012_root/bin/fexu.exe" ]]; then
    printf '%s\n' "$v012_root/bin/fexu.exe"
  else
    return 1
  fi
}

if [[ "$baseline" == "0.2.0" ]]; then
  # Prove the actual registry baseline; do not use a cached or supplied binary.
  cargo install --locked --version "=0.2.0" --root "$old_root" fexu-cli
  old_cli="$old_root/bin/fexu"
  [[ -x "$old_cli" ]] || old_cli="$old_root/bin/fexu.exe"
elif [[ -n "${FEXU_V012_CLI_PATH:-}" && ( -x "${FEXU_V012_CLI_PATH}" || -f "${FEXU_V012_CLI_PATH}" ) ]]; then
  old_cli="${FEXU_V012_CLI_PATH}"
elif old_cli="$(resolve_v012_cli)"; then
  :
else
  cargo install --locked --version 0.1.2 --root "$v012_root" fexu-cli
  old_cli="$(resolve_v012_cli)" || {
    echo "published fexu-cli 0.1.2 install did not produce bin/fexu or bin/fexu.exe" >&2
    exit 1
  }
fi
"$old_cli" --version | grep -F "$baseline"

cd "$work"
"$old_cli" new project V012FullstackPg --template fullstack
cd V012FullstackPg
project="$(pwd)"
if [[ "$baseline" == "0.2.0" ]]; then
  # Published 0.2.0 scaffolding omits error propagation for a Result.
  # Explicitly repair the disposable generated project before building the baseline.
  python3 "$runner" repair-v020-resolver "$project/src/project/mod.rs"
fi
if project_native="$(pwd -W 2>/dev/null)"; then
  project_native="${project_native//\\//}"
else
  project_native="$project"
fi

python3 "$runner" patch-database "$project_native/fexu.toml" "$DATABASE_URL"
export CARGO_TARGET_DIR="$work/app-target"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
admin_email="release-admin@example.test"
staff_email="release-staff@example.test"
admin_password="$(python3 -c 'import secrets; print("V012full!" + secrets.token_hex(16))')"
staff_password="$(python3 -c 'import secrets; print("V012staff!" + secrets.token_hex(16))')"
export FEXU_PASSWORD="$admin_password"
port="$(python3 "$runner" pick-port)"
python3 - "$project_native/fexu.toml" "$port" <<'PY'
import sys
from pathlib import Path
path = Path(sys.argv[1])
path.write_text(path.read_text().replace("port = 3000", f"port = {sys.argv[2]}"))
PY

"$old_cli" new app catalog
"$old_cli" generate model Author --app catalog
"$old_cli" migrate make initial_author --app catalog
"$old_cli" migrate up
"$old_cli" generate model Book --app catalog
python3 "$runner" patch-book "$project_native/apps/catalog/models/book.rs"
"$old_cli" migrate make initial_book --app catalog
"$old_cli" migrate up

author_id="$(python3 -c 'import uuid; print(uuid.uuid4())')"
book_id="$(python3 -c 'import uuid; print(uuid.uuid4())')"
python3 "$runner" seed-pg "$DATABASE_URL" "$author_id" "$book_id"

"$old_cli" admin create-user --email "$admin_email" --staff --superuser
FEXU_PASSWORD="$staff_password" "$old_cli" admin create-user --email "$staff_email" --staff
"$old_cli" auth create-role release_reader
"$old_cli" auth grant-permission --role release_reader books.read
"$old_cli" auth assign-role --email "$staff_email" --role release_reader

csrf_secret="$(python3 "$runner" csrf "$project_native")"
before_json="$work/before-snapshot.json"
python3 "$runner" snap-save "$before_json" --database-url "$DATABASE_URL"
before_snap="$(python3 "$runner" snap-count --database-url "$DATABASE_URL")"
export FEXU_CSRF_SECRET="$csrf_secret"
base="http://127.0.0.1:$port"
server_pid=""
cleanup_server() {
  if [[ -n "${server_pid:-}" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
    server_pid=""
  fi
}

cargo build --quiet
cargo run --quiet >/tmp/v012-fullstack-pg-before.log 2>&1 &
server_pid=$!
for _ in $(seq 1 180); do
  if curl -fsS "$base/admin/login" >/dev/null 2>&1; then
    break
  fi
  sleep 0.5
done
session_before="$(python3 "$runner" login "$base" "$admin_email" "$admin_password")"
python3 "$runner" session-check -- "$base" "$session_before" "$admin_email"
cleanup_server

python3 "$runner" rewrite "$project_native/Cargo.toml" "$repo_native/crates/fexu" "$project_native/src/project/mod.rs" --baseline "$baseline"
cargo check

set +e
upgrade_output=$("$new_cli" upgrade --check 2>&1)
upgrade_status=$?
set -e
printf '%s\n' "$upgrade_output"
if [[ "$upgrade_status" -ne 1 ]]; then
  grep -Fq 'project.file.modified Cargo.toml' <<<"$upgrade_output" || {
    echo "Expected upgrade --check to report modified tracked files (exit 1); got $upgrade_status" >&2
    exit 1
  }
fi

plan_output=$("$new_cli" migrate plan 2>&1)
printf '%s\n' "$plan_output"
python3 "$runner" snap-assert "$before_json" --database-url "$DATABASE_URL"

"$new_cli" generate model Shelf --app catalog
"$new_cli" migrate make add_shelf --app catalog
"$new_cli" migrate up
"$new_cli" check
python3 "$runner" post-upgrade-pg "$DATABASE_URL" "$staff_email" release_reader

cargo build --quiet
cargo run --quiet >/tmp/v012-fullstack-pg-after.log 2>&1 &
server_pid=$!
for _ in $(seq 1 180); do
  curl -fsS "$base/admin/login" >/dev/null 2>&1 && break
  sleep 0.5
done
python3 "$runner" session-check -- "$base" "$session_before" "$admin_email"
python3 "$runner" http-matrix "$base" "$admin_email" "$admin_password" "wrong-password-please"
cleanup_server

python3 "$runner" failed-migration "$project_native" "$new_cli" --database-url "$DATABASE_URL"

echo "PASS: published $baseline fullstack project upgraded on PostgreSQL with auth, sessions, HTTP and catalog data ($before_snap)"