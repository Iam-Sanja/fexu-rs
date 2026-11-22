# Fexu 0.2.0 final release acceptance

**Repository:** [Iam-Sanja/fexu-rs](https://github.com/Iam-Sanja/fexu-rs)  
**Acceptance-Basis:** `release/020-final-acceptance` @ `d76dab7c93739f50cfea7fb49f872a3eef16e923` (14/14 CI grün)  
**Versions-Kandidat:** Branch `cursor/release-020-version` (nur `0.2.0`-Bump)  
**Workspace-Label:** `0.2.0` (Kandidat, **nicht** auf crates.io veröffentlicht)  
**Laufzeit:** 2026-11-22, Windows 10.0.26300, rustc/cargo 1.100.0  
**Isoliertes Worktree (Version):** `D:\04_Projekte_Arbeit\Projekte\Fexu\release-version-020`  
**TestShop / Haupt-Checkout:** unangetastet.

## Gate-Tabelle

| Phase | Gate | Status | Evidenz |
|---|---|---|---|
| 0 | Acceptance-Branch CI grün | **PASS** | `d76dab7` — 14/14 Checks inkl. PostgreSQL auth + Fullstack |
| 4 | Published 0.1.2 **API** CLI → Checkout (SQLite) | **PASS** | `scripts/test-published-v012-upgrade.sh` in CI @ `d76dab7` |
| 4 | Published 0.1.2 Auth-Binär-Upgrade SQLite | **PASS** | `scripts/test-published-v012-auth-upgrade.sh` + CI |
| 4 | Published 0.1.2 Auth-Binär-Upgrade PostgreSQL | **PASS** | `scripts/test-published-v012-postgres-auth-upgrade.sh` + CI |
| 4 | Published 0.1.2 **Fullstack**-App → Checkout (SQLite, HTTP, Rollen, Daten) | **PASS** | `scripts/test-published-v012-fullstack-upgrade.sh` — lokal Git-Bash + CI |
| 4 | Published 0.1.2 Fullstack → Checkout **PostgreSQL 16** (eigene DB) | **PASS** (CI) / **BLOCKED** (lokal) | CI-Job „Published 0.1.2 PostgreSQL auth and fullstack upgrade“ @ `d76dab7` grün. Lokal Windows: weder `docker` noch `psql` im PATH (Skript Exit 3) |
| 4 | Fehlgeschlagene Migration ohne Datenverlust (Fullstack-Skript) | **PASS** | `failed-migration` in Fullstack-Skripten; `schema_evolution.rs` in CI |
| 5 | HTTP Admin vor/nach Upgrade (Login, Session, CSRF, Bad Password) | **PASS** | `v012_fullstack_upgrade_lib.py` Matrix |
| 5 | Playwright/Myshop-Browser-Matrix (kein Upgrade) | **PASS** | CI `tests/browser` @ `d76dab7` |
| 5 | Vollständige Admin-CRUD/Paging-Matrix auf Upgrade-App | **NOT TESTED** | Nur Kern-HTTP; optional Playwright auf Disposable-App |
| 6 | Argon2 Reproducer debug 8×25 | **PASS** | 25/25 PASS, 0 AV |
| 6 | Argon2 Reproducer debug 24×25 | **PASS** | 25/25 PASS, 0 AV |
| 6 | Argon2 Reproducer release 8×25 | **PASS** | 25/25 PASS, 0 AV |
| 6 | Argon2 Reproducer release 24×25 | **PASS** | 25/25 PASS, 0 AV |
| 6 | `fexu-auth` Framework-Tests | **PASS** | `cargo test -p fexu-auth --lib`: 48 passed, 3 ignored (PostgreSQL) |
| 6 | Historische 30/180 AV-Reproduktion | **NOT TESTED** | Kein 0xc0000005 in dokumentierter Matrix; **keine** Behauptung „historischer Crash behoben“ |
| 7 | `cargo fmt --check` | **PASS** | Lokal auf Versions-Worktree nach Bump |
| 7 | `cargo clippy -D warnings` | **PASS** | CI @ `d76dab7` (Versions-PR muss erneut grün werden) |
| 7 | `cargo test --workspace --locked` | **PASS** | CI @ `d76dab7` |
| 7 | `cargo build/check --release --locked` | **PASS** | Release-Preflight-Workflow-Äquivalent in CI-Linux-Jobs @ `d76dab7`; Versions-PR CI ausstehend |
| 7 | MSRV 1.89 | **PASS** | CI-Job `msrv` @ `d76dab7` |
| 7 | `cargo audit` | **PASS** (mit Follow-up) | CI; 3× unmaintained `chrono`-Familie |
| 9 | Version `0.2.0` / 16 Crates / Lockfile | **PASS** (lokal) | `scripts/publish-workspace.py --version 0.2.0 --plan`; `fexu-cli` `release_set`; Kandidat-PR `cursor/release-020-version` |
| 9 | Publish-Preflight (`publish-workspace.py` / `publish.yml` **ohne** Publish) | **PARTIAL** | Lokal: `--plan` + `cargo fmt --check` PASS auf Versions-SHA; voller GHA-Preflight (`release-preflight.yml`) und `cargo test/clippy/release` auf **Versions-SHA** noch nicht abgeschlossen |
| 9 | Maintainer-Sign-off / Tag / crates.io | **BLOCKED** | Bewusst außerhalb Scope; kein Merge/Tag/Publish |

### Argon2-Matrix (dieser Lauf)

| Serie | Befehl | Läufe | PASS | AV (0xc0000005) |
|---|---|---:|---:|---:|
| debug-8 | `fexu-auth-crash-minimal.exe 8 8388608` | 25 | 25 | 0 |
| debug-24 | `fexu-auth-crash-minimal.exe 24 8388608` | 25 | 25 | 0 |
| release-8 | `fexu-auth-crash-minimal.exe 8 8388608` | 25 | 25 | 0 |
| release-24 | `fexu-auth-crash-minimal.exe 24 8388608` | 25 | 25 | 0 |
| **Summe** | | **100** | **100** | **0** |

Stack: 8 388 608 Byte; Argon2 **0.5.3** (Parameter unverändert); rustc **1.100.0**.

## Preflight (Versions-SHA, ohne Publish)

| Schritt | Ergebnis |
|---|---|
| `python3 scripts/publish-workspace.py --version 0.2.0 --plan` | **PASS** — 16 Crates in Abhängigkeitsreihenfolge |
| `cargo fmt --all -- --check` | **PASS** |
| `cargo metadata` — 16× Version `0.2.0` | **PASS** |
| `publish.yml`-Gates `clippy` / `test --workspace` / `release` build | **Ausstehend** auf Versions-PR-CI bzw. manuelles `workflow_dispatch` |

## Änderungen Versions-PR

- Workspace + 16 publishable Crates `0.2.0-dev.1` → `0.2.0`; `Cargo.lock`; `PUBLISHED_FEXU_VERSION` / Generator-Tests `=0.2.0`
- `CHANGELOG.md`, README, Manual/Docs-Kandidatversionen
- Kein Argon2-Parameter-Change; kein Tag; kein crates.io-Upload

## Verbleibende Blocker

1. **CI auf Versions-Kandidat-SHA** (inkl. erneuter 0.1.2-Upgrade-Regressionen mit Generator `=0.2.0`-Pin).
2. **GHA Release candidate preflight** (`release-preflight.yml`, Input `0.2.0`) auf finalem Versions-SHA.
3. **Maintainer-Sign-off**, Tag, GitHub Release, crates.io — nicht autorisiert.
4. **Optionale Gates:** Admin-CRUD auf Upgrade-App; historische 30/180 AV-Reproduktion.

## Urteil

### **NO-GO**

Alle **mandatory Consumer-Upgrade-Gates** (SQLite/PostgreSQL Fullstack inkl. CI @ `d76dab7`) und der **konsistente 0.2.0-Versionsgraph** sind dokumentiert. **TECHNICAL GO** ist noch nicht gerechtfertigt: Maintainer-Sign-off fehlt, Publish-Preflight auf dem Versions-SHA ist nur teilweise lokal belegt, optionale/erweiterte Gates sind offen, und **kein** Merge/Tag/Publish ist autorisiert.