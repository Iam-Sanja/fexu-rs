#!/usr/bin/env bash
# Published fexu-cli 0.1.2 fullstack -> current-checkout upgrade with auth, HTTP and data.
set -euo pipefail

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
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
chmod 700 "$work"

old_root="$work/published-cli"
new_cli="$repo/target/debug/fexu"
mkdir -p "$old_root"
if [[ ! -x "$new_cli" ]]; then
  echo "Build the current CLI first: cargo build -p fexu-cli --bin fexu" >&2
  exit 1
fi

cache_cli="$repo/target/v012-cli/bin/fexu"
[[ -x "$cache_cli" ]] || cache_cli="$repo/target/v012-cli/bin/fexu.exe"
if [[ -n "${FEXU_V012_CLI_PATH:-}" && ( -x "${FEXU_V012_CLI_PATH}" || -f "${FEXU_V012_CLI_PATH}" ) ]]; then
  old_cli="${FEXU_V012_CLI_PATH}"
elif [[ -f "$cache_cli" ]]; then
  old_cli="$cache_cli"
else
  cargo install --locked --version 0.1.2 --root "$repo/target/v012-cli" fexu-cli
  old_cli="$cache_cli"
  [[ -f "$old_cli" ]] || old_cli="$repo/target/v012-cli/bin/fexu.exe"
fi
"$old_cli" --version | grep -F '0.1.2'

cd "$work"
"$old_cli" new project V012Fullstack --template fullstack
cd V012Fullstack
project="$(pwd)"
if project_native="$(pwd -W 2>/dev/null)"; then
  project_native="${project_native//\\//}"
else
  project_native="$project"
fi
db="$project/db.sqlite3"
export DATABASE_URL="sqlite://db.sqlite3?mode=rwc"
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
python3 -c "import sqlite3; sqlite3.connect('db.sqlite3').close()"
"$old_cli" migrate make initial_author --app catalog
"$old_cli" migrate up
"$old_cli" generate model Book --app catalog
python3 "$runner" patch-book "$project_native/apps/catalog/models/book.rs"
"$old_cli" migrate make initial_book --app catalog
"$old_cli" migrate up

author_id="$(python3 -c 'import uuid; print(uuid.uuid4())')"
book_id="$(python3 -c 'import uuid; print(uuid.uuid4())')"
python3 "$runner" seed "db.sqlite3" "$author_id" "$book_id"

"$old_cli" admin create-user --email "$admin_email" --staff --superuser
FEXU_PASSWORD="$staff_password" "$old_cli" admin create-user --email "$staff_email" --staff
"$old_cli" auth create-role release_reader
"$old_cli" auth grant-permission --role release_reader books.read
"$old_cli" auth assign-role --email "$staff_email" --role release_reader

csrf_secret="$(python3 "$runner" csrf "$project_native")"
before_snap="$(python3 "$runner" snap-count "db.sqlite3")"
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
trap 'cleanup_server; rm -rf "$work"' EXIT

cargo build --quiet
cargo run --quiet >/tmp/v012-fullstack-before.log 2>&1 &
server_pid=$!
for _ in $(seq 1 180); do
  if curl -fsS "$base/admin/login" >/dev/null 2>&1; then
    break
  fi
  sleep 0.5
done
session_before="$(python3 "$runner" login "$base" "$admin_email" "$admin_password")"
cleanup_server

python3 "$runner" rewrite "$project_native/Cargo.toml" "$repo_native/crates/fexu" "$project_native/src/project/mod.rs"
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

before_db="$(python3 -c "import hashlib, pathlib; print(hashlib.sha256(pathlib.Path('db.sqlite3').read_bytes()).hexdigest())")"
plan_output=$("$new_cli" migrate plan 2>&1)
printf '%s\n' "$plan_output"
after_db="$(python3 -c "import hashlib, pathlib; print(hashlib.sha256(pathlib.Path('db.sqlite3').read_bytes()).hexdigest())")"
[[ "$before_db" == "$after_db" ]]

"$new_cli" generate model Shelf --app catalog
"$new_cli" migrate make add_shelf --app catalog
"$new_cli" migrate up
"$new_cli" check
python3 "$runner" post-upgrade-db "db.sqlite3"

cargo build --quiet
cargo run --quiet >/tmp/v012-fullstack-after.log 2>&1 &
server_pid=$!
for _ in $(seq 1 180); do
  curl -fsS "$base/admin/login" >/dev/null 2>&1 && break
  sleep 0.5
done
python3 "$runner" http-matrix "$base" "$admin_email" "$admin_password" "wrong-password-please"
python3 "$runner" session-check "$base" "$session_before"
cleanup_server

python3 "$runner" failed-migration "$project_native" "$new_cli" "db.sqlite3"

echo "PASS: published 0.1.2 fullstack project upgraded with auth, sessions, HTTP and catalog data ($before_snap)"