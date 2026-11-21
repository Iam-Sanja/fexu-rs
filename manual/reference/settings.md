# Project settings reference

Fexu uses `fexu.toml` for non-secret project settings. Generated projects additionally contain `src/project/settings.rs`, which loads the project config. Use protected environment variables for credentials.

## File: `fexu.toml`

The following is a representative **excerpt** of the generated configuration. Keep any additional generated sections.

```toml
migrations_dir = "migrations"
schema_command = "cargo run --quiet -- --print-schema"

[project]
name = "bookstore"
template = "fullstack"
layout_version = 2
apps_dir = "apps"
default_locale = "en"
fallback_locale = "en"

[database]
url = "sqlite://db.sqlite3?mode=rwc"

[server]
host = "127.0.0.1"
port = 3000
debug = true

[admin]
enabled = true
embedded_assets = true
site_title = "Bookstore Admin"
site_header = "Bookstore"

[auth]
enabled = true
session_hours = 12
idle_minutes = 30

[templates]
enabled = true
engine = "tera"
templates_dir = "templates"

[branding]
app_name = "Bookstore"
primary_color = "#0f172a"
currency = "EUR"

[static]
enabled = true
url_prefix = "/static"
directory = "static"
cache_seconds = 3600

[storage]
backend = "local"
directory = "media"
max_upload_size_mb = 10

[jobs]
enabled = true

[mail]
backend = "disabled"
public_origin = "http://127.0.0.1:3000"
reset_ttl_minutes = 30
```

## Important settings

| Section | Key | What it controls |
| --- | --- | --- |
| `project` | `name`, `template` | Project identity and generated template |
| `project` | `layout_version`, `apps_dir` | App layout and directory for **new** apps |
| `project` | `default_locale`, `fallback_locale` | Localized page rendering |
| `database` | `url` | SQLite or PostgreSQL connection |
| `server` | `host`, `port`, `debug` | Listener and debug behavior |
| `admin` | `enabled`, `embedded_assets`, `site_title` | Admin presentation |
| `auth` | `enabled`, `session_hours`, `idle_minutes` | Authentication/session configuration |
| `templates` | `engine`, `templates_dir` | Tera and template location |
| `static` | `url_prefix`, `directory` | Public static assets |
| `storage` | `backend`, `directory`, `max_upload_size_mb` | Upload storage |
| `mail` | `backend`, `public_origin` | Mail and external application URL |

## Environment

The generated `.env.example` documents environment-based configuration:

```dotenv
DATABASE_URL=sqlite://db.sqlite3?mode=rwc
BIND_ADDRESS=127.0.0.1:3000
# FEXU_CSRF_SECRET: 64 hexadecimal characters in production
# FEXU_MFA_KEY: independent 64 hexadecimal characters in production
# FEXU_PUBLIC_ORIGIN: public HTTPS origin for password reset links
```

Do not put secrets in `fexu.toml`, a published Docker image or Git. `fexu doctor` and `fexu check --deploy` help validate setup without applying migrations.

Read [first-project tutorial](../tutorial/first-project.md) for the generated Rust loader and [deployment](../how-to/deployment.md) for server setup.
