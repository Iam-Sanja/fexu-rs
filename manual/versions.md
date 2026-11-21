# Documentation versions

Each published Fexu release gets its **own frozen manual**. Use the manual matching the Fexu version installed in your Cargo project, especially for CLI flags, API signatures and migration instructions.

## Choose the right version

| Version | What it documents |
| --- | --- |
| **0.1.2** | Latest published release as of 21 November 2026. Installation: `cargo install fexu-cli --version 0.1.2 --locked` |
| **0.2.0** | Next planned release. Its manual is published only after the 0.2.0 release itself is available. |
| **Development source** | Unreleased `main`. Use a matching source checkout; public APIs may differ from published crates. |

When the documentation website is enabled, the version selector will let you switch between released versions. The `stable` URL will point to the latest **approved** release manual, not automatically to `main`.

## Install the version you are reading about

For the published 0.1.2 release:

```sh
cargo install fexu-cli --version 0.1.2 --locked
cargo add fexu@0.1.2
```

After a release, the matching version page will contain its exact installation commands. Code examples, flags and signatures on a versioned page apply to that version only.

## Find the right instructions

- [Quickstart](getting-started/quickstart.md) — build your first project
- [CLI reference](reference/cli.md) — all subcommands, options and examples
- [Configuration](reference/settings.md) — project and environment settings
- [Database](topics/database.md) — query interfaces and backend behavior
- [Migrations](topics/migrations.md) — schema changes
- [Admin](topics/auth-and-admin.md) and [forms](topics/forms.md) — interactive application features
- [Deployment](how-to/deployment.md) — production-related setup

A framework release before 1.0 may change Rust APIs between minor versions. When upgrading, use the release-specific upgrade guide and release notes instead of assuming that compiling against a new dependency requires no source updates.
