# Local deployment example

This stack is for a disposable machine. It is not a production deployment.
`docker compose up` in the repository root still starts only PostgreSQL on
`127.0.0.1`. This directory is separate.

Do not point `DATABASE_URL` at a database you need to keep. The backup rehearsal requires two distinct database names starting with `fexu_test_`, explicit literal loopback IP addresses and TCP ports, and no URL query or fragment. It refuses remote endpoints and strips libpq environment overrides. Use a fresh, empty disposable restore target and an explicitly authorized local test role; this is not a production backup tool. Python 3 and PostgreSQL client tools are required.

## What is here

- `Dockerfile` builds the `myshop` example. The image runs as a non-root user
  and copies the static assets, application migrations and migration CLI. It sets `FEXU_ALLOW_WILDCARD_BIND=true` next to `BIND_ADDRESS=0.0.0.0:8080`.
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
- `backup-restore.sh` performs an isolated PostgreSQL backup/restore rehearsal with a single-transaction restore; the URL guard is tested without connecting to any database.

The app container does not run migrations on startup. Run the included CLI as a separate step with the migration owner URL: `fexu migrate up --migrations-dir /home/fexu/migrations`. The image uses `FEXU_MYSHOP_STATIC_DIR` and `FEXU_MYSHOP_MEDIA_DIR`; the media directory and Caddy CA have named volumes. The proxy and database use separate networks with the app on both. `/healthz` only shows that the
process is up. `/readyz` runs `SELECT 1`.

`FEXU_TRUST_PROXY` is not set. Enabling it also requires `FEXU_TRUSTED_PROXIES`
with the proxy's immediate address. Read that address from the running
container. The framework does not require a fixed private IP. A `*` entry
trusts nobody. `Host`, `Forwarded` and `X-Forwarded-*` do not become allowed
origins, and they do not weaken the CSRF check.

The deployment CI scripts exercise a randomly named disposable Compose project, trusted local TLS, runtime role permissions, database outage, graceful app exit and session/volume persistence, plus a separate actual PostgreSQL dump/restore drill. Their results must be checked before claiming acceptance. These are HTTP client tests, not a live NPM or LAN browser test.

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
$env:FEXU_MFA_KEY = "<different 64 hexadecimal characters>"
docker compose -f deploy/compose.yaml up -d --wait db
docker compose -f deploy/compose.yaml run --rm --no-deps -e DATABASE_URL="postgres://fexu:fexu_dev_only@db:5432/fexu_test_deploy" app fexu migrate up --migrations-dir /home/fexu/migrations
docker compose -f deploy/compose.yaml up --build
```

Generate the secret outside the shell history when you can. The example
password `fexu_dev_only` is a local default, not a production secret.

## Deployment boundaries and verification (manual checklist)

These settings control **different trust boundaries**; none substitutes for another:

| Setting or header | Meaning | Security rule |
| --- | --- | --- |
| `BIND_ADDRESS` | Socket address inside the app container | `0.0.0.0` needs the explicit `FEXU_ALLOW_WILDCARD_BIND=true` opt-in; do not publish the app port publicly when a proxy is the sole entry point. |
| `FEXU_ALLOWED_ORIGINS` | Exact browser-facing scheme, hostname and port for cookie-authenticated writes | Configure the public URL explicitly; never derive it from `Host` or `X-Forwarded-Host`. |
| `FEXU_COOKIE_SECURE` / `FEXU_ENV` | Session-cookie TLS policy | Use HTTPS and secure cookies behind a TLS-terminating proxy; direct LAN HTTP is a separate development-only layout. |
| `FEXU_TRUST_PROXY` and `FEXU_TRUSTED_PROXIES` | Which immediate peers may supply a client address for login rate limiting | Leave disabled unless the exact proxy peer is known; this does **not** authorize a browser origin or bypass CSRF. |
| `Origin` / `Referer` | Browser-supplied information checked on unsafe requests | Forward unchanged. Never synthesize a trusted origin from `Host`, `Forwarded` or `X-Forwarded-*`. |

For a Portainer/Nginx Proxy Manager deployment, connect NPM and the app
to a shared *private* Docker network, use the app's **service name and
internal port** as the NPM upstream, and publish only NPM's intended
HTTP/HTTPS ports. Keep PostgreSQL on an app/database-only network without
a published `5432` port. Use an application database role for runtime
queries and an owner/migration role only when applying migrations.
Configure one exact public HTTPS origin and a real deployment secret.
The `deploy/compose.yaml` example starts Caddy, **not** NPM; do not
start a second proxy from that file as part of an existing NPM stack.

### Reproducible preflight on a disposable host

Run these from the repository root, against **test data only**. Set both independent secrets and apply the owner migrations first, as shown above. The
Compose example uses disposable development passwords and a local TLS
issuer; it is **not** a production template.

```powershell
# Provide a 64-character hexadecimal test secret through a safe secret
# mechanism before running this example.
docker compose -f deploy/compose.yaml config --quiet
docker compose -f deploy/compose.yaml up --build -d
docker compose -f deploy/compose.yaml ps
docker compose -f deploy/compose.yaml port proxy 443
docker compose -f deploy/compose.yaml port db 5432
```

Expected configuration: `proxy` has a loopback-only published port
(`127.0.0.1:8443`), while `db` has **no published port**; `app` also
has no published host port. `docker compose port db 5432` should not
return a host mapping. Do not confuse a container-internal listening port
with a host-published port.

For the HTTPS local-issuer test, trust Caddy's local test CA in an
isolated test browser **or** use an explicit certificate exception only
for a one-off local smoke test; never disable certificate validation in
production. Check `/healthz` (process only) and `/readyz`
(database reachability). Neither proves migrations or authentication
are ready. Run `fexu migrate up` with the migration/owner credentials
as a separate deliberate operation; startup must not do this for you.

### Browser and proxy negative-test matrix

Use a fresh cookie jar or browser profile, inspect the final response
headers after **all** middleware, and record the observed status and
request headers. Do not mark the matrix complete from static
configuration inspection alone.

| Request / situation | Required observation |
| --- | --- |
| Direct local HTTP with an explicitly allowed local origin | A normal CSRF-protected write works with its valid session and token; no trust of proxy headers is required. |
| HTTPS through Caddy or NPM with an exact allowed HTTPS origin | A fresh-session login and a valid authenticated form submit work, with `Secure` cookies. |
| No `Origin` header | Observe the documented Referer / Fetch Metadata policy; do not silently permit untrusted writes. |
| Foreign `Origin`, `Origin: null`, or a forged `Host` | Unsafe requests must not gain origin trust merely by reaching the reverse proxy. |
| Forged `Forwarded` / `X-Forwarded-Host` / `X-Forwarded-Proto` from a client | The headers must not add allowed origins or disable CSRF. |
| Missing or incorrect CSRF token | Cookie-authenticated writes remain rejected. |
| App not reachable or database unavailable | Distinguish process failure from `/readyz` database failure; do not claim readiness from `/healthz` alone. |

The NPM integration, real-browser origin headers, direct LAN HTTP and
live HTTPS scenarios have **not** been verified by this documentation
change. Their observed results, including the chosen origin and proxy
configuration (but **not** secrets or cookies), are still required to
finish issue #78.
