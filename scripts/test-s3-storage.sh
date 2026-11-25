#!/usr/bin/env bash
# Only a disposable, TLS-only loopback bucket; no external endpoint accepted.
set -euo pipefail
work="$(mktemp -d)"
container=""
cleanup() {
  if [[ -n "$container" ]]; then docker rm -f -v "$container" >/dev/null 2>&1 || true; fi
  rm -rf "$work"
}
trap cleanup EXIT
mkdir "$work/certs"
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
  -keyout "$work/ca.key" -out "$work/ca.crt" -days 1 -subj /CN=FexuS3TestCA \
  -addext 'basicConstraints=critical,CA:TRUE' -addext 'keyUsage=critical,keyCertSign,cRLSign'
openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
  -keyout "$work/certs/private.key" -out "$work/server.csr" -subj /CN=localhost
printf 'subjectAltName=IP:127.0.0.1\nextendedKeyUsage=serverAuth\nbasicConstraints=critical,CA:FALSE\n' >"$work/server.ext"
openssl x509 -req -in "$work/server.csr" -CA "$work/ca.crt" -CAkey "$work/ca.key" \
  -CAcreateserial -out "$work/certs/public.crt" -days 1 -extfile "$work/server.ext"
chmod 755 "$work/certs"
chmod 644 "$work/certs/private.key"
export FEXU_S3_ACCESS_KEY="$(openssl rand -hex 16)"
export FEXU_S3_SECRET_KEY="$(openssl rand -hex 24)"
container="$(docker run -d -p 127.0.0.1::9000 \
  -e MINIO_ROOT_USER="$FEXU_S3_ACCESS_KEY" -e MINIO_ROOT_PASSWORD="$FEXU_S3_SECRET_KEY" \
  -v "$work/certs:/certs:ro" ghcr.io/coollabsio/minio:RELEASE.2025-10-15T17-29-55Z@sha256:69b55a1c1c5dc285ce04db96689f5b2102317fc77a50680a1874ca6efd1c87f9 \
  server /data --certs-dir /certs --address :9000)"
port="$(docker port "$container" 9000/tcp | sed -n 's/^127\.0\.0\.1://p')"
test -n "$port"
export FEXU_TEST_S3_ENDPOINT="https://127.0.0.1:$port"
export FEXU_TEST_S3_CA="$work/ca.crt"
for _ in $(seq 1 60); do
  if curl --silent --fail --cacert "$work/ca.crt" "$FEXU_TEST_S3_ENDPOINT/minio/health/ready" >/dev/null; then break; fi
  sleep 1
done
curl --silent --fail --cacert "$work/ca.crt" "$FEXU_TEST_S3_ENDPOINT/minio/health/ready" >/dev/null
# The runner's AWS CLI creates the private fixture bucket with verified TLS.
export AWS_ACCESS_KEY_ID="$FEXU_S3_ACCESS_KEY" AWS_SECRET_ACCESS_KEY="$FEXU_S3_SECRET_KEY"
export AWS_DEFAULT_REGION=us-east-1 AWS_CA_BUNDLE="$work/ca.crt" AWS_EC2_METADATA_DISABLED=true
aws --endpoint-url "$FEXU_TEST_S3_ENDPOINT" s3api create-bucket --bucket fexu-test-media >/dev/null
cargo test --locked -p fexu-storage --features s3-storage --lib
cargo test --locked -p fexu-storage --features s3-storage --lib s3_cleanup_ -- --ignored --test-threads=1
cargo test --locked -p fexu-storage --features s3-storage --test s3_contract -- --ignored --test-threads=1 --skip s3_restart_contract

# Preserve the same disposable container and its data across a real restart.
export FEXU_TEST_S3_RESTART_STATE="$work/restart-state"
FEXU_TEST_S3_RESTART_PHASE=seed cargo test --locked -p fexu-storage --features s3-storage --test s3_contract -- s3_restart_contract --ignored --exact
docker restart "$container" >/dev/null
port="$(docker port "$container" 9000/tcp | sed -n 's/^127\.0\.0\.1://p')"
test -n "$port"
export FEXU_TEST_S3_ENDPOINT="https://127.0.0.1:$port"
for _ in $(seq 1 60); do
  if curl --silent --fail --cacert "$work/ca.crt" "$FEXU_TEST_S3_ENDPOINT/minio/health/ready" >/dev/null; then break; fi
  sleep 1
done
curl --silent --fail --cacert "$work/ca.crt" "$FEXU_TEST_S3_ENDPOINT/minio/health/ready" >/dev/null
FEXU_TEST_S3_RESTART_PHASE=verify cargo test --locked -p fexu-storage --features s3-storage --test s3_contract -- s3_restart_contract --ignored --exact
