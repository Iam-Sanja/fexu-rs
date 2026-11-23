# Quickstart

See the maintained walkthrough in [manual/getting-started/quickstart.md](../manual/getting-started/quickstart.md). This page mirrors the **0.2.0** source install path.

**crates.io** may still list **0.1.2**. Build the CLI from the repository until publish:

```bash
cargo install --path crates/fexu-cli --locked
fexu new project demo --template fullstack --fexu-path .
cd demo
fexu migrate plan
fexu migrate up
fexu admin create-user --email admin@example.com --staff --superuser
fexu dev
```

Post-publish install (smoke only after index lists 0.2.0):

```bash
cargo install fexu-cli --version 0.2.0 --locked
```

Details: [installation](../manual/getting-started/installation.md) · [upgrade](../manual/how-to/upgrade-0.1.2-to-0.2.md) · [CLI](cli.md).