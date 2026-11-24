# Documentation versions

**Latest published stable version: 0.2.0.** Next planned milestone: **0.3.0** (not released). Fexu is experimental.

| Version | Status | Source |
| --- | --- | --- |
| **0.2.0** | Published on crates.io; all 16 crates | [GitHub Release](https://github.com/Iam-Sanja/fexu-rs/releases/tag/v0.2.0) and [immutable tagged manual](https://github.com/Iam-Sanja/fexu-rs/tree/v0.2.0/manual) |
| **0.1.2** | Previous published release | [Upgrade guide](../manual/how-to/upgrade-0.1.2-to-0.2.md) |
| **0.3.0** | In development (`0.3.0-dev.1` source only); not published | [Roadmap](roadmap.md) |

The [GitHub Pages documentation](https://iam-sanja.github.io/fexu-rs/) currently deploys the latest `manual/` from `main`, **not** an immutable tagged release or a `mike` version selector. Pages can show explicitly labelled newer development work. To read exactly the 0.2.0 source manual, use the tagged link above.

## Install published 0.2.0

```sh
cargo install fexu-cli --version 0.2.0 --locked
cargo add fexu@0.2.0
```

For work against unreleased development source, use a matching Git checkout and the `--fexu-path` CLI option rather than assuming that source-only changes are published.

See [Quickstart](quickstart.md), [CLI](cli.md), [Database](database.md), [Migrations](migrations.md), [Admin](admin.md) and [Deployment](deployment.md). Pre-1.0 upgrades may require source-code changes; follow the release-specific upgrade guide.
