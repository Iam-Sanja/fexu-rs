# Authentication and admin

The fullstack generator enables the server-rendered administration interface and authentication, but does not create a default administrator.

**Password-hashing compatibility:** Published Fexu v0.2.0 uses the RustCrypto `argon2` 0.5 series. The upcoming v0.3.0 development source is evaluating `argon2` 0.6. Existing password hashes use the PHC Argon2id encoding, including the salt and work parameters. Migration must preserve verification of hashes created by older releases; never reset existing account passwords or modify stored hashes merely because the dependency changes. The new dependency is not part of the v0.2.0 release tag.

## Create an administrator

After applying migrations:

```sh
fexu migrate plan
fexu migrate up
fexu admin create-user --email admin@example.com --staff --superuser
fexu dev
```

The CLI prompts for a password. Visit the development server's `/admin` route to sign in. Keep `--staff` when an account must access the admin; do not assume `--superuser` alone activates staff status.

## Roles and permissions

```sh
fexu auth create-role editor
fexu auth grant-permission books.read --role editor
fexu auth grant-permission books.update --role editor
fexu auth assign-role --email editor@example.com --role editor
fexu auth list-users
```

Model-level permissions, route authorization and CSRF are separate concerns. An app route added with `UrlSet` is not automatically protected because it shares the application with the admin.

For login, session expiry and form security, apply the framework's authentication checks to each relevant route. Use the [CLI reference](../reference/cli.md) for all user/role subcommands.
