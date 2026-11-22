# Documentation versions

| Version | What it documents |
| --- | --- |
| **0.1.2** | Latest **published** crates.io release. `cargo install fexu-cli --version 0.1.2 --locked` |
| **0.2.0** | Current workspace and manual target for this release cycle. **Not on crates.io until publish.** |

## Installing 0.2.0 today

Build from the matching Git checkout:

```sh
cargo install --path crates/fexu-cli --locked
```

Post-publish install (smoke test only after index update):

```sh
cargo install fexu-cli --version 0.2.0 --locked
cargo add fexu@0.2.0
```

## Generated projects

- Registry mode (no `--fexu-path`): `fexu = "=0.2.0"` (+ `admin-ui` for fullstack).
- Source mode: `fexu = { path = "...", features = [...] }` via `--fexu-path`.

Badges on the repository README reflect **live crates.io** versions, which may still read 0.1.2 before publish.

See [upgrade guide](how-to/upgrade-0.1.2-to-0.2.md) and [release candidate gate](https://github.com/Iam-Sanja/fexu-rs/blob/main/docs/release-candidate-0.2.md).