#!/usr/bin/env bash
# Disposable Redis only; no external endpoint is accepted.
set -euo pipefail
name="fexu-cache-$(python3 -c 'import secrets; print(secrets.token_hex(8))')"
container=""
cleanup() {
  if [[ -n "$container" ]]; then docker rm -f -v "$container" >/dev/null 2>&1 || true; fi
}
trap cleanup EXIT
container="$(docker run -d --name "$name" -p 127.0.0.1::6379 redis:7-alpine --save '' --appendonly no)"
port="$(docker port "$container" 6379/tcp | sed -n 's/^127\.0\.0\.1://p')"
test -n "$port"
for _ in $(seq 1 60); do
  if docker exec "$container" redis-cli ping | grep -Fx PONG; then break; fi
  sleep 1
done
docker exec "$container" redis-cli ping | grep -Fx PONG
export FEXU_TEST_REDIS_URL="redis://127.0.0.1:$port/"
cargo test --locked -p fexu-cache --features redis-cache --test redis_contract redis_shared_contract -- --ignored --exact
docker stop "$container"
cargo test --locked -p fexu-cache --features redis-cache --test redis_contract redis_outage_contract -- --ignored --exact
