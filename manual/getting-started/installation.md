# Installation

Fexu **0.2.0** is the current workspace version. **crates.io still serves 0.1.2** until an approved publish. Choose the install path that matches how you work.

## Published registry (0.1.2 today)

```sh
cargo install fexu-cli --version 0.1.2 --locked
cargo add fexu@0.1.2
```

Use this when you need a registry-only workflow against the latest **published** release.

## Fexu 0.2.0 from source (current)

From the Fexu repository root:

```sh
cargo install --path crates/fexu-cli --locked
fexu --version
```

Create a project against this checkout (recommended until 0.2.0 is on crates.io):

```sh
fexu new project demo --template fullstack --fexu-path .
cd demo
```

Registry-generated projects without `--fexu-path` write `fexu = "=0.2.0"` in `Cargo.toml`. That pin resolves from crates.io **only after** 0.2.0 is published.

## After publish (post-release smoke — do not use early)

```sh
cargo install fexu-cli --version 0.2.0 --locked
cargo add fexu@0.2.0
```

Maintainers run these as a post-publish smoke test; they are not valid install steps before the index lists 0.2.0.

## Requirements

- Rust **1.89.0** or newer (`rustc --version`)
- Cargo (bundled with Rust)
- SQLite: no server install required for the default URL
- PostgreSQL: optional; use a disposable database for experiments

See [versions](../versions.md) and [quickstart](quickstart.md).