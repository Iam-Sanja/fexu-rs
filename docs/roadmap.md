# Fexu roadmap

[Documentation](index.md) · [Capability matrix](django-capability-matrix.md) · [Release checklist](release-readiness.md) · [GitHub issues](https://github.com/Iam-Sanja/fexu-rs/issues)

**Implementation baseline reviewed:** 2026-11-20 against `main` at `e5e140a6c70bbdf9c1f98f4f588119457b5ba910` (after PR #226). **Issue status rechecked:** 2026-11-21; #143 and #144 are closed, while release candidate tests are still outstanding. This is a proposed release plan, **not a promise of dates or that every listed feature is complete**. Update the verification baseline when implementation or release status changes.

## Version meanings and current status

- **Published:** `v0.1.2` was released on 2026-11-15. All 16 publishable Fexu workspace crates have a `0.1.2` crates.io release. The release remains **experimental and not production-ready**. See [README](https://github.com/Iam-Sanja/fexu-rs/blob/main/README.md) and [the v0.1.2 release](https://github.com/Iam-Sanja/fexu-rs/releases/tag/v0.1.2).
- **Development:** `main` includes **unreleased** changes after `v0.1.2`. Use a source checkout to exercise those changes; do not advertise them as shipped in `0.1.2`.
- **Proposed next release:** `v0.2.0` — tested developer preview and a coherent consumer experience.
- **Proposed subsequent release:** `v0.3.0` — optional production-oriented infrastructure foundations. Neither version has a committed release date or a production-readiness guarantee.
- **Historical labels:** changelog entries and old engineering notes such as `v0.13` / `v0.28` are *internal feature milestones*, **not** Cargo versions or crates.io releases.

The implementation inventory below refers to **source on the reviewed commit**, not the published `0.1.2` packages. See the [capability matrix](django-capability-matrix.md) for partial coverage and backend restrictions.

## Selected recent work on main (unreleased since v0.1.2)

These commits are on `main` after the previous review and after the published `0.1.2` crates. They do not cut `0.2.0`.

- [#186](https://github.com/Iam-Sanja/fexu-rs/pull/186) form acceptance and [#188](https://github.com/Iam-Sanja/fexu-rs/pull/188) authenticated relation workflows, including PostgreSQL rollback coverage, are on `main`. They do not close [#84](https://github.com/Iam-Sanja/fexu-rs/issues/84) or [#80](https://github.com/Iam-Sanja/fexu-rs/issues/80).
- [#189](https://github.com/Iam-Sanja/fexu-rs/pull/189) localizes Board and Desk example pages. Board HTML uses `locale_of` in `crates/fexu-api/src/generic.rs`: `Accept-Language` starting with `en` selects English, and every other value stays German. Desk's HTTP stock handler calls `stock_markup`, which stays German. [#190](https://github.com/Iam-Sanja/fexu-rs/pull/190) uses the same `en` prefix for the MyShop contact and thank-you pages. The MyShop start page stays fixed German HTML. None of this translates stored records, field labels or choices.
- [#192](https://github.com/Iam-Sanja/fexu-rs/pull/192) makes developer diagnostics English by default. German remains when the locale is `de` or `FEXU_LOCALE=de`.

[#148](https://github.com/Iam-Sanja/fexu-rs/issues/148) stays the proposed catalog and record-content work under v0.3. The example pages above are not that work.

### 2026-11-20 release-hardening update

- PRs #198–#226 have been merged since the previous review, including [#222](https://github.com/Iam-Sanja/fexu-rs/pull/222) published v0.1.2 SQLite consumer test, [#223](https://github.com/Iam-Sanja/fexu-rs/pull/223) app-aware migration planning, [#224](https://github.com/Iam-Sanja/fexu-rs/pull/224) PostgreSQL regression and [#226](https://github.com/Iam-Sanja/fexu-rs/pull/226) the non-publishing preflight.
- [#83](https://github.com/Iam-Sanja/fexu-rs/issues/83), [#117](https://github.com/Iam-Sanja/fexu-rs/issues/117), [#143](https://github.com/Iam-Sanja/fexu-rs/issues/143) and [#144](https://github.com/Iam-Sanja/fexu-rs/issues/144) are closed with recorded acceptance; that is not release authorization. Candidate regression and compatibility checks remain mandatory.
- [#92](https://github.com/Iam-Sanja/fexu-rs/issues/92) and [#93](https://github.com/Iam-Sanja/fexu-rs/issues/93) still require a published-to-candidate consumer upgrade including persisted accounts and sessions on SQLite/PostgreSQL, documentation/security sign-off and release approval.
- All 16 crates target `0.2.0` together; Cargo manifests and crates.io remain at `0.1.2` until the coordinated, reviewed version bump.

## What is available in the development source

| Area | Implemented source capabilities | Important limits |
| --- | --- | --- |
| Data | `FexuModel`, typed repositories, SeaQuery SQL, SQLite via bundled rusqlite 0.42, PostgreSQL via SQLx 0.8.6, explicit relationships and query scopes | Not unrestricted Django QuerySets; safe empty `IN` behavior and explicit query completeness were addressed under closed [#143](https://github.com/Iam-Sanja/fexu-rs/issues/143) and [#144](https://github.com/Iam-Sanja/fexu-rs/issues/144), and must be regression-tested on the 0.2.0 candidate |
| Migrations | Versioned app/framework migrations, guarded destructive operations, drift planning, SQLite rebuild protections | The original migration contract in [#83](https://github.com/Iam-Sanja/fexu-rs/issues/83) is complete; published v0.2.0 upgrade/sign-off belongs to #92/#93 |
| HTTP/API | Axum routing, model CRUD, model-derived OpenAPI 3.1, cursor lists, HTML views | Explicit authentication and policy configuration is still required; not every Django-style view is implemented |
| Auth | Argon2id, sessions, role/object permissions, CSRF for protected browser routes, TOTP and password reset | OIDC is still a documented design, not a shipped implementation; see [#85](https://github.com/Iam-Sanja/fexu-rs/issues/85) |
| Native admin | Server-rendered Axum/Tera CRUD, filters, actions, bulk workflows, scoped paginated relation pickers, inlines | No React dependency for basic admin; full browser/accessibility acceptance is still tracked by [#80](https://github.com/Iam-Sanja/fexu-rs/issues/80) |
| Forms and media | Standalone/model forms, contracts, custom widgets, relation fields, formsets, multipart media fields and staged storage lifecycle | End-to-end acceptance remains tracked by [#84](https://github.com/Iam-Sanja/fexu-rs/issues/84); DB and filesystem do **not** share an atomic transaction; no working S3 network client |
| Services/tooling | Project/app CLI, developer diagnostics, local cache, mail, persistent jobs, local storage | Shared cache, remote storage and multi-instance worker guarantees remain scoped future work |

## Proposed v0.2.0 — developer preview

**Goal:** a published, reproducible framework that external developers can install, generate, test and upgrade using supported workflows. This is **not** a claim of production readiness or complete Django feature parity.

### Release blockers and evidence

- [ ] **Data safety candidate validation:** [#143](https://github.com/Iam-Sanja/fexu-rs/issues/143) and [#144](https://github.com/Iam-Sanja/fexu-rs/issues/144) are closed. Before publishing, rerun empty `IN`/`NOT IN`, scoped predicates, query completeness and full iteration regression tests against the exact 0.2.0 candidate on SQLite/PostgreSQL. The implementation issues are no longer open blockers; the final candidate proof remains a release gate.
- [ ] **Forms/admin acceptance:** independently verify [#84](https://github.com/Iam-Sanja/fexu-rs/issues/84) and [#80](https://github.com/Iam-Sanja/fexu-rs/issues/80), including real HTML/JSON, CSRF, uploads, rollback, auth-scoped relation editing and browser accessibility. Close only criteria actually proven.
- [x] **Migration contract:** original drift, older-schema artifact, rollback and populated-data criteria verified; [#83](https://github.com/Iam-Sanja/fexu-rs/issues/83) is closed. Published 0.1.2-to-candidate upgrades with live sessions remain open under #92/#93.
- [x] **PostgreSQL fixture isolation:** [#117](https://github.com/Iam-Sanja/fexu-rs/issues/117) closed after PostgreSQL 17/18 CI and external TestShop PostgreSQL 18.6 evidence (five parallel runs, 70 successful tests).
- [ ] **Independent clean-room consumer:** generate and run a candidate consumer on Windows/Linux and SQLite/PostgreSQL with no unintended local path dependencies ([#91](https://github.com/Iam-Sanja/fexu-rs/issues/91), [#93](https://github.com/Iam-Sanja/fexu-rs/issues/93)).
- [ ] **Release regression:** exercise generated-project login, CSRF, forms, admin, ORM, migration and relevant failure paths in browsers and SQLite/PostgreSQL CI, including a tagged-to-next-version migration/upgrade test ([#92](https://github.com/Iam-Sanja/fexu-rs/issues/92), [#93](https://github.com/Iam-Sanja/fexu-rs/issues/93)).
- [ ] **Documentation and policy:** finish the [versioned manual](documentation-maintenance.md), source-check documentation and the [Django capability matrix](django-capability-matrix.md), reconcile installation commands, all 16 crate versions, security limits, changelog and [public checklist](public-release-checklist.md). A passing link audit is not a complete API review.
- [ ] **Publishing:** coordinate all 16 crate versions/dependencies to planned `0.2.0` in a dedicated release-candidate PR, run the [non-publishing preflight](release-candidate-0.2.md), confirm Trusted Publishing/release environment and checks, and smoke-test *published* crates after explicit authorization (#92).

**Important:** The checkboxes are **release gates**, not statements that no implementation exists. Several features already have extensive tests; gates stay unchecked until evidence is reviewed against the release candidate. Individual P1/P2 issues may be scoped down with explicit documented limitations rather than silently called complete.

### Deliberately not required for v0.2.0

A working S3 client, Redis cache, OIDC provider support, distributed workers, nested admin inlines, an exhaustive framework benchmark comparison and complete Django parity are **not** v0.2 release gates. SQLx 0.9 is deferred; do not upgrade it merely to satisfy a release number. This exception concerns the dependency-version upgrade only: PostgreSQL tests, dependency security findings and release packaging still must be assessed and pass the applicable gates.

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
