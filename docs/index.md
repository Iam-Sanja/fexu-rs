# Fexu Documentation

Fexu is a Rust web framework for building database-backed web applications. It provides model declarations, SQLite/PostgreSQL access, migrations, HTML and JSON endpoints, authentication, forms, a server-rendered admin, and background jobs.

**Latest published Fexu release: 0.2.0** (all 16 crates). **Next planned milestone: 0.3.0**, not yet released. The public GitHub Pages manual reflects current `main` and must label post-release development behavior. Match CLI/API examples to your installed version; see [documentation versions](documentation-versions.md).

## Start building

1. [Build your first Bookstore project, step by step](tutorial-first-project.md). For a short install guide, see [quickstart](quickstart.md).
2. [Create an app](create-app.md) and [define models](models.md).
3. [Configure SQLite or PostgreSQL](database.md) and [apply migrations](migrations.md).
4. [Build views](views.md), [routes](urls.md) and [templates](templates.md).
5. [Configure authentication](auth.md) and [the admin](admin.md).
6. [Deploy your application](deployment.md).

The generated fullstack project includes admin-facing functionality. The API template is available for smaller HTTP applications. Consult [project templates](project-templates.md) for the supported layouts.

## Guides by topic

| Topic | Guides |
| --- | --- |
| Projects and tooling | [Quickstart](quickstart.md), [CLI reference](cli.md), [Project generator](project-generator.md), [Development server](development-server.md), [Project structure](project-structure.md) |
| Configuration | [Configuration](configuration.md), [Settings](settings.md), [Custom settings](custom-settings.md) |
| Models and data | [Models](models.md), [Relations](relations.md), [Database queries](database.md), [Constraints](constraints.md), [Migrations](migrations.md), [Transactions](transactions.md) |
| HTTP and templates | [Routing](routing.md), [URLs](urls.md), [Views](views.md), [Web views](web-views.md), [Templates](templates.md) |
| Forms and validation | [Forms](forms.md), [Validation](validation.md), [Uploads](uploads.md) |
| Accounts and security | [Authentication](auth.md), [Sessions](sessions.md), [Permissions](permissions.md), [CSRF](csrf.md), [Security](security.md) |
| Admin interface | [Admin](admin.md), [Native admin](admin-native.md), [Admin inlines](admin-inlines.md), [Bulk actions](admin-bulk-actions.md), [Admin extensions](admin-extension-api.md) |
| Application services | [Jobs](jobs.md), [Workers](workers.md), [Mail](mail.md), [Cache](cache.md), [Storage](storage.md), [Static files](staticfiles.md) |
| APIs and reuse | [API](api.md), [OpenAPI](openapi.md), [Apps](apps.md), [Reusable apps](reusable-apps.md) |
| Operations | [Deployment](deployment.md), [Docker and reverse proxies](deploy-docker.md), [Testing](testing.md), [Fixtures](fixtures.md) |

## Command reference

The [CLI reference](cli.md) covers every command family defined by the development-source `fexu` CLI:

- `new`, `doctor`, `check`, `upgrade`, `dev`, `build`, `generate`
- `migrate`, `admin`, `auth`, `worker`, `jobs`
- `dumpdata`, `loaddata`, `manage`

For flags supported by a specific installed release, check `fexu --help` and `fexu <command> --help`.

## Supported database backends

SQLite is the default local backend and uses bundled SQLite via rusqlite. PostgreSQL is supported through the SQLx PostgreSQL driver. See the [database guide](database.md) for SQL/query semantics and the [deployment guide](deployment.md) for server setup.

## Code examples and API versions

Examples in a release-specific manual should be tested against that release's source and dependencies. Do not use an example from an unreleased branch as proof that it works with the currently published crates. The [version guide](documentation-versions.md) explains how to choose the right manual.
