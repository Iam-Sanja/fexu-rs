# Quickstart

The maintained [0.2.0 walkthrough](../manual/getting-started/quickstart.md) is the canonical quickstart. Fexu **0.2.0 is published** on crates.io; 0.3.0 is planned.

```bash
cargo install fexu-cli --version 0.2.0 --locked
fexu new project demo --template fullstack
cd demo
fexu migrate plan
fexu migrate up
fexu admin create-user --email admin@example.com --staff --superuser
fexu dev
```

For unreleased development source, build a matching CLI from the repository and generate with `--fexu-path`. Do not confuse an unpublished source prerelease with the available registry packages.

Details: [installation](../manual/getting-started/installation.md) · [upgrade](../manual/how-to/upgrade-0.1.2-to-0.2.md) · [CLI](cli.md).
