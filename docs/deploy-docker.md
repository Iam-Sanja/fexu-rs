# Docker bind address, public origin and trusted proxies

Three settings are independent.

| Setting | Meaning | Default |
| --- | --- | --- |
| `server.host` and `BIND_ADDRESS` | Socket the process binds. | `127.0.0.1:3000`. |
| `FEXU_ALLOWED_ORIGINS` | Exact browser origins accepted by CSRF and login. | Development loopback origins. Production starts empty. |
| `FEXU_TRUST_PROXY` and `FEXU_TRUSTED_PROXIES` | Which immediate peers may supply `X-Forwarded-For` for the client key. | Off, and an empty peer list. |

A wildcard bind is an explicit opt-in. Set `server.allow_wildcard_bind = true` in `fexu.toml`, or set `FEXU_ALLOW_WILDCARD_BIND=true` when `BIND_ADDRESS` is `0.0.0.0:port` or `[::]:port`. The bind address is not added to the origin allowlist. `Host`, `X-Forwarded-Host` and `X-Forwarded-Proto` are not public origins.

`FEXU_TRUST_PROXY=true` alone does not trust forwarded headers. The immediate peer address must also be listed in `FEXU_TRUSTED_PROXIES`. A `*` entry is ignored. `Origin: null` is rejected unless the browser also sends `Sec-Fetch-Site: same-origin`. `Forwarded` is not read as an origin or as a client address.

`fexu check --deploy` reads `BIND_ADDRESS` when it is set. A wildcard value is an error until `server.allow_wildcard_bind` or `FEXU_ALLOW_WILDCARD_BIND=true` makes it explicit. The OK line states that the bind address is not an origin. `FEXU_TRUST_PROXY=1` with only `*` is an error because no peer remains.

The disposable stack in `deploy/` is the Compose example. Its app sets `FEXU_ALLOW_WILDCARD_BIND=true` and does not set `FEXU_TRUST_PROXY`. PostgreSQL has no published port. The first database start runs `deploy/postgres/init-app-role.sh`, which creates `fexu_app` as `LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION` and grants `CONNECT` plus default table and sequence privileges for objects the owner creates afterwards. `/readyz` is `SELECT 1` and needs that `CONNECT`. Migrations use the owner role, not `fexu_app`. The local passwords `fexu_dev_only` and `fexu_app_dev_only` are defaults for this disposable database. An existing volume does not rerun the init script.

`deploy/Caddyfile` forwards `X-Forwarded-Proto` and `X-Forwarded-Host` from the proxy connection and leaves the browser `Origin` unchanged. `deploy/nginx-proxy-manager.example.conf` shows the same headers for NPM, including `X-Forwarded-For`, `Origin` and `Referer`. Neither file is a public origin allowlist. A real NPM process, a LAN browser, and a live proxy were not part of the automated checks. See [deploy/README.md](../deploy/README.md).

The example in `examples/deploy` is a multi-stage image. The runtime stage copies only `embedded-migrate`. Auth and job migration JSON is compiled into that binary, so the image does not need the Rust source tree or those JSON files. PostgreSQL has no published host port. Caddy publishes HTTPS on `127.0.0.1:443` only.

`embedded-migrate` proves the migration contract. A full application image must also copy the application's own templates, static files and upload directory when those are not embedded. Application migration directories remain files beside the application; an absent application directory is optional and is not reported as a successful framework migration.

Windows and Linux use the same embedded resources. `scripts/cold-start-embedded.sh` and `scripts/cold-start-embedded.ps1` copy `embedded-migrate` to a temporary directory, rename `crates/fexu-auth/migrations` and `crates/fexu-jobs/migrations` out of the tree, and run the binary. Linux CI runs the Bash script. Windows CI runs the PowerShell script. Both require `login=ok` and a reported job, not only exit code 0. The PostgreSQL 16 job runs the Bash script against `fexu_test_relocated`, then drops that database. A PostgreSQL URL is accepted only when it has no query string, its decoded database name starts with `fexu_test_`, and its host is the literal form `127.0.0.1`, `localhost`, or `[::1]`. The host is not percent-decoded. Every query parameter is refused, including `dbname`, `host`, `port`, and `user`. The URL user must already be a disposable test role. The scripts do not read `pg_roles` and do not verify that role's privileges. User, password, and host text cannot satisfy the database-name check. The scripts do not read an ambient `DATABASE_URL`. Local PostgreSQL checks use `fexu::db::DisposablePostgres` and are ignored unless `DATABASE_URL` names an approved admin database. Docker is not required for that proof. The runtime image above is the container form of the same contract: the final stage contains the binary and CA certificates, not the migration JSON.
