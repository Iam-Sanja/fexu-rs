# crates.io release preparation

This repository intentionally does **not** publish on pushes to `main` or on pull requests.
Publishing requires a stable GitHub Release (`vX.Y.Z`), a tag pointing to the
current `main` HEAD, approval of the GitHub `release` environment, and a
version-consistent publishable workspace. The action obtains a short-lived
crates.io credential through GitHub OIDC.

## One-time setup (repository administrator)

1. In GitHub open **Settings → Environments → New environment** and name it
   **`release`**. Configure required reviewers (when supported) and restrict
   deployment to release tags matching `v*`. Do not create a crates.io token
   secret.
2. On each existing crates.io crate, configure the GitHub Trusted Publisher:
   owner `Iam-Sanja`, repository `fexu-rs`, workflow filename `publish.yml`,
   environment `release`. The strings must match exactly.
3. Do not turn on *Trusted Publishing Only* until at least one automatic
   publication succeeds. Ensure all intended crate names are already owned
   and the first publication, if any, has been performed through the initial
   manual onboarding process.
4. Merge this workflow only after the current feature PRs are integrated and
   full regression tests have passed. Review the GitHub Actions permissions
   and protect the release environment from unreviewed workflows.

## Version policy

For the first synchronized workspace release, propose `0.1.2`.
Currently most workspace crates inherit `0.1.0` while `fexu-cli` has `0.1.1`.
`0.1.2` is therefore a proposed **common next version**, not a version
automatically derived by Cargo. Each publishable package must have version
`0.1.2` and each internal path dependency must require `0.1.2`.
This version bump and the lockfile update belong in a separately reviewed
release preparation PR. Existing released versions must never be overwritten.

Inspect the publishing graph before making a release:

```bash
python3 scripts/publish-workspace.py --version 0.1.2 --plan
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

The planner errors if versions are inconsistent, internal dependencies are
unpublishable, or the local dependency graph has a cycle.

## Release steps

1. Merge all feature work and the version-bump PR. Run complete CI, including
   PostgreSQL and browser integration tests, at the final commit.
2. Confirm all crates to be published have valid crates.io owners and trusted
   publisher configurations; confirm the `release` environment exists.
3. Create a tag `v0.1.2` at the **current main HEAD** and publish the GitHub
   Release for that tag. The publishing workflow starts automatically.
4. Review and approve the `release` environment deployment.
5. Confirm every intended version appears on crates.io, inspect the workflow
   log, and check downstream example installation.
6. Only then consider enforcing *Trusted Publishing Only* on each crate.

The publisher uses `cargo metadata` to topologically order workspace crates.
It publishes all publishable workspace packages with `cargo publish --locked`
and waits between packages for registry indexing. Publishing is not atomic:
if a later crate fails, earlier crates may already be live and immutable.
Never rerun the same release blindly; diagnose partial publication first.
The OIDC token is short-lived; a slow multi-crate release may require renewal
or a split publishing workflow before production use.

No production database is involved in the publish workflow.
