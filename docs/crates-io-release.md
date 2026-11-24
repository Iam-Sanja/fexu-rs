# crates.io release and versioning policy

**Latest published:** [v0.2.0](https://github.com/Iam-Sanja/fexu-rs/releases/tag/v0.2.0) (2026-11-23), all 16 crates, successful [publish workflow](https://github.com/Iam-Sanja/fexu-rs/actions/runs/45090038609). **Next planned:** v0.3.0 (not published). Fexu remains experimental.

**Do not alter the v0.2.0 release tag.** Publishing a stable GitHub Release triggers crates.io publishing; code pushes and documentation updates do not.

This repository intentionally does **not** publish on pushes to `main` or on pull requests.
Publishing requires a stable GitHub Release (`vX.Y.Z`), a tag pointing to the
current `main` HEAD, an eligible deployment tag in the GitHub `release`
environment, and a version-consistent publishable workspace. The action obtains
a short-lived crates.io credential through GitHub OIDC. The environment currently
has no required reviewers, so **publishing the GitHub Release starts publishing
automatically without a separate environment approval**.

## One-time setup (repository administrator)

1. In GitHub open **Settings → Environments** and configure the **`release`**
   environment. Restrict deployments to release tags matching `v*`. Required
   reviewers are currently disabled; only enable them when an eligible approver
   is configured, otherwise deployment will be blocked. Do not create a crates.io
   token secret.
2. On each existing crates.io crate, configure the GitHub Trusted Publisher:
   owner `Iam-Sanja`, repository `fexu-rs`, workflow filename `publish.yml`,
   environment `release`. The strings must match exactly.
3. Do not turn on *Trusted Publishing Only* until at least one automatic
   publication succeeds. Ensure all intended crate names are already owned
   and the first publication, if any, has been performed through the initial
   manual onboarding process.
4. Merge this workflow only after the feature integration and full regression
   tests have passed. Review the GitHub Actions permissions, restrict who may
   publish GitHub Releases, and explicitly approve creation/publication of each
   GitHub Release before using this workflow. The environment tag restriction
   does not itself provide a human approval gate.

## Coordinated versioning

All **16 publishable crates** and their internal workspace dependency requirements must share the same version. The 0.2.0 tag is immutable. Development source uses the coordinated `0.3.0-dev.1` version; registry scaffolding remains pinned to published 0.2.0. The publication planner intentionally accepts only stable numeric versions; do not relax that guard to validate a development prerelease. **A prerelease version in GitHub source is not published to crates.io automatically.** Do not bump versions for a roadmap-only change.

Verify version consistency and package metadata before a future publication:

```bash
python3 scripts/publish-workspace.py --version VERSION --plan
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

## Release steps

1. Merge all feature work and the version-bump PR. Run complete CI, including
   PostgreSQL and browser integration tests, at the final commit.
2. Confirm all crates to be published have valid crates.io owners and trusted
   publisher configurations; confirm the `release` environment exists.
3. Create a newly approved stable tag (for example `v0.3.0`) at the **current main HEAD** and publish the GitHub
   Release for that tag. The publishing workflow starts automatically.
4. Monitor the `release` environment deployment and the publishing workflow.
   With no required reviewers configured, the job runs automatically after the
   GitHub Release is published and cannot wait for a second manual approval.
5. Confirm every intended version appears on crates.io, inspect the workflow
   log, and check downstream example installation.
6. Only then consider enforcing *Trusted Publishing Only* on each crate.

The publisher uses `cargo metadata` to topologically order workspace crates.
It publishes with `cargo publish --locked`. After each new upload it polls
the sparse index for that exact version, for up to 90 seconds, before the
next crate. A version already present in the index is skipped, so a rerun
continues after a partial release instead of failing on the first live crate.
Publishing is not atomic: earlier crates may already be live and immutable.
Trusted-publishing tokens expire after 30 minutes and the workflow requests
the token only after formatting, Clippy and tests. If a run stops because the
token expired, rerun the same release job after the index shows the crates
that already uploaded. Do not publish a second version number to finish the
same release.

No production database is involved in the publish workflow.
