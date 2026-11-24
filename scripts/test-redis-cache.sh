#!/usr/bin/env bash
# Disposable Redis only; no external endpoint is accepted.
set -euo pipefail
name="fexu-cache-$(python3 -c 'import secrets; print(secrets.token_hex(8))')"
work="$(mktemp -d)"
container=""
tls_container=""
restart_pid=""
cleanup() {
  if [[ -n "$restart_pid" ]]; then
    kill "$restart_pid" 2>/dev/null || true
    wait "$restart_pid" 2>/dev/null || true
  fi
  if [[ -n "$container" ]]; then docker rm -f -v "$container" >/dev/null 2>&1 || true; fi
  if [[ -n "$tls_container" ]]; then docker rm -f -v "$tls_container" >/dev/null 2>&1 || true; fi
  rm -rf "$work"
}
trap cleanup EXIT
wait_plain() {
  for _ in $(seq 1 60); do
    if docker exec "$container" redis-cli ping | grep -Fxq PONG; then return 0; fi
    sleep 1
  done
  docker logs "$container" >&2
  return 1
}
run_test() {
  cargo test --locked -p fexu-cache --features redis-cache --test redis_contract "$1" -- --ignored --exact
}
container="$(docker run -d --name "$name" -p 127.0.0.1::6379 redis:7-alpine --save '' --appendonly no)"
port="$(docker port "$container" 6379/tcp | sed -n 's/^127\.0\.0\.1://p')"
test -n "$port"
wait_plain
export FEXU_TEST_REDIS_URL="redis://127.0.0.1:$port/"
run_test redis_shared_contract
run_test redis_connection_reuse_contract
docker stop "$container"
run_test redis_outage_contract
docker start "$container"
wait_plain

# Keep the same Rust process and handles alive while Docker stops/restarts.
export FEXU_TEST_REDIS_MARKERS="$work/markers"
mkdir "$FEXU_TEST_REDIS_MARKERS"
run_test redis_restart_contract >"$work/restart.log" 2>&1 &
restart_pid=$!
wait_marker() {
  for _ in $(seq 1 60); do
    if [[ -f "$FEXU_TEST_REDIS_MARKERS/$1" ]]; then return 0; fi
    if ! kill -0 "$restart_pid" 2>/dev/null; then break; fi
    sleep 1
  done
  cat "$work/restart.log" >&2
  return 1
}
wait_marker ready
docker stop "$container"
touch "$FEXU_TEST_REDIS_MARKERS/stopped"
wait_marker observed
docker start "$container"
wait_plain
touch "$FEXU_TEST_REDIS_MARKERS/restarted"
if ! wait "$restart_pid"; then cat "$work/restart.log" >&2; exit 1; fi
restart_pid=""
cat "$work/restart.log"

# A private test CA, a server certificate for loopback, and a separate untrusted CA.
mkdir "$work/tls" "$work/empty-certs"
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
  -keyout "$work/ca.key" -out "$work/tls/ca.crt" -days 1 -subj /CN=FexuTestCA \
  -addext 'basicConstraints=critical,CA:TRUE' -addext 'keyUsage=critical,keyCertSign,cRLSign'
openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
  -keyout "$work/tls/server.key" -out "$work/server.csr" -subj /CN=localhost
printf 'subjectAltName=IP:127.0.0.1\nextendedKeyUsage=serverAuth\nbasicConstraints=critical,CA:FALSE\n' >"$work/server.ext"
openssl x509 -req -in "$work/server.csr" -CA "$work/tls/ca.crt" -CAkey "$work/ca.key" \
  -CAcreateserial -out "$work/tls/server.crt" -days 1 -extfile "$work/server.ext"
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
  -keyout "$work/untrusted.key" -out "$work/untrusted.crt" -days 1 -subj /CN=UnrelatedTestCA \
  -addext 'basicConstraints=critical,CA:TRUE' -addext 'keyUsage=critical,keyCertSign,cRLSign'
# The key is disposable and accessible only in the private parent directory
# and the read-only bind mount used by this isolated container.
chmod 755 "$work/tls"
chmod 644 "$work/tls/server.key"
tls_container="$(docker run -d --name "$name-tls" -p 127.0.0.1::6379 \
  -v "$work/tls:/fixture:ro" redis:7-alpine --port 0 --tls-port 6379 \
  --tls-cert-file /fixture/server.crt --tls-key-file /fixture/server.key \
  --tls-ca-cert-file /fixture/ca.crt --tls-auth-clients no --save '' --appendonly no)"
tls_port="$(docker port "$tls_container" 6379/tcp | sed -n 's/^127\.0\.0\.1://p')"
test -n "$tls_port"
for _ in $(seq 1 60); do
  if docker exec "$tls_container" redis-cli --tls --cacert /fixture/ca.crt ping | grep -Fxq PONG; then break; fi
  sleep 1
done
docker exec "$tls_container" redis-cli --tls --cacert /fixture/ca.crt ping | grep -Fx PONG
export FEXU_TEST_REDIS_TLS_URL="rediss://127.0.0.1:$tls_port/"
export SSL_CERT_DIR="$work/empty-certs"
SSL_CERT_FILE="$work/tls/ca.crt" FEXU_TEST_REDIS_TLS_TRUSTED=yes run_test redis_tls_contract
SSL_CERT_FILE="$work/untrusted.crt" FEXU_TEST_REDIS_TLS_TRUSTED=no run_test redis_tls_contract
cargo test --locked -p fexu-cache --features redis-cache --test redis_contract redis_configuration_rejects_insecure_urls_and_unbounded_timeout -- --exact
