# Installation

The latest published stable Fexu version is **0.2.0**. Version **0.3.0** is planned and not on crates.io. Fexu is experimental; do not assume production readiness.

## Install Fexu 0.2.0 from crates.io

Requires Rust **1.89.0** or newer.

```sh
cargo install fexu-cli --version 0.2.0 --locked
fexu --version
```

Create a fullstack project using the published registry dependency:

```sh
fexu new project demo --template fullstack
cd demo
fexu migrate plan
fexu migrate up
fexu admin create-user --email admin@example.com --staff --superuser
fexu dev
```

The generator pins `fexu = "=0.2.0"` in the generated project's `Cargo.toml` (with `admin-ui` enabled for the fullstack template). To add Fexu to a separate Cargo project, use `cargo add fexu@0.2.0`.

## Development builds toward 0.3.0

To test unreleased source changes, clone the repository, install the CLI from the matching checkout and pass the absolute source checkout path with `--fexu-path`. Do not mix a development CLI or prerelease source with stable registry dependencies without checking compatibility.

```sh
git clone https://github.com/Iam-Sanja/fexu-rs.git
cd fexu-rs
cargo install --path crates/fexu-cli --locked
```

## Requirements

- Rust **1.89.0+** and Cargo
- SQLite: no separate server required by the default setup
- PostgreSQL: optional; use a disposable database for experiments

See [version policy](../versions.md), [quickstart](quickstart.md) and the [upgrade guide](../how-to/upgrade-0.1.2-to-0.2.md).
