# Windows Release-Audit — 22.11.2026

**Repository:** [Iam-Sanja/fexu-rs](https://github.com/Iam-Sanja/fexu-rs)  
**Audit-Basis (origin/main):** `3a0ac3fc1b4a86fbe21fd179cfce8e383ce6b101`  
**Entwicklungslabel im Workspace:** `0.2.0-dev.1`  
**Zuletzt auf crates.io:** `0.1.2`  
**Audit-Branch:** `cursor/windows-release-audit-78b7`  
**Arbeitskopie:** isoliertes Git-Worktree `D:\04_Projekte_Arbeit\Projekte\Fexu\audit-worktree-20261122` (Haupt-Checkout `D:\04_Projekte_Arbeit\Projekte\Fexu\main` unverändert auf lokalem `main`, hinter `origin/main`)

---

## Commit und Umgebung

| Kennzahl | Wert |
|---|---|
| Audit-Commit (Basis) | `3a0ac3f` — *test(release): published 0.1.2 auth/session transition on isolated PostgreSQL (#249)* |
| rustc | `1.100.0` (x86_64-pc-windows-msvc, LLVM 23.1.5) |
| cargo | `1.100.0` |
| rust-toolchain.toml | `stable`, Komponenten `rustfmt`, `clippy`; `rust-version`/`MSRV` 1.89.0 |
| Windows | NT 10.0.26300.0 (Build 26200-Klasse) |
| CPU | 13th Gen Intel Core i7-13700K |
| Logische Prozessoren | 24 |
| Cargo.lock (relevant) | `argon2` **0.5.3**, `sqlx-core` / `sqlx-postgres` **0.8.6**, `rusqlite` **0.42.0**, `sea-query` **0.32.7** |

**Nach Windows-Neuinstallation fehlende Werkzeuge (installiert für dieses Audit):**

- Visual Studio Build Tools 2022 **17.14.45** (Workload VC Tools) — ohne MSVC war `link.exe` nicht verfügbar (**BLOCKER** für lokale Builds, behoben im Audit).
- `cargo-audit` **0.23.0** via `cargo install cargo-audit --locked`.
- `psql` / lokaler PostgreSQL-Server: **nicht** vorhanden (PostgreSQL-Integrationstests lokal übersprungen).
- `rustup` im PATH: **nicht** gefunden (nur einzelnes `rustc`/`cargo`).

**Gelesene TestShop-Berichte (historisch, Pfade vorhanden):**

- `D:\04_Projekte_Arbeit\TestShop\reports\windows-auth-crash-investigation-20261121.md`
- `D:\04_Projekte_Arbeit\TestShop\reports\windows-auth-crash-cursor-assessment-20261121.md`
- `D:\04_Projekte_Arbeit\TestShop\reports\windows-argon2-followup-20261121.md`

Diese beschreiben **frühere** Messungen (u. a. 180/30 AV unter Debug/24 Threads); sie sind **kein** Nachweis für das heutige Verhalten nach Reinstall/BIOS-Update.

---

## Tatsächlich ausgeführte Befehle

Alle Build-/Test-Befehle nach Installation der Build Tools über `vcvars64.bat` und Wechsel ins Audit-Worktree:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --workspace --all-targets --locked
cargo build --workspace --locked
cargo build --workspace --locked --release
cargo test -p fexu-storage --lib traversal_mime
cargo audit
```

**Argon2-Reproducer** (`scripts/repro-windows-auth-crash`):

```text
cargo build --locked
cargo build --locked --release
# Pro Lauf: fexu-auth-crash-minimal.exe <threads> 8388608
```

**Fullstack-Smoke (Scratch, außerhalb des Repos):** Entwicklungs-CLI `target\debug\fexu.exe` mit `new project bookstore --template fullstack --fexu-path <repo>`, `migrate up`, `check` (siehe Abschnitt Fullstack).

**Git:**

```text
git fetch origin
git worktree add -b cursor/windows-release-audit-78b7 <audit-worktree> origin/main
```

---

## Qualität: bestanden / fehlgeschlagen / übersprungen

| Schritt | Ergebnis | Anmerkung |
|---|---|---|
| `cargo fmt --check` | **VERIFIED** PASS | Exit 0 |
| `cargo clippy -D warnings` | **VERIFIED** PASS | Exit 0; Cargo-Manifest-Warnungen (`unused workspace dependency`, `redundant homepage`) — keine Rust-Clippy-Verstöße |
| `cargo test --workspace` (erster Lauf) | **HIGH** FAIL | `fexu-storage::traversal_mime_partials_and_symlinks_do_not_escape` — Windows-Fehler 1314 (Symlink-Recht); danach Fix + Einzeltest PASS |
| `cargo check --locked` | **VERIFIED** PASS | |
| `cargo build` debug + release `--locked` | **VERIFIED** PASS | |
| PostgreSQL-Tests | **NOT TESTED** (lokal) | ~105 `ignored` wegen fehlendem `DATABASE_URL`/Server; **VERIFIED** in CI auf `3a0ac3f` |
| CI `origin/main` @ `3a0ac3f` | **VERIFIED** | 12/12 Checks grün inkl. `Rust (windows-latest)`, Postgres, Browser, MSRV, Published 0.1.2 PG auth upgrade, Cargo audit |

**Hinweis:** Vollständiger Workspace-`cargo test` nach dem Storage-Fix wurde **nicht** erneut komplett ausgeführt (Zeitbudget); nur der regressionsrelevante Test wurde **VERIFIED**.

---

## Argon2-Messreihe (dieser Lauf)

**Toolchain:** rustc 1.100.0, Argon2 **0.5.3**, Stack **8 388 608** Byte, Reproducer wie in `scripts/repro-windows-auth-crash`.

| Serie | Befehl | Läufe | PASS | AV (0xc0000005) | Sonstiges |
|---|---|---:|---:|---:|---|
| Smoke | `debug.exe 24 8388608` | 1 | 1 | 0 | Exit 0 |
| debug / 8 Threads | `debug.exe 8 8388608` | 10 | 10 | 0 | Exit 0 |
| debug / 24 Threads | `debug.exe 24 8388608` | 10 | 10 | 0 | Exit 0 |
| release / 8 Threads | `release.exe 8 8388608` | 10 | 10 | 0 | Exit 0 |
| release / 24 Threads | `release.exe 24 8388608` | 10 | 10 | 0 | Exit 0 |
| **Summe** | | **41** | **41** | **0** | |

Rohdaten: `.audit-argon2-results.json` (nicht committet).

**Einordnung (LOW bis MEDIUM Risiko, nicht als Release-Freigabe):**

- In **dieser** Session auf frisch eingerichtetem Windows: **kein** AV — widerspricht nicht der historischen 30/180-Matrix vom 21.11. (vor Reinstall).
- Ursache weiter **unbewiesen** (Argon2, Codegen, Intermittenz, Hardware); **kein** Produktionsfix durch opt-level oder Thread-Reduktion.
- `fexu-auth`-Tests (48 passed) liefen im Workspace-Test ohne AV; isolierter Reproducer deckt Framework-DB/HTTP nicht ab.

---

## Reproduzierbare Bugs (dieser Lauf)

| ID | Befund | Schwere | Status |
|---|---|---|---|
| R1 | `cargo test` scheiterte an Storage-Symlink-Test ohne Developer Mode / SeCreateSymbolicLink | **MEDIUM** | **VERIFIED** behoben (siehe Dateiänderungen); entspricht Muster in `fexu-static` |
| R2 | Fehlendes MSVC nach OS-Reinstall blockierte alle Kompilationen | **BLOCKER** (Umgebung) | **VERIFIED** behoben durch Build Tools 17.14.45 |
| R3 | Historische parallele Argon2-AV unter Windows Debug/24 Threads | **HIGH** (Release-Risiko) | **NOT TESTED** reproduzierbar heute; historisch **VERIFIED** in TestShop-Berichten |

---

## Sicherheit (Code-Review + cargo audit)

| Thema | Befund | Label |
|---|---|---|
| Passwort-Hashing | Argon2id, Parameter in `fexu-auth` konsistent mit Reproducer; keine Schwäche im Code nachgewiesen | **VERIFIED** |
| Sessions / Cookies / CSRF | Abgedeckt durch umfangreiche `fexu-auth`/`admin-browser`-Tests; kein neuer Exploit in diesem Audit | **VERIFIED** (Unit/Integration), **NOT TESTED** (manuelles HTTP auf Scratch-App) |
| SQL-Injection | ORM/SQLx-Pfad; keine direkte String-Konkatenation in geprüften Auth-Pfaden | **LOW** |
| Logging von Secrets | `fexu-middleware` filtert Query/Cookie-Secrets aus Log-Pfaden (Tests vorhanden) | **VERIFIED** |
| cargo audit | Keine kritischen CVEs; **3× unmaintained** (`chrono`, `chrono-tz`, `chrono-tz-build`) | **MEDIUM** Follow-up |
| Keine Schwachstelle ohne Beleg | — | — |

**Keine** als „Vulnerability“ deklarierte neue Lücke ohne Test/Code-Beleg.

**Major-Upgrade-Follow-ups (nicht durchgeführt):** Dependabot-Zweige für `argon2` 0.6, `sqlx` 0.9, `rusqlite` 0.43, `sea-query` 1.x — separat bewerten.

---

## Upgrade-Nachweise (PR #247–#249)

| Upgrade-Fall | CI / Code | Lokal Windows | Bewertung |
|---|---|---|---|
| Bookstore-Generator Smoke (#247) | **VERIFIED** (Linux CI) | **VERIFIED** partiell: Generierung, `cargo check`, `migrate up`, `books`-Schema in `db.sqlite3` | Kein HTTP-Admin |
| SQLite Auth 0.1.2 → Dev-Binaries (#248) | **VERIFIED** (Linux CI) | **NOT TESTED** (bash-Skript) | Bibliotheks-Upgrade, kein generiertes Vollprojekt |
| PostgreSQL Auth 0.1.2 → Dev (#249) | **VERIFIED** (dedizierter CI-Job) | **NOT TESTED** (kein Postgres) | Wie oben |
| Echte 0.1.2-**App** inkl. alte Migrationsdateien | — | **NOT TESTED** | **BLOCKER** für 0.2.0-GO |
| Bestehende Passwort-Hashes | Teilweise (#248/#249 Consumer) | — | **VERIFIED** CI |
| Bestehende Sessions / Rollen | Teilweise (#248/#249) | — | **VERIFIED** CI |
| Fehlgeschlagene Migration ohne Datenverlust | `schema_evolution` o. Ä. | — | **VERIFIED** CI (nicht externes Binary) |
| Fullstack + Browser-Login über Upgrade-Grenze | — | **NOT TESTED** | **HIGH** Lücke |

---

## Fullstack-Scratch (Windows)

| Prüfpunkt | Status |
|---|---|
| Projektgenerierung fullstack + `--fexu-path` | **VERIFIED** |
| Modell/Migration `migrate make` / `up` | **VERIFIED** |
| `fexu check` / `cargo check` | **VERIFIED** |
| Serverstart, Admin-Login, Session-Cookie, CSRF, CRUD, Neustart | **NOT TESTED** |

---

## Dokumentation vs. CLI/API

| Thema | Abgleich | Label |
|---|---|---|
| Version `0.2.0-dev.1` nicht auf crates.io | `manual/versions.md`, Installation → 0.1.2 | **VERIFIED** konsistent |
| Upgrade-Anleitung vs. `release-candidate-0.2.md` | Manual `how-to/upgrade-0.1.2-to-0.2.md` nennt Auth-Session-Upgrade noch als „nicht“ / partiell blockiert; `release-candidate-0.2.md` auf `3a0ac3f` verweist bereits auf #248/#249 | **MEDIUM** Abweichung |
| Bookstore-Tutorial | Disclaimer „not yet verified“ vs. CI-Smoke #247 | **LOW** — Tutorial strenger formuliert als CI-Smoke |
| Installationsbefehle | `cargo install fexu-cli --version 0.1.2` stimmt mit crates.io | **VERIFIED** |

---

## Geänderte Dateien in diesem Audit

| Datei | Ursache | Regressionstest |
|---|---|---|
| `crates/fexu-storage/src/local.rs` | Symlink-Hilfen wie `fexu-static`: bei fehlendem Windows-Symlink-Recht Testteil überspringen statt paniken | `cargo test -p fexu-storage --lib traversal_mime` **VERIFIED** PASS |
| `docs/audits/windows-release-audit-2026-11-22.md` | Audit-Artefakt | — |

---

## Offene Risiken

1. **BLOCKER:** Kein genehmigtes `0.2.0`-Release-Paket, kein Preflight auf finaler Kandidaten-Version, Maintainer-Sign-off offen (`docs/release-candidate-0.2.md`).
2. **HIGH:** Vollständiger Upgrade-Pfad **generierte 0.1.2-Anwendung** (SQLite + PostgreSQL) inkl. HTTP/Admin nicht unabhängig nachgewiesen.
3. **HIGH:** Historische **intermittierende** Argon2-AV unter Windows (Debug, hohe Parallelität) — heute nicht ausgelöst, Ursache ungeklärt; weiter relevant für parallele Auth-Tests auf Windows-Entwicklern.
4. **MEDIUM:** `chrono`-Ökosystem „unmaintained“ (cargo audit).
5. **MEDIUM:** Lokaler Windows-`cargo test` ohne Developer Mode war bis Storage-Fix fehleranfällig.

---

## Urteil

### **NO-GO** für Veröffentlichung von **Fexu 0.2.0** auf crates.io

**Begründung (nur Evidenz dieses Laufs):** Die Qualitätsbasis auf `3a0ac3f` ist in **CI** stark (**VERIFIED**), und dieser Windows-Audit zeigt nach MSVC-Nachrüstung **grüne** Builds sowie **41/41** Argon2-Reproducer-PASS. Das hebt das vorherige NO-GO **nicht** auf: Release-Gates für echte App-Upgrades, finale Version `0.2.0` (nicht `-dev.1`), Preflight, Maintainer-Freigabe und geklärte Windows-Auth-Stabilität unter Last bleiben offen. Keine neuen Messungen rechtfertigen opt-level=3 oder Thread-Drosselung als Fix.

**Primäre Blocker:** Release-Pipeline/Preflight (#1), fehlender Nachweis echter 0.1.2-Consumer-App-Upgrades (#2), ungeklärte historische Argon2-AV (#3).