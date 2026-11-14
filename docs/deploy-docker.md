# Docker bind address, public origin and trusted proxies

Three settings are independent.

| Setting | Meaning | Default |
| --- | --- | --- |
| `server.host` and `BIND_ADDRESS` | Socket the process binds. | `127.0.0.1:3000`. |
| `FEXU_ALLOWED_ORIGINS` | Exact browser origins accepted by CSRF and login. | Development loopback origins. Production starts empty. |
| `FEXU_TRUST_PROXY` and `FEXU_TRUSTED_PROXIES` | Which immediate peers may supply `X-Forwarded-For` for the client key. | Off, and an empty peer list. |

A wildcard bind is an explicit opt-in. Set `server.allow_wildcard_bind = true` in `fexu.toml`, or set `FEXU_ALLOW_WILDCARD_BIND=true` when `BIND_ADDRESS` is `0.0.0.0:port` or `[::]:port`. The bind address is not added to the origin allowlist. `Host`, `X-Forwarded-Host` and `X-Forwarded-Proto` are not public origins.

`FEXU_TRUST_PROXY=true` alone does not trust forwarded headers. The immediate peer address must also be listed in `FEXU_TRUSTED_PROXIES`. A `*` entry is ignored. `Origin: null` is rejected unless the browser also sends `Sec-Fetch-Site: same-origin`.

The example in `examples/deploy` is a multi-stage image. The runtime stage copies only `embedded-migrate`. Auth and job migration JSON is compiled into that binary, so the image does not need the Rust source tree or those JSON files. PostgreSQL has no published host port. Caddy publishes HTTPS on `127.0.0.1:443` only.

`embedded-migrate` proves the migration contract. A full application image must also copy the application's own templates, static files and upload directory when those are not embedded. Application migration directories remain files beside the application; an absent application directory is optional and is not reported as a successful framework migration.

Windows and Linux use the same embedded resources. `scripts/cold-start-embedded.sh` and `scripts/cold-start-embedded.ps1` copy `embedded-migrate` to a temporary directory, rename `crates/fexu-auth/migrations` and `crates/fexu-jobs/migrations` out of the tree, and run the binary. Linux CI runs the Bash script. Windows CI runs the PowerShell script. Both require `login=ok` and a reported job, not only exit code 0. The PostgreSQL 16 job runs the Bash script against `fexu_test_relocated`, then drops that database. A PostgreSQL URL is accepted only when its decoded database name starts with `fexu_test_` and its host is loopback. The URL user must already be a disposable test role. User, password, and query text cannot satisfy the check. The scripts do not read an ambient `DATABASE_URL`. Local PostgreSQL checks use `fexu::db::DisposablePostgres` and are ignored unless `DATABASE_URL` names an approved admin database. Docker is not required for that proof. The runtime image above is the container form of the same contract: the final stage contains the binary and CA certificates, not the migration JSON.
