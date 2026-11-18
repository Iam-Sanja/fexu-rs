# Fexu roadmap

[Documentation](index.md) · [Capability matrix](django-capability-matrix.md) · [Release checklist](release-readiness.md) · [GitHub issues](https://github.com/Iam-Sanja/fexu-rs/issues)

**Reviewed:** 2026-11-18 against `main` at `a591b85` (after PR #183). This is a proposed release plan, **not a promise of dates or that every listed feature is complete**. Update the verification baseline when implementation or release status changes.

## Version meanings and current status

- **Published:** `v0.1.2` was released on 2026-11-15. All 16 publishable Fexu workspace crates have a `0.1.2` crates.io release. The release remains **experimental and not production-ready**. See [README](../README.md) and [the v0.1.2 release](https://github.com/Iam-Sanja/fexu-rs/releases/tag/v0.1.2).
- **Development:** `main` includes **unreleased** changes after `v0.1.2`. Use a source checkout to exercise those changes; do not advertise them as shipped in `0.1.2`.
- **Proposed next release:** `v0.2.0` — tested developer preview and a coherent consumer experience.
- **Proposed subsequent release:** `v0.3.0` — optional production-oriented infrastructure foundations. Neither version has a committed release date or a production-readiness guarantee.
- **Historical labels:** changelog entries and old engineering notes such as `v0.13` / `v0.28` are *internal feature milestones*, **not** Cargo versions or crates.io releases.

The implementation inventory below refers to **source on the reviewed commit**, not the published `0.1.2` packages. See the [capability matrix](django-capability-matrix.md) for partial coverage and backend restrictions.

## What is available in the development source

| Area | Implemented source capabilities | Important limits |
| --- | --- | --- |
| Data | `FexuModel`, typed repositories, SeaQuery SQL, SQLite via bundled rusqlite 0.42, PostgreSQL via SQLx 0.8.6, explicit relationships and query scopes | Not unrestricted Django QuerySets; query result completeness and empty `IN` semantics remain tracked in [#144](https://github.com/Iam-Sanja/fexu-rs/issues/144) and [#143](https://github.com/Iam-Sanja/fexu-rs/issues/143) |
| Migrations | Versioned app/framework migrations, guarded destructive operations, drift planning, SQLite rebuild protections | Upgrade and drift behavior still requires the complete acceptance evidence in [#83](https://github.com/Iam-Sanja/fexu-rs/issues/83) |
| HTTP/API | Axum routing, model CRUD, model-derived OpenAPI 3.1, cursor lists, HTML views | Explicit authentication and policy configuration is still required; not every Django-style view is implemented |
| Auth | Argon2id, sessions, role/object permissions, CSRF for protected browser routes, TOTP and password reset | OIDC is still a documented design, not a shipped implementation; see [#85](https://github.com/Iam-Sanja/fexu-rs/issues/85) |
| Native admin | Server-rendered Axum/Tera CRUD, filters, actions, bulk workflows, scoped paginated relation pickers, inlines | No React dependency for basic admin; full browser/accessibility acceptance is still tracked by [#80](https://github.com/Iam-Sanja/fexu-rs/issues/80) |
| Forms and media | Standalone/model forms, contracts, custom widgets, relation fields, formsets, multipart media fields and staged storage lifecycle | End-to-end acceptance remains tracked by [#84](https://github.com/Iam-Sanja/fexu-rs/issues/84); DB and filesystem do **not** share an atomic transaction; no working S3 network client |
| Services/tooling | Project/app CLI, developer diagnostics, local cache, mail, persistent jobs, local storage | Shared cache, remote storage and multi-instance worker guarantees remain scoped future work |

## Proposed v0.2.0 — developer preview

**Goal:** a published, reproducible framework that external developers can install, generate, test and upgrade using supported workflows. This is **not** a claim of production readiness or complete Django feature parity.

### Release blockers and evidence

- [ ] **Data safety:** resolve or document a deliberately safe API contract for empty `IN`/`NOT IN` ([#143](https://github.com/Iam-Sanja/fexu-rs/issues/143)) and explicit query truncation/full iteration ([#144](https://github.com/Iam-Sanja/fexu-rs/issues/144)), with SQLite/PostgreSQL and scope regression tests.
- [ ] **Forms/admin acceptance:** independently verify [#84](https://github.com/Iam-Sanja/fexu-rs/issues/84) and [#80](https://github.com/Iam-Sanja/fexu-rs/issues/80), including real HTML/JSON, CSRF, uploads, rollback, auth-scoped relation editing and browser accessibility. Close only criteria actually proven.
- [ ] **Migration safety:** verify drift, older-schema upgrade, rollback and populated-data contracts on both supported databases ([#83](https://github.com/Iam-Sanja/fexu-rs/issues/83)).
- [ ] **Isolation and clean-room:** resolve PostgreSQL parallel-fixture evidence ([#117](https://github.com/Iam-Sanja/fexu-rs/issues/117)); generate and run an **independent** consumer with no workspace path dependencies on Windows/Linux ([#91](https://github.com/Iam-Sanja/fexu-rs/issues/91), [#93](https://github.com/Iam-Sanja/fexu-rs/issues/93)).
- [ ] **Release regression:** exercise generated-project login, CSRF, forms, admin, ORM, migration and relevant failure paths in browsers and SQLite/PostgreSQL CI, including a tagged-to-next-version migration/upgrade test ([#92](https://github.com/Iam-Sanja/fexu-rs/issues/92), [#93](https://github.com/Iam-Sanja/fexu-rs/issues/93)).
- [ ] **Documentation and policy:** reconcile [Django capability claims](django-capability-matrix.md) ([#70](https://github.com/Iam-Sanja/fexu-rs/issues/70)), installation examples, crate versions, safety limits, change log and the [public release security checklist](public-release-checklist.md). Complete the required release security/asset review; unresolved findings must be explicitly triaged.
- [ ] **Publishing:** synchronize the 16 crate versions/dependency pins, confirm the Trusted Publishing workflow and release environment, require successful checks for the final commit, then smoke-test published artifacts from crates.io; no automated publish merely for merging a PR ([#92](https://github.com/Iam-Sanja/fexu-rs/issues/92)).

**Important:** The checkboxes are **release gates**, not statements that no implementation exists. Several features already have extensive tests; gates stay unchecked until evidence is reviewed against the release candidate. Individual P1/P2 issues may be scoped down with explicit documented limitations rather than silently called complete.

### Deliberately not required for v0.2.0

A working S3 client, Redis cache, OIDC provider support, distributed workers, nested admin inlines, an exhaustive framework benchmark comparison and complete Django parity are **not** v0.2 release gates. SQLx 0.9 is deferred; do not upgrade it merely to satisfy a release number.

## Proposed v0.3.0 — production foundations

**Goal:** optional services and operational reliability appropriate for more demanding deployments, with evidence-based limits.

- **Storage [#96](https://github.com/Iam-Sanja/fexu-rs/issues/96):** backend-neutral local/S3-compatible implementation, authorization, quotas, orphan cleanup, user deletion and media corruption/restore tests. Full image decoding and malware scanning must not be claimed unless actually integrated.
- **Identity [#85](https://github.com/Iam-Sanja/fexu-rs/issues/85), [#172–175](https://github.com/Iam-Sanja/fexu-rs/issues/172):** opt-in OIDC authorization code + PKCE, account linking and replay controls, tested against a local mock provider and existing session/CSRF requirements.
- **Runtime [#87](https://github.com/Iam-Sanja/fexu-rs/issues/87), [#88](https://github.com/Iam-Sanja/fexu-rs/issues/88):** worker crash/restart, idempotency/outbox and scheduling guarantees; optional shared cache with defined consistency and invalidation contracts.
- **Reusable apps [#86](https://github.com/Iam-Sanja/fexu-rs/issues/86):** compile-time-compatible versioned app distribution and upgrade lifecycle.
- **Operations [#94](https://github.com/Iam-Sanja/fexu-rs/issues/94):** observability, readiness, graceful shutdown, least-privilege deployment, PostgreSQL/media backup-and-restore drills and incident guidance.
- **Localization [#148](https://github.com/Iam-Sanja/fexu-rs/issues/148):** bundled reviewed locale catalogs and opt-in translatable application labels/records, delivered in safe increments.

Each workstream needs its own API, security and multi-backend review. **v0.3.0 is not automatically production-certified.** Unmet commitments can be deferred explicitly or block that release after maintainers reassess scope.

## Independent longer-term tracks

- Full Django-oriented capability and admin comparison: [#58](https://github.com/Iam-Sanja/fexu-rs/issues/58), [#70](https://github.com/Iam-Sanja/fexu-rs/issues/70).
- Cross-framework application/benchmark scorecard versus Runique, Cot and Loco: [#95](https://github.com/Iam-Sanja/fexu-rs/issues/95). The current benchmark baseline covers **Fexu only**, not comparative performance.
- Optional advanced ORM and typed admin configuration: [#64](https://github.com/Iam-Sanja/fexu-rs/issues/64), [#79](https://github.com/Iam-Sanja/fexu-rs/issues/79), [#81](https://github.com/Iam-Sanja/fexu-rs/issues/81), [#82](https://github.com/Iam-Sanja/fexu-rs/issues/82). Evaluate each acceptance criterion independently; do not equate an implemented subset with a closed issue.

## Maintaining this roadmap

1. Update the **reviewed commit/date** alongside verified changes; use live GitHub issues and CI as the acceptance evidence.
2. Distinguish `published`, `on main but unreleased`, `partially verified` and `planned`.
3. Keep historical branch/test/feature milestone records dated rather than silently rewriting them as current execution evidence.
4. Do not close an issue solely because a PR was merged; check its full acceptance criteria.
5. Publishing a GitHub Release can trigger the [Trusted Publishing workflow](crates-io-release.md). Do not create a release or tag as a routine documentation operation.
