# Fexu roadmap

[Documentation](index.md) · [Capability matrix](django-capability-matrix.md) · [Release process](crates-io-release.md) · [GitHub issues](https://github.com/Iam-Sanja/fexu-rs/issues)

**Updated: 2026-11-24. Reviewed baseline: `20aee5b`.** A plan, not a promise of dates, completeness, or production readiness. Fexu remains experimental.

## Current releases and version policy

- **Latest published: [v0.2.0](https://github.com/Iam-Sanja/fexu-rs/releases/tag/v0.2.0)** (2026-11-23). All 16 crates published via the [successful release workflow](https://github.com/Iam-Sanja/fexu-rs/actions/runs/45090038609), with CI and the [release preflight](https://github.com/Iam-Sanja/fexu-rs/actions/runs/45086997959).
- **Previous: [v0.1.2](https://github.com/Iam-Sanja/fexu-rs/releases/tag/v0.1.2)**. See the [upgrade guide](../manual/how-to/upgrade-0.1.2-to-0.2.md).
- **Next planned: v0.3.0.** No release tag, registry packages or date yet.
- **Development:** Workspace source is coordinated at `0.3.0-dev.1` (unpublished). Registry scaffolding remains pinned to 0.2.0; use `--fexu-path` for development. The version change requires locked cross-platform CI. No release tag or registry publication is implied.
- Old engineering milestone labels (for example `v0.13`) are not Cargo release tags.

## v0.3.0 dependency modernization

- [Dependabot #237](https://github.com/Iam-Sanja/fexu-rs/pull/237) upgraded the SHA-pinned `actions/setup-python` action to 7.1.0 (merged).
- `rusqlite` 0.43 integrated and tested through [PR #256](https://github.com/Iam-Sanja/fexu-rs/pull/256); Dependabot #238 closed.
- Argon2 0.6 integrated through [PR #257](https://github.com/Iam-Sanja/fexu-rs/pull/257); SQLx 0.9.1 (core and PostgreSQL) through [PR #258](https://github.com/Iam-Sanja/fexu-rs/pull/258). Both passed required CI. Dependabot #240/#241/#242 closed.
- SeaQuery 1.0.2 merged in [PR #259](https://github.com/Iam-Sanja/fexu-rs/pull/259); Dependabot #239 closed as superseded. Linux, Windows, MSRV 1.89, PostgreSQL, browser, published 0.1.2 upgrade, security and documentation checks passed on the PR.
- Dependency updates on GitHub are not new published Fexu versions. Keep all 16 framework crates, internal requirements, lockfile and documentation coordinated; keep registry scaffolding on the latest published version.

## What v0.2.0 delivered

- 16 coordinated crates: CLI, models, SQLite/PostgreSQL, migrations, APIs, authentication, native admin, forms, storage, mail and jobs.
- Published 0.1.2 consumer upgrade validation on SQLite and PostgreSQL, including fullstack data, auth and session continuity.
- Cross-platform CI, release preflight, security and documentation reviews. These do not constitute a production-readiness certification.

## Capability baseline from v0.2.0

| Area | Implemented source capabilities | Important limits |
| --- | --- | --- |
| Data | `FexuModel`, typed repositories, SeaQuery SQL, SQLite via bundled rusqlite 0.42, PostgreSQL via SQLx 0.8.6, explicit relationships and query scopes | Not unrestricted Django QuerySets; safe empty `IN` behavior and explicit query completeness were addressed under closed [#143](https://github.com/Iam-Sanja/fexu-rs/issues/143) and [#144](https://github.com/Iam-Sanja/fexu-rs/issues/144), and remain regression-tested |
| Migrations | Versioned app/framework migrations, guarded destructive operations, drift planning, SQLite rebuild protections | The original migration contract in [#83](https://github.com/Iam-Sanja/fexu-rs/issues/83) is complete; upgrades to 0.2.0 were covered in release acceptance |
| HTTP/API | Axum routing, model CRUD, model-derived OpenAPI 3.1, cursor lists, HTML views | Explicit authentication and policy configuration is still required; not every Django-style view is implemented |
| Auth | Argon2id, sessions, role/object permissions, CSRF for protected browser routes, TOTP and password reset | OIDC is still a documented design, not a shipped implementation; see [#85](https://github.com/Iam-Sanja/fexu-rs/issues/85) |
| Native admin | Server-rendered Axum/Tera CRUD, filters, actions, bulk workflows, scoped paginated relation pickers, inlines | No React dependency for basic admin; full browser/accessibility acceptance is still tracked by [#80](https://github.com/Iam-Sanja/fexu-rs/issues/80) |
| Forms and media | Standalone/model forms, contracts, custom widgets, relation fields, formsets, multipart media fields and staged storage lifecycle | End-to-end acceptance remains tracked by [#84](https://github.com/Iam-Sanja/fexu-rs/issues/84); DB and filesystem do **not** share an atomic transaction; optional S3 network client exists in development source; production lifecycle acceptance remains open |
| Services/tooling | Project/app CLI, developer diagnostics, local cache, mail, persistent jobs, local storage | Shared cache, remote storage and multi-instance worker guarantees remain scoped future work |


## Next: v0.3.0 — production foundations

See the [0.3.0 acceptance plan](release-candidate-0.3.md) for release gates.

**Goal:** optional services and operational reliability appropriate for more demanding deployments, with evidence-based limits.

- **Storage [#96](https://github.com/Iam-Sanja/fexu-rs/issues/96):** backend-neutral local/S3-compatible implementation, authorization, quotas, orphan cleanup, user deletion and media corruption/restore tests. Per-ID settled quota release passed in #296; opt-in local fenced quota cleanup passed in #297. Bounded local quota worker integration is pending CI; remote release, historical reconciliation and production acceptance remain open. Full image decoding and malware scanning must not be claimed unless actually integrated.
- **Identity [#85](https://github.com/Iam-Sanja/fexu-rs/issues/85), [#172–175](https://github.com/Iam-Sanja/fexu-rs/issues/172):** opt-in OIDC authorization code + PKCE, account linking and replay controls, tested against a local mock provider and existing session/CSRF requirements.
- **Runtime [#87](https://github.com/Iam-Sanja/fexu-rs/issues/87), [#88](https://github.com/Iam-Sanja/fexu-rs/issues/88):** worker crash/restart, idempotency/outbox and scheduling guarantees; optional shared cache with defined consistency and invalidation contracts.
- **Reusable apps [#86](https://github.com/Iam-Sanja/fexu-rs/issues/86):** compile-time-compatible versioned app distribution and upgrade lifecycle.
- **Operations [#94](https://github.com/Iam-Sanja/fexu-rs/issues/94):** observability, readiness, graceful shutdown, least-privilege deployment, PostgreSQL/media backup-and-restore drills and incident guidance.
- **Localization [#148](https://github.com/Iam-Sanja/fexu-rs/issues/148):** bundled reviewed locale catalogs and opt-in translatable application labels/records, delivered in safe increments.

Each workstream needs its own API, security and multi-backend review. The release gates are operations (#94), cross-backend acceptance (#93), and a new upgrade from published 0.2.0 (#92). Prioritize local/S3 storage, worker reliability, optional shared cache, OIDC with its mandatory security tests, a minimal reusable-app contract and English-first locale foundations. Defer multilingual stored records (#193), broader ORM/admin API expansion and optional distributed auth limiting (#175) to v0.4.0; comparative benchmarks (#95) target v0.5.0. These are planning targets, not shipped features. See [issue backlog](https://github.com/Iam-Sanja/fexu-rs/issues) and [milestones](https://github.com/Iam-Sanja/fexu-rs/milestones). **v0.3.0 is not automatically production-certified.** Unmet commitments can be deferred explicitly or block that release after maintainers reassess scope.

## Independent longer-term tracks

- Full Django-oriented capability and admin comparison: [#58](https://github.com/Iam-Sanja/fexu-rs/issues/58), [#70](https://github.com/Iam-Sanja/fexu-rs/issues/70).
- Cross-framework application/benchmark scorecard versus Runique, Cot and Loco: [#95](https://github.com/Iam-Sanja/fexu-rs/issues/95). The current benchmark baseline covers **Fexu only**, not comparative performance.
- Optional advanced ORM and typed admin configuration: [#64](https://github.com/Iam-Sanja/fexu-rs/issues/64), [#79](https://github.com/Iam-Sanja/fexu-rs/issues/79), [#81](https://github.com/Iam-Sanja/fexu-rs/issues/81), [#82](https://github.com/Iam-Sanja/fexu-rs/issues/82). Evaluate each acceptance criterion independently; do not equate an implemented subset with a closed issue.

## Maintaining this roadmap and documentation

1. Update the **reviewed commit/date** alongside verified changes; use live GitHub issues and CI as the acceptance evidence.
2. Distinguish `published`, `on main but unreleased`, `partially verified` and `planned`.
3. Keep historical branch/test/feature milestone records dated rather than silently rewriting them as current execution evidence.
4. Do not close an issue solely because a PR was merged; check its full acceptance criteria.
5. Publishing a GitHub Release can trigger the [Trusted Publishing workflow](crates-io-release.md). Do not create a release or tag as a routine documentation operation.

## Ongoing PR requirements

- Update relevant `manual/` pages **in the same PR** as user-facing code changes. Update `README.md` for changed capabilities/quickstarts, or explicitly justify no impact in the PR.
- Update `CHANGELOG.md` and the roadmap when features are added, removed, fixed, deferred or verified. Do not mark future work shipped until tested and merged.
- Coordinate all 16 workspace crate versions, internal dependencies, lockfile and generator dependency pins on every version change; running code on GitHub does not imply that prerelease crates were published.
- GitHub Pages tracks the current `main` manual after relevant docs merges; the immutable `v0.2.0` manual remains in the release tag.
