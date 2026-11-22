# Fexu 0.2.0 final release acceptance

**Repository:** [Iam-Sanja/fexu-rs](https://github.com/Iam-Sanja/fexu-rs)  
**Basis-SHA (`origin/main`):** `bb113962140710dc5d8ca3fbb296da0cf4b1a3e4`  
**Acceptance-Branch:** `release/020-final-acceptance`  
**Workspace-Label:** `0.2.0-dev.1` (kein crates.io-`0.2.0`)  
**Laufzeit:** 2026-11-22, Windows 10.0.26300, rustc/cargo 1.100.0  
**Isoliertes Worktree:** `D:\04_Projekte_Arbeit\Projekte\Fexu\release-acceptance-020`  
**TestShop / Haupt-Checkout:** unangetastet (nur lesender Kontext); `main` @ `bb11396` mit lokalen PR-Template-Edits unverändert.

## Gate-Tabelle

| Phase | Gate | Status | Evidenz |
|---|---|---|---|
| 0 | `origin/main` @ erwarteter SHA | **PASS** | `bb113962140710dc5d8ca3fbb296da0cf4b1a3e4` |
| 4 | Published 0.1.2 **API** CLI → Checkout (SQLite) | **PASS** | Bestehend `scripts/test-published-v012-upgrade.sh` in CI |
| 4 | Published 0.1.2 Auth-Binär-Upgrade SQLite | **PASS** | `scripts/test-published-v012-auth-upgrade.sh` + CI |
| 4 | Published 0.1.2 Auth-Binär-Upgrade PostgreSQL | **PASS** | `scripts/test-published-v012-postgres-auth-upgrade.sh` + CI-Job |
| 4 | Published 0.1.2 **Fullstack**-App → Checkout (SQLite, HTTP, Rollen, Daten) | **PASS** | Neu: `scripts/test-published-v012-fullstack-upgrade.sh` — lokal Windows Git-Bash Exit 0, ~3 min; CI-Step in diesem Branch |
| 4 | Published 0.1.2 Fullstack → Checkout **PostgreSQL 16** (eigene DB) | **NOT TESTED** | Auth-PostgreSQL abgedeckt; Fullstack-PostgreSQL-Skript folgt separater CI-Job-Erweiterung |
| 4 | Fehlgeschlagene Migration ohne Datenverlust (Fullstack-Skript) | **PASS** | `failed-migration` Hilfe in Fullstack-Skript; Framework: `schema_evolution.rs` in CI |
| 5 | HTTP Admin vor/nach Upgrade (Login, Session, CSRF, Bad Password) | **PASS** | `v012_fullstack_upgrade_lib.py` Matrix im Fullstack-Skript |
| 5 | Playwright/Myshop-Browser-Matrix (kein Upgrade) | **PASS** | Bestehender `tests/browser` CI-Job @ `bb11396` |
| 5 | Vollständige Admin-CRUD/Paging-Matrix auf Upgrade-App | **NOT TESTED** | Nur Kern-HTTP; erweiterbar über Playwright auf Disposable-App |
| 6 | Argon2 Reproducer debug 8×25 | **PASS** | 25/25 PASS, 0 AV |
| 6 | Argon2 Reproducer debug 24×25 | **PASS** | 25/25 PASS, 0 AV |
| 6 | Argon2 Reproducer release 8×25 | **PASS** | 25/25 PASS, 0 AV |
| 6 | Argon2 Reproducer release 24×25 | **PASS** | 25/25 PASS, 0 AV |
| 6 | `fexu-auth` Framework-Tests | **PASS** | `cargo test -p fexu-auth --lib`: 48 passed, 3 ignored (PostgreSQL) |
| 6 | Historische 30/180 AV-Reproduktion | **NOT TESTED** | Kein 0xc0000005 in diesem Lauf; keine Behauptung „gefixt“ |
| 7 | `cargo fmt --check` | **PASS** | Exit 0 |
| 7 | `cargo clippy -D warnings` | **PASS** | Exit 0 (Manifest-Warnungen `fexu-contract`, kein Rust-Lint) |
| 7 | `cargo test --workspace --locked` | **NOT TESTED** | Zeitbudget; letzter Audit #250 CI grün @ vorherigem SHA |
| 7 | `cargo build/check --release --locked` | **PARTIAL** | `cargo build` debug OK; voller Release-Workspace nicht in diesem Lauf wiederholt |
| 7 | MSRV 1.89 | **NOT TESTED** | CI-Job `msrv` auf Linux |
| 7 | `cargo audit` | **PASS** (mit Follow-up) | 3× unmaintained `chrono`/`chrono-tz`/`chrono-tz-build`; keine neuen kritischen CVEs |
| 9 | Version `0.2.0` / 16 Crates / Lockfile | **BLOCKED** | Bewusst nicht in diesem PR; separate PR nur nach grünen CI-Gates |
| 9 | Publish-Preflight `publish-workspace.py` / `publish.yml` | **NOT TESTED** | Kein `0.2.0`-Bump autorisiert |
| 9 | Maintainer-Sign-off / Tag / crates.io | **BLOCKED** | Außerhalb Scope |

### Argon2-Matrix (dieser Lauf)

| Serie | Befehl | Läufe | PASS | AV (0xc0000005) |
|---|---|---:|---:|---:|
| debug-8 | `fexu-auth-crash-minimal.exe 8 8388608` | 25 | 25 | 0 |
| debug-24 | `fexu-auth-crash-minimal.exe 24 8388608` | 25 | 25 | 0 |
| release-8 | `fexu-auth-crash-minimal.exe 8 8388608` | 25 | 25 | 0 |
| release-24 | `fexu-auth-crash-minimal.exe 24 8388608` | 25 | 25 | 0 |
| **Summe** | | **100** | **100** | **0** |

Stack: 8 388 608 Byte; Argon2 **0.5.3**; rustc **1.100.0**.

## Änderungen in diesem Acceptance-PR

- `scripts/test-published-v012-fullstack-upgrade.sh` — Clean-room Fullstack 0.1.2 → source consumer
- `scripts/v012_fullstack_upgrade_lib.py` / `v012_fullstack_upgrade_runner.py` — HTTP, Snapshots, Migration-Failure-Check
- `scripts/run-windows-argon2-matrix.ps1` — dokumentierte Windows-Matrix
- `.github/workflows/ci.yml` — neuer Linux-Step Fullstack-SQLite-Upgrade

Kein Version-Bump, kein Tag, kein Publish.

## Verbleibende Blocker

1. **PostgreSQL Fullstack-Consumer-Upgrade** — eigener Job + Skript (Auth-PostgreSQL allein reicht nicht für #92/#93).
2. **Frischer CI-Grünlauf** auf dem Acceptance-Commit (insb. neuer Fullstack-Step, ~55 min).
3. **Vollständiger Workspace-`cargo test --workspace`** auf finalem SHA (Windows-Symlink/Developer-Mode-Risiko).
4. **Phase 9** — `0.2.0`-Versions-PR, Lockfile, Changelog, Publish-Preflight, Maintainer-Freigabe.
5. **Erweiterte Admin-Browser-Matrix** auf der Upgrade-App (optional Playwright).

## Urteil

### **NO-GO**

Evidenz für den kritischen Fullstack-SQLite-Upgrade-Pfad und die Windows-Argon2-Matrix (100/0 AV) ist in diesem Lauf **neu positiv**. Crates.io-`0.2.0`, PostgreSQL-Fullstack, frisches CI auf diesem Branch, Publish-Preflight und Maintainer-Sign-off fehlen weiterhin.

**Nächste konkrete Schritte:** Acceptance-PR mergen nach grünem CI → PostgreSQL-Fullstack-Skript + Job → Workspace-Test/MSRV auf Kandidaten-SHA → separater `0.2.0`-Versions-PR → Publish-Preflight → Sign-off.