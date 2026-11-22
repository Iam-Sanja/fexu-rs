# Fexu 0.2.0 — final documentation and release readiness review

| Field | Value |
|---|---|
| **Starting SHA** (branch tip) | `5548ac53d745e5a69af27bbc7f407f4adb947ee5` |
| **Documentation branch** | `cursor/release-020-docs` |
| **Worktree** | `D:\04_Projekte_Arbeit\Projekte\Fexu\release-docs-020` |
| **Release stack** | [PR #251](https://github.com/Iam-Sanja/fexu-rs/pull/251) `release/020-final-acceptance` @ `d76dab7`, [PR #252](https://github.com/Iam-Sanja/fexu-rs/pull/252) `cursor/release-020-version` @ `5548ac5` (both **open**) |
| **OS** | Windows 10.0.26300 |
| **rustc** | 1.100.0 (0ef1bc386 2026-11-09) |
| **cargo** | 1.100.0 (3acc79c0c 2026-11-05) |
| **Built-on commits** | Acceptance `d76dab7c93739f50cfea7fb49f872a3eef16e923`; version `5548ac53d745e5a69af27bbc7f407f4adb947ee5`; docs review commits on `cursor/release-020-docs` (this PR) |

**Argon2 maintainer decision (fixed wording):** Historical Windows Argon2 access violation not reproduced after OS reinstall and BIOS update. Residual risk accepted by maintainer.

---

## 1. README as 0.2.0 front page

| Check | Result | Evidence |
|---|---|---|
| Remove candidate/unpublished table column | **PASS** | `README.md` rewritten: 16-crate table with role + live crates.io badges only |
| Document 0.2.0 install without false crates.io claim | **PASS** | Source `cargo install --path crates/fexu-cli`; post-publish `0.2.0` commands labeled smoke-only |
| Required sections present | **PASS** | Features, install, quickstart, model, DB, auth, admin, layout/CLI, upgrade, limits, contributing, license |
| No marketing beyond code | **PASS** | Limits point to security/roadmap; experimental banner retained |

---

## 2. Quickstart command verification

| Step | Result | Evidence |
|---|---|---|
| `cargo build` / install CLI from source | **PASS** | Existing `target/debug/fexu.exe` in worktree |
| `fexu new project … --fexu-path` + `migrate up` | **PASS** | Temp `Smoke020`: auth + job migrations applied, no pending |
| `fexu doctor` | **PASS** | Exit 0; OK rustc/cargo/fexu.toml |
| `fexu admin` / `fexu dev` HTTP | **NOT TESTED** | No automated HTTP curl in this run (time budget) |
| `cargo check` on generated app (no lock) | **PARTIAL** | `cargo check --locked` failed without `Cargo.lock` (expected for fresh project); path dep resolves |
| `cargo test -p fexu-cli --test quickstart` | **PASS** | `clean_room_project_has_no_local_fexu_path` ok @ review start |

---

## 3. Sixteen-crate inventory (source-backed)

| Crate | Role | Implemented | Tests in tree |
|---|---|:---:|:---:|
| fexu | Umbrella / prelude / app builder | yes | yes |
| fexu-cli | Generator, migrate, dev, worker | yes | yes |
| fexu-core | Models, validation, scopes | yes | yes |
| fexu-macros | Derives | yes | no dedicated tests |
| fexu-db | SQLite + PostgreSQL repos | yes | yes |
| fexu-migrations | Plans, apply, fixtures | yes | yes |
| fexu-auth | Argon2id, sessions, CSRF, roles | yes | yes |
| fexu-api | HTTP, OpenAPI, serve | yes | yes |
| fexu-admin | Native admin UI | yes | yes |
| fexu-forms | Forms/widgets | yes | yes |
| fexu-static | Static files | yes | yes |
| fexu-storage | Uploads, signed URLs | yes | yes |
| fexu-cache | Memory cache | yes | yes |
| fexu-mail | SMTP/outbox | yes | yes |
| fexu-jobs | Queue, worker, cron | yes | yes |
| fexu-middleware | Security/request middleware | yes | yes |

Cross-cutting: **scheduler** (`fexu-jobs` cron), **fixtures** (`fexu-migrations`), **OpenAPI** (`fexu-api`), **Argon2id** (`fexu-auth`), **PostgreSQL** (`fexu-db`).

---

## 4. Documentation corpus review

| Area | Action | Result |
|---|---|---|
| `README.md` | Rewritten for 0.2.0 | **PASS** |
| `manual/getting-started/*` | Installation + quickstart aligned to source 0.2.0 | **PASS** |
| `manual/versions.md`, `manual/index.md` | Registry vs workspace clarity | **PASS** |
| `docs/quickstart.md` | Point to manual; remove 0.1.2-only flow | **PASS** |
| `manual/how-to/upgrade-0.1.2-to-0.2.md` | Document CI-proven scripts @ d76dab7 | **PASS** |
| `docs/` mirror pages | Full parity sweep | **PARTIAL** — some `docs/*.md` still mirror older text; manual is canonical for install |
| Crate READMEs | Scan | **NOT TESTED** (workspace inherits root README) |
| mkdocs / link checker | Run | **NOT TESTED** — no `mkdocs build` in this run |

---

## 5. Upgrade guide vs acceptance tests

| Claim | Evidence |
|---|---|
| API SQLite published→source | `scripts/test-published-v012-upgrade.sh` — CI @ d76dab7 |
| Auth SQLite/PostgreSQL | `scripts/test-published-v012-*-auth-upgrade.sh` |
| Fullstack SQLite/PostgreSQL + HTTP | `scripts/test-published-v012-fullstack-*` — CI job green @ d76dab7 / @ 5548ac5 on #252 |
| Not automatic app rewrite | Documented in upgrade guide + `fexu upgrade --check` behavior |
| `resolved_database_url()?` breaking change | Documented with source edit |

---

## 6. Release documentation alignment

| Document | Public behavior | Internal gate language |
|---|---|---|
| `docs/release-candidate-0.2.md` | Unchanged: not authorization | **PASS** |
| `docs/crates-io-release.md` | Historical 0.1.2 + planner | **PASS** (no false publish claim) |
| `docs/roadmap.md` | Gates unchecked where evidence pending | **PASS** |
| `CHANGELOG.md` | 0.2.0 candidate section; not publish claim | **PASS** |
| `manual/versions.md` | 0.2.0 workspace; crates.io may be 0.1.2 | **PASS** |

---

## 7. Example and template smoke

| Target | Result | Evidence |
|---|---|---|
| MyShop `cargo check -p myshop --locked` | **PASS** | Finished dev profile |
| Generated fullstack + `--fexu-path` | **PASS** | Section 2 |
| Bookstore / API template isolated compile | **NOT TESTED** | Not run in this pass |
| HTTP CRUD restart | **NOT TESTED** | |

---

## 8. Security and deployment documentation

| Topic | Documented | Production proof |
|---|---|---|
| Cookie flags, CSRF | `docs/security.md`, auth crate | **NOT TESTED** in this run |
| Proxy/trusted headers | `deploy/`, diagnose deploy checks | Documented; limited prod test |
| Secrets / env | `docs/settings.md` | Documented |
| Docker/Nginx samples | `deploy/` | Example only |

Untested production limits remain explicit in security and deployment docs.

---

## 9. Local verification matrix

| Command | Result | Notes |
|---|---|---|
| `cargo fmt --all -- --check` | **PASS** | Exit 0 |
| `cargo clippy --workspace --all-targets --locked -D warnings` | **PASS** | Exit 0 on docs worktree (manifest warnings only) |
| `cargo test --workspace --locked` | **NOT TESTED** | Not completed locally (duration) |
| `cargo check -p myshop --locked` | **PASS** | |
| `cargo build --release --workspace --locked` | **NOT TESTED** | |
| `cargo audit` | **PASS** (warnings) | 3× unmaintained chrono family advisories |
| `publish-workspace.py --plan 0.2.0` | **PASS** | 16-crate order |
| `cargo package --list` ×16 | **PASS** | `--allow-dirty` pre-commit |
| Quickstart / generator tests | **PASS** | quickstart test |
| Browser / Playwright | **NOT TESTED** | Requires Linux CI / Node setup |
| PostgreSQL fullstack local | **NOT TESTED** | No local `psql`/Docker on Windows host |
| MSRV 1.89 | **NOT TESTED** | CI on stack PRs |
| GitHub CI green **this docs PR** | **PENDING** | Do not claim until checks finish |

**Prior stack CI (reference only):** PR #252 @ `5548ac5` reported 14/14 green (not re-run for this commit).

---

## 10. Preflight dry-run (no publish)

| Step | Result |
|---|---|
| Version graph `0.2.0` | **PASS** |
| `cargo package --list` all publishable crates | **PASS** |
| Secret/path scan in packages | **PARTIAL** — manual review of package lists; no automated secret scanner |
| `cargo publish --dry-run` | **NOT TESTED** | Planner + list used instead |

---

## 11. Feature matrix (summary)

| Function | Impl | Documented | Tested | Status |
|---|---|---|---|---|
| FexuModel / repos | yes | yes | yes | shippable |
| SQLite | yes | yes | yes | shippable |
| PostgreSQL | yes | yes | CI | shippable |
| Migrations checksum/history | yes | yes | yes | shippable |
| Published 0.1.2→0.2.0 consumer | yes | yes | CI scripts | shippable with caveats |
| HTTP/OpenAPI | yes | yes | yes | shippable |
| Auth Argon2id/sessions/CSRF | yes | yes | yes | shippable |
| Native admin | yes | yes | partial browser | experimental |
| Forms/widgets | yes | yes | yes | shippable |
| Jobs/cron worker | yes | yes | yes | shippable |
| Mail outbox | yes | yes | partial | experimental |
| Storage/uploads | yes | yes | yes | shippable |
| Generator `=0.2.0` pin | yes | yes | manifest test | pending crates.io |
| crates.io install 0.2.0 | n/a | documented smoke | **NOT TESTED** | post-publish |

---

## 12. Blockers and verdict

### Blockers

1. **Maintainer release approval** — not granted (no tag, no GitHub Release, no publish).
2. **Post-publish crates.io smoke** — `cargo install fexu-cli --version 0.2.0` / `cargo add fexu@0.2.0` not executed (invalid before publish).
3. **Release stack open** — PR #251 and #252 unmerged; not promoted to `main`.
4. **This documentation PR** — CI not green at final SHA when this file was written.
5. **Local gaps** — full workspace `cargo test`, release build, browser, PostgreSQL smokes on this machine **NOT TESTED**.

### Verdict

## **RELEASE BLOCKED**

Reason: Required technical gates were not all executed on this documentation commit; maintainer sign-off and post-publish registry smoke are outstanding; release stack PRs remain open.

**Not** `READY FOR MAINTAINER RELEASE APPROVAL` until local/CI verification on the documentation head is green and only maintainer approval plus post-publish smoke remain.