# Deployment and operation

[Overview](index.md) · Local SQLite remains the default. `compose.yaml` starts PostgreSQL on `127.0.0.1`. [deploy/README.md](https://github.com/Iam-Sanja/fexu-rs/blob/main/deploy/README.md) is a disposable app, proxy and database example. PR #264 passed the disposable Docker HTTPS and PostgreSQL recovery CI; this is not production certification.

## Local Development (Implemented)

SQLite is the standard database. Rust and Cargo are enough. Docker and PostgreSQL do not have to be installed.

```powershell
cargo run -p myshop -- --bootstrap
cargo run -p myshop
```

A generated project starts with `fexu dev` in the project directory. Windows and Linux use the same command. The server starts without automatic migration. First run `fexu migrate up`. Generated applications do not bind to `0.0.0.0`. `myshop --bootstrap` does not change the file. The file `db.sqlite3` is in the working directory. An older bootstrap file without `_fexu_migrations` is not taken over and not deleted.

The admin does not need a node process. `cargo build` is sufficient. Put `FEXU_CSRF_SECRET` and `FEXU_MFA_KEY` into production. Each value is 64 hexadecimal characters. The MFA key is not derived from the CSRF secret. The `.fexu/csrf.key` and `.fexu/mfa.key` files are only local sources. Production should set `FEXU_ENV=production` or `FEXU_COOKIE_SECURE=1`, and `FEXU_ALLOWED_ORIGINS` to the public `https` origins. A reverse proxy must forward `Origin` and `Referer` for cookie-authenticated unsafe requests. `FEXU_TRUST_PROXY` only changes the login rate-limit client address. It does not add an origin and it does not turn off `Secure`. Do not log `Cookie`, `x-csrf-token`, or `Authorization`.

`fexu check --deploy` is read-only. It reports debug, bind address, cookie `Secure`, the presence and shape of `FEXU_CSRF_SECRET` without printing it, https origins, pending migrations, and the proxy flag. `FEXU_TRUST_PROXY=1` must be paired with `FEXU_TRUSTED_PROXIES` containing explicitly trusted immediate peer addresses. Without configured peers no peer is trusted; wildcard entries are ignored. The deployment check reports the configured peer count without exposing addresses. The check does not migrate.

`fexu::serve` stops the process on Ctrl+C and, on Unix, on SIGTERM. `FEXU_LOG_FORMAT=json` selects JSON logs. The access log records method, matched route template (or `<unmatched>`), status, elapsed time and `x-request-id`. Raw dynamic paths and query strings are not logged by this event. A single inbound `x-request-id` is kept only when it is 8 to 64 ASCII letters, digits or hyphens. It remains untrusted caller data. `check --deploy` requires exact HTTPS origin syntax, distinct CSRF/MFA secrets, canonical proxy IP peers when trust is enabled, and an inspectable database. It cannot verify that an external worker is running. OpenTelemetry is not bundled. `/healthz` does not touch the database. `/readyz` runs `SELECT 1` and does not migrate. `App::strict_migrations` refuses startup when migrations are pending and does not apply them.

A different `BIND_ADDRESS`, in particular `0.0.0.0:3000`, can make the service accessible on other networks. `ProjectConfig::bind_address` rejects `0.0.0.0` and `::` unless `server.allow_wildcard_bind = true` or `FEXU_ALLOW_WILDCARD_BIND=true`. That opt-in does not add the bind address to `FEXU_ALLOWED_ORIGINS`. With the anonymous demo model, this is not an Internet deployment. Inside `deploy/compose.yaml` the app listens on `0.0.0.0` only on the Compose network and sets the opt-in. The published port is the proxy on `127.0.0.1:8443`. The database port is not published. The app connects as `fexu_app`, which is `LOGIN NOSUPERUSER`. Migrations stay on the owner role `fexu`.

## Optional PostgreSQL (Implemented)

`compose.yaml` starts `postgres:16-alpine` with database, user, volume and `pg_isready`. The port is limited to `127.0.0.1:5432:5432`. That file does not build the API. The disposable image, proxy and unpublished database live in [deploy/README.md](https://github.com/Iam-Sanja/fexu-rs/blob/main/deploy/README.md).

```powershell
docker compose up -d db
$env:DATABASE_URL="postgres://fexu:fexu_dev_only@127.0.0.1:5432/fexu"
cargo run -p myshop -- --bootstrap
cargo run -p myshop
```

The same registered application then speaks PostgreSQL. The handlers remain unchanged. Docker is an optional tool for this server, not the prerequisite for Fexu.

## Production architecture (not a production deployment)

`deploy/` is a local example: multi-stage image, non-root user, Caddy on `127.0.0.1:8443`, `/readyz`, graceful shutdown and a separate migration step. The isolated Compose HTTPS and restore checks passed on PR #264. It is not a production deployment. Still required before calling a host production:

```text
TLS-Client → Reverse Proxy → Fexu-App → PostgreSQL
                                  └→ Log-/Metrics-Ziel
```

1. A separately managed PostgreSQL, not the unpublished example database.
2. Forwarded headers only from trusted proxies. Enable `FEXU_TRUST_PROXY=1` only with a deliberate `FEXU_TRUSTED_PROXIES` allow-list; otherwise forwarded client addresses are not trusted.
3. HTTPS on the proxy, with session cookies and CSRF. The example uses Caddy's local issuer.
4. Secrets from the operational environment, not from the image or the repository.
5. Schema changes through `fexu migrate up`, not at process start. `down` is not that step.
6. No productive `allow_anonymous=true` without a security review.
7. Timeouts, connection limits and a metrics target. OpenTelemetry is not bundled.

SQLite can carry local development and testing. For several app processes, it is not the intended production database: the pool has a connection, and write accesses serialize through the file. PostgreSQL remains the supported server backend as soon as it is operated.

## Docker, Portainer and Nginx Proxy Manager

`deploy/compose.yaml` uses an internal network and Caddy. Portainer is not part of this repository. `deploy/nginx-proxy-manager.example.conf` is a header snippet for one NPM Proxy Host. It is not started by Compose.

Four layouts use the same rules:

1. Direct local HTTP. Bind `127.0.0.1:3000`. Leave `FEXU_ALLOW_WILDCARD_BIND` and `FEXU_TRUST_PROXY` unset. Set `FEXU_ALLOWED_ORIGINS` to the loopback `http` origins you actually open. `Secure` cookies stay off only while `FEXU_COOKIE_SECURE` is unset and `FEXU_ENV` is not `production`.
2. Docker Compose with PostgreSQL. `deploy/compose.yaml` binds the app to `0.0.0.0:8080` with `FEXU_ALLOW_WILDCARD_BIND=true`, publishes only Caddy on `127.0.0.1:8443`, and does not publish PostgreSQL. The runtime role is `fexu_app`. The owner `fexu` remains for migrations. Both passwords in the file are local defaults.
3. HTTPS behind Nginx Proxy Manager. Terminate TLS in NPM. Forward `Host`, `X-Forwarded-Proto`, `X-Forwarded-Host`, `X-Forwarded-For`, `Origin` and `Referer` with the snippet. Set `FEXU_ALLOWED_ORIGINS` to that public `https` origin and `FEXU_COOKIE_SECURE=1`.
4. A trusted proxy peer is optional and separate. Set `FEXU_TRUST_PROXY=1` only together with `FEXU_TRUSTED_PROXIES` set to the proxy's immediate address. Read that address from the running container. The framework does not require a fixed private IP, and `*` trusts nobody.

Untrusted `Host`, `Forwarded`, `X-Forwarded-Host`, `X-Forwarded-Proto` and `X-Forwarded-For` never become an allowed origin. CSRF still requires the session cookie and the matching token. An `http` origin is rejected when the `Secure` cookie flag is on.

## Acceptance criteria

The local SQLite startup does not need a database server. An optional PostgreSQL setup remains limited to localhost. Secrets do not appear in the log. `deploy/` does not publish the database and does not migrate on startup.

## Serving the admin

`/admin` renders HTML from the binary. Unknown asset paths are 404. `/admin/api` remains the JSON API. Paths with `..` are not read as files.
