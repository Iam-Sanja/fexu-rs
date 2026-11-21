# Installation

## Requirements

Install Rust with Cargo and ensure that `cargo` and `rustc` are available in your terminal. The current framework workspace declares Rust **1.89.0** as its minimum supported toolchain.

```sh
rustc --version
cargo --version
```

Install the **published** CLI:

```sh
cargo install fexu-cli --version 0.1.2 --locked
fexu --version
fexu --help
```

For the next unreleased version, install the CLI directly from the corresponding Fexu checkout instead:

```sh
cargo install --path crates/fexu-cli
```

Run that last command from the **Fexu framework repository root**, not from your application directory. Use the matching `--fexu-path` generator option when creating a project against unpublished source.

On Windows use PowerShell; on Linux and macOS use a shell. The generated default uses local SQLite and does not require a PostgreSQL server.

See [quickstart](quickstart.md), the [first-project tutorial](../tutorial/first-project.md) and the [version guide](../versions.md).
