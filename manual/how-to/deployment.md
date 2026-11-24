# Deploy with Docker and PostgreSQL

A production deployment needs a release build, a persistent PostgreSQL database, a protected application origin and durable storage for any uploaded files.

## Prepare the application

```sh
fexu check --deploy
fexu migrate plan
fexu build
```

Review migrations and create a verified, restorable backup before `fexu migrate up` against an existing PostgreSQL database. `fexu build` builds the release executable but does not itself start a web server.

`fexu check --deploy` checks exact HTTPS origin syntax (no path, credentials, query or fragment), separate CSRF/MFA keys, explicit IP peers for trusted proxies, and database/migration readiness without applying migrations. It does not verify that a worker is running. Standard access logs use route templates, not dynamic path values or query strings; application logs and caller-supplied request IDs must not contain secrets.

## Deployment checklist

- Provide `DATABASE_URL` through a protected secret rather than a committed file.
- Keep application sessions and CSRF keys private. Production `FEXU_CSRF_SECRET` and `FEXU_MFA_KEY` are separate values.
- Use HTTPS at the reverse proxy; configure the public HTTPS origin consistently.
- Store uploaded files on a persistent volume, not only in the container's ephemeral filesystem.
- Use a dedicated PostgreSQL role/database and avoid exposing port 5432 publicly.
- Start the app only after compatible migrations have been applied.
- Validate login, logout, admin permissions, uploads and session persistence after a container restart.

The repository's disposable Myshop Compose example separates the runtime and migration roles, supplies both independent secrets, and mounts persistent media. See [the deployment example](https://github.com/Iam-Sanja/fexu-rs/tree/main/deploy) for the isolated smoke-test procedure. It is not a production backup or deployment template.

The generated project provides Rust application source, not a universal production Compose manifest for every infrastructure layout. Build a Compose file appropriate to your infrastructure and test it in an isolated environment before deploying to a live shop.

See [settings](../reference/settings.md), [migrations](../topics/migrations.md) and [auth/admin](../topics/auth-and-admin.md).
