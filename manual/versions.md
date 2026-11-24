# Documentation versions

| Version | Status | Documentation |
| --- | --- | --- |
| **0.2.0** | **Published** on GitHub and crates.io (all 16 crates) | Published release snapshot; see the [release](https://github.com/Iam-Sanja/fexu-rs/releases/tag/v0.2.0) |
| **0.1.2** | Previous published release | [Upgrade to 0.2.0](how-to/upgrade-0.1.2-to-0.2.md) |
| **0.3.0** | **Planned**, not released | [Development roadmap](https://github.com/Iam-Sanja/fexu-rs/blob/main/docs/roadmap.md); no registry installation yet |

## Install the latest published release

```sh
cargo install fexu-cli --version 0.2.0 --locked
cargo add fexu@0.2.0
```

The published 0.2.0 CLI generates projects with `fexu = "=0.2.0"` (plus `admin-ui` for fullstack). For unreleased development work, install a CLI from a matching repository checkout and generate with `--fexu-path`.

## Versioning policy

GitHub source and crates.io are different distribution channels. The README's dynamic crates.io badges show **published** registry versions, not unreleased code. Before the next feature change, bump all 16 publishable workspace crates and internal dependencies together to an explicit development prerelease such as `0.3.0-dev.1` in a tested PR; keep the last published installation instructions on `0.2.0`. Never imply that a development version is already published.

The online manual follows the latest `main` when its GitHub Pages deployment succeeds; it is **not** a frozen documentation snapshot of a release tag. To consult an immutable 0.2.0 source snapshot, use [the tagged manual](https://github.com/Iam-Sanja/fexu-rs/tree/v0.2.0/manual).
