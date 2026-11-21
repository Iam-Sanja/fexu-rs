# Deploy with Docker and PostgreSQL

A production deployment needs a release build, a persistent PostgreSQL database, a protected application origin and durable storage for any uploaded files.

## Prepare the application

```sh
fexu check --deploy
fexu migrate plan
fexu build
```

Review migrations and create a verified, restorable backup before `fexu migrate up` against an existing PostgreSQL database. `fexu build` builds the release executable but does not itself start a web server.

## Deployment checklist

- Provide `DATABASE_URL` through a protected secret rather than a committed file.
- Keep application sessions and CSRF keys private. Production `FEXU_CSRF_SECRET` and `FEXU_MFA_KEY` are separate values.
- Use HTTPS at the reverse proxy; configure the public HTTPS origin consistently.
- Store uploaded files on a persistent volume, not only in the container's ephemeral filesystem.
- Use a dedicated PostgreSQL role/database and avoid exposing port 5432 publicly.
- Start the app only after compatible migrations have been applied.
- Validate login, logout, admin permissions, uploads and session persistence after a container restart.

The generated project provides Rust application source, not a universal production Compose manifest for every infrastructure layout. Build a Compose file appropriate to your infrastructure and test it in an isolated environment before deploying to a live shop.

See [settings](../reference/settings.md), [migrations](../topics/migrations.md) and [auth/admin](../topics/auth-and-admin.md).
