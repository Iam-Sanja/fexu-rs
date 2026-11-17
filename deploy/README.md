# Local deployment example

This stack is for a disposable machine. It is not a production deployment.
`docker compose up` in the repository root still starts only PostgreSQL on
`127.0.0.1`. This directory is separate.

Do not point `DATABASE_URL` at a database you need to keep. The backup script
refuses a database name that does not contain `fexu_test`.

## What is here

- `Dockerfile` builds the `myshop` example. The image runs as a non-root user
  and sets `FEXU_ALLOW_WILDCARD_BIND=true` next to `BIND_ADDRESS=0.0.0.0:8080`.
  Generated applications still reject that bind unless the same opt-in is set.
- `compose.yaml` puts PostgreSQL on an internal network, publishes the proxy
  only on `127.0.0.1:8443`, and does not publish the database port. The app
  uses `fexu_app`. The owner role `fexu` is the migration role.
- `postgres/init-app-role.sh` creates that role on the first database start.
  It is `LOGIN` and not a superuser, and it receives `CONNECT` plus default
  privileges on tables and sequences created later by the owner.
- `Caddyfile` terminates TLS with Caddy's local issuer. It forwards
  `X-Forwarded-Proto` and `X-Forwarded-Host` and does not invent an origin.
- `nginx-proxy-manager.example.conf` is a header snippet for one NPM Proxy
  Host. Compose does not start NPM.
- `backup-restore.sh` dumps and restores a database whose name contains
  `fexu_test`.

The app container does not run migrations on startup. Run
`fexu migrate up` as a separate step. `/healthz` only shows that the
process is up. `/readyz` runs `SELECT 1`.

`FEXU_TRUST_PROXY` is not set. Enabling it also requires `FEXU_TRUSTED_PROXIES`
with the proxy's immediate address. Read that address from the running
container. The framework does not require a fixed private IP. A `*` entry
trusts nobody. `Host`, `Forwarded` and `X-Forwarded-*` do not become allowed
origins, and they do not weaken the CSRF check.

No container from this file was started while the files were added if Docker
is not installed on the machine that prepared them. A real browser session
and a live Nginx Proxy Manager were not part of the automated checks.

## Direct local HTTP

```text
BIND_ADDRESS=127.0.0.1:3000
FEXU_ALLOWED_ORIGINS=http://127.0.0.1:3000,http://localhost:3000
```

Leave `FEXU_ALLOW_WILDCARD_BIND`, `FEXU_COOKIE_SECURE` and `FEXU_TRUST_PROXY`
unset. An `http` origin is rejected once `FEXU_COOKIE_SECURE=1` or
`FEXU_ENV=production` is set. Add a LAN origin only as an exact `http` URL
while cookies are not `Secure`. Do not infer it from `Host`.

## HTTPS behind a proxy

```text
BIND_ADDRESS=0.0.0.0:8080
FEXU_ALLOW_WILDCARD_BIND=true
FEXU_ENV=production
FEXU_COOKIE_SECURE=1
FEXU_ALLOWED_ORIGINS=https://app.example.test
```

`https://app.example.test` is a placeholder. `FEXU_TRUST_PROXY=1` is a
separate choice and needs the proxy's immediate peer address in
`FEXU_TRUSTED_PROXIES`. The Compose file uses the Caddy layout and does not
enable that flag. The owner URL for migrations is
`postgres://fexu:<owner password>@db:5432/fexu_test_deploy` on the Compose
network. Do not publish port 5432 to reach it.

## Commands

```powershell
$env:FEXU_CSRF_SECRET = "<64 hexadecimal characters>"
docker compose -f deploy/compose.yaml up --build
```

Generate the secret outside the shell history when you can. The example
password `fexu_dev_only` is a local default, not a production secret.
