#!/usr/bin/env bash
# Calls the cold-start URL parser. It never opens a database connection.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck source=cold-start-url.sh
source "$root/scripts/cold-start-url.sh"

failed=0

expect_reject() {
  local label="$1"
  local url="$2"
  local needle="$3"
  local reason
  reason=$(cold_start_database_rejection "$url")
  case "$reason" in
    *"$needle"*)
      echo "reject $label"
      ;;
    "")
      echo "FAIL $label was accepted" >&2
      failed=1
      ;;
    *)
      echo "FAIL $label unexpected: $reason" >&2
      failed=1
      ;;
  esac
}

expect_allow() {
  local label="$1"
  local url="$2"
  local reason
  reason=$(cold_start_database_rejection "$url")
  if [ -n "$reason" ]; then
    echo "FAIL $label was rejected: $reason" >&2
    failed=1
  else
    echo "allow $label"
  fi
}

expect_reject "user" "postgres://fexu_test@127.0.0.1:5432/fexu" "database name"
expect_reject "password" "postgres://fexu:fexu_test@127.0.0.1:5432/production" "database name"
expect_reject "query" "postgres://fexu@127.0.0.1:5432/fexu?application_name=fexu_test" "database name"
expect_reject "scheme" "Postgres://fexu@127.0.0.1:5432/fexu" "database name"
expect_reject "exact-name" "postgres://fexu@127.0.0.1:5432/fexu_test" "database name"
expect_reject "remote" "postgres://fexu@203.0.113.10:5432/fexu_test_relocated" "loopback"
expect_reject "encoded-short" "postgres://fexu@127.0.0.1:5432/fexu%5ftest" "database name"
expect_reject "unclear" "postgres://fexu@127.0.0.1:5432/fexu_test_relocated/extra" "single database name"

expect_allow "relocated" "postgres://fexu@127.0.0.1:5432/fexu_test_relocated"
expect_allow "postgresql" "postgresql://fexu@localhost:5432/fexu_test_relocated"
expect_allow "encoded" "postgres://fexu@127.0.0.1:5432/%66exu_test_relocated"
expect_allow "ipv6" "postgres://fexu@[::1]:5432/fexu_test_relocated"

if [ "$failed" -ne 0 ]; then
  exit 1
fi
echo "cold-start URL policy passed"
