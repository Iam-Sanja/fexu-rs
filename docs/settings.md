# Settings

[Overview](index.md) · Own values: [custom-settings.md](custom-settings.md) · **actual file:** `crates/fexu-migrations/src/project.rs`.

## Purpose

`fexu.toml` is the project file written for new projects. A directory that contains only `fexu.toml` is still loaded. A directory that contains both files is an error, and Fexu does not choose either database URL. `.fexu/project.json` remains the on-disk project marker. Database tables keep their existing `_fexu_*` names and migration IDs.

## Priority

1. Built-in defaults if a key is missing.
2. The value from `fexu.toml`.
3. A set environment variable replaces only the intended value.

An unknown section or key remains an error. Exceptions are `[custom.<name>]`, described in [custom-settings.md](custom-settings.md), and `[app.<name>]`, described in [app-registry.md](app-registry.md). `[custom]` and `[app]` without names are errors. `[app.<name>]` declares a migration directory and does not load a rust code. App settings remain under `[custom.<name>]` and do not override framework keys.

## Reading

```rust
fn load() -> Result<fexu::migrations::ProjectConfig, Box<dyn std::error::Error>> {
    let settings = fexu::migrations::Settings::discover(std::path::Path::new("."))?;
    let project = settings.project();
    let _url = project.resolved_database_url()?;
    let _address = project.bind_address()?;
    Ok(project.clone())
}
```

`discover` searches `fexu.toml` from the start directory upwards and calls `enforce_runtime`. `from_project` checks an already read `ProjectConfig` again. `project()` borrows the configuration. The generator writes the same sequence in `src/settings.rs`.

## Sections

Existing sections remain `project`, `server`, `database`, `admin`, `auth`, `templates`, `branding`, `static`, `storage`, `mail`, `jobs` and `api.docs`. Omitted settings retain their defaults: language `de` with fallback `en`, 12-hour sessions, a 30-minute idle timeout, templates disabled, color `#0f172a`, currency `EUR`, static files disabled, local storage under `media` with a 10 MiB limit, and mail backend `disabled`. `[jobs]` is optional. Without it, workers are permitted but never start automatically. See [workers.md](workers.md) for limits. `[api.docs]` is optional. Outside production the OpenAPI document and Swagger UI are on, try-it-out is off, and access is public. In production an omitted section mounts no documentation routes. See [openapi.md](openapi.md).

`server.host` must not be `0.0.0.0`. `templates.engine` is `tera`. Languages are `de` and `en`. `branding.currency` consists of three capital letters. `storage.backend` is `local`. Static prefixes must not cover Admin, API, Auth, Health and Uploads.

## Environment

| Variable | Effect |
|---|---|
| `DATABASE_URL` |Replaces the file URL if it is not empty. The value does not appear in `Debug`|
| `FEXU_ENV=production` | Turns on production mode. Any other or missing value remains development |
| `FEXU_DEBUG` | Only `true` or `false`. Invalid text is an error and is not replaced by the file |
| `FEXU_SMTP_PASSWORD` | SMTP password. It is not stored in `fexu.toml` |
| `FEXU_CSRF_SECRET` | 64 hex characters in production. The value is not logged |
| `FEXU_MFA_KEY` | 64 hex characters, 32 bytes, no prefix. Production requires it. Development uses `.fexu/mfa.key` when the variable is unset. The value is not logged |
| `BIND_ADDRESS` |Replaces `host:port`. A value starting with `0.0.0.0` is rejected|
| `FEXU_PUBLIC_ORIGIN` |Replaces `mail.public_origin` only when opening the mail service|
| `FEXU_SMTP_USERNAME` | SMTP user. If it is missing, the field remains empty |
| `FEXU_SESSION_HOURS` | Replaces the file when set |
| `FEXU_IDLE_MINUTES` | Replaces the file when set |
| `FEXU_COOKIE_SECURE` | `1` or `true` sets `Secure` on `fexu_session` and `fexu_anonymous`. In production this is the default |
| `FEXU_ALLOWED_ORIGINS` | Comma-separated origins. In production without this variable, the list is empty until `apply_bound_address` sets loopback |
| `FEXU_TRUST_PROXY` | `1` or `true` allows reading `X-Forwarded-For`. Alone it trusts no peer |
| `FEXU_TRUSTED_PROXIES` | Comma-separated immediate peer addresses allowed to supply `X-Forwarded-For`. Empty means no peer is trusted. A `*` entry is ignored |

A missing optional variable retains the file or default value. `DATABASE_URL` and `BIND_ADDRESS` are not `FEXU_` settings. A missing mandatory production specification remains an error: `FEXU_DEBUG` with invalid text, `BIND_ADDRESS` on `0.0.0.0`, and `server.debug = true` together with `FEXU_ENV=production`. Fexu does not switch debug silently to `false` in this case.

## Development and production

Development is the default. Generated projects set `debug = true` and bind to `127.0.0.1`. Production is selected only by `FEXU_ENV=production`; `enforce_runtime` then rejects enabled debug mode. `fexu check` and `fexu doctor` use the same runtime validation without opening a database or SMTP connection for that check. Success is `OK settings.runtime`; failure is `ERROR settings.runtime` with exit code 1.

## Debug and logs

`ProjectConfig` implements `Debug` by hand. The database URL with user and password becomes `schema://[redacted]@host`. A URL with `password=` becomes `[redacted]`. Mail passwords, CSRF secrets and custom values with `password`, `secret`, `token` or `credential` in the key name are rejected when read without writing the value into the message.

## Database target

CLI commands, diagnostics, workers, and the generated runtime call the same resolver:

1. `--database-url`, when the command has that option
2. `DATABASE_URL`
3. `database.url` in `fexu.toml`
4. `sqlite://db.sqlite3?mode=rwc` only when no project file is available and the command already allowed that default

An empty value or an unsupported scheme is an error. The resolver does not continue to the next source. A repeated `database.url` is also an error. Relative SQLite paths from the project file are resolved from the directory that contains `fexu.toml`, including when the command starts in a subdirectory. Override and environment URLs stay as written. Commands that open a database print `database: <redacted> (source)` on stderr. User names and passwords are replaced by `[redacted]`. `check`, `doctor`, `migrate status`, and `migrate plan` do not apply migrations.

`fexu worker` and `fexu jobs` still refuse an implicit SQLite file when no project and no URL are present.

## CLI

`fexu check` and `fexu doctor` read the file, check the runtime rules, and report the redacted database target. A set `DATABASE_URL` is reported as an override without printing its raw value. A static test is not combined.

## Example

```toml
[project]
name = "myshop"
template = "fullstack"
default_locale = "de"
fallback_locale = "en"

[server]
host = "127.0.0.1"
port = 3000
debug = true

[database]
url = "sqlite://db.sqlite3?mode=rwc"

[api.docs]
enabled = true
ui = "swagger"
path = "/api/docs"
schema_path = "/api/openapi.json"
schema = true
try_it_out = false
access = "public"
```

The workspace file with only `migrations_dir` and `schema_command` remains valid.

## Tests

`legacy_manifest_keeps_defaults`, `full_settings_parse_and_unknown_keys_fail` and `custom_settings_env_and_production_stay_explicit` in `project.rs`.

## Limitations

`resolved_database_url` returns the selected URL to the caller. Logging that value directly bypasses the redaction provided by `ProjectConfig::Debug` and `redact_database_url`. HTML error responses from view helpers are separate from project templates under `templates/errors/`.
