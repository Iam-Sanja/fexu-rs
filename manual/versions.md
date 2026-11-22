# Documentation versions

Each published Fexu release gets its **own frozen manual**. Use the manual matching the Fexu version installed in your Cargo project, especially for CLI flags, API signatures and migration instructions.

## Choose the right version

| Version | What it documents |
| --- | --- |
| **0.1.2** | Latest published release as of 21 November 2026. Installation: `cargo install fexu-cli --version 0.1.2 --locked` |
| **0.2.0 (candidate)** | Source checkout on the `0.2.0` candidate branch. **Not** an approved crates.io release until publish and manual publication complete. |

The CLI built from a `0.2.0` candidate checkout generates standalone registry projects pinned to `=0.2.0`. Until that version exists on crates.io, use `--fexu-path` for local source or stay on published `0.1.2`.

When the documentation website is enabled, the version selector will let you switch between released versions. The `stable` URL will point to the latest **approved** release manual, not automatically to `main`.

## Install the version you are reading about

For the published 0.1.2 release:

```sh
cargo install fexu-cli --version 0.1.2 --locked
cargo add fexu@0.1.2
```

For the unreleased `0.2.0` candidate, build from a matching checkout:

```sh
cargo install --path crates/fexu-cli --locked
```

After a release, the matching version page will contain its exact installation commands. Code examples, flags and signatures on a versioned page apply to that version only.

## Find the right instructions

- [Upgrade from 0.1.2 to development source](how-to/upgrade-0.1.2-to-0.2.md) — known source edits, SQLite evidence and release blockers
- [Quickstart](getting-started/quickstart.md) — build your first project
- [CLI reference](reference/cli.md) — all subcommands, options and examples
- [Configuration](reference/settings.md) — project and environment settings
- [Database](topics/database.md) — query interfaces and backend behavior
- [Migrations](topics/migrations.md) — schema changes
- [Admin](topics/auth-and-admin.md) and [forms](topics/forms.md) — interactive application features
- [Deployment](how-to/deployment.md) — production-related setup

A framework release before 1.0 may change Rust APIs between minor versions. When upgrading, use the release-specific upgrade guide and release notes instead of assuming that compiling against a new dependency requires no source updates.