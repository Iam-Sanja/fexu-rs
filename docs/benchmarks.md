# Benchmark baseline

Checked on 2026-11-17. This page describes a repeatable Fexu measurement and a feature matrix. It is not a ranking. Runique, Cot and Loco were not built or timed in this round. Cells that were not measured say so.

## Fexu workload

The crate `examples/bench-baseline` is a separate binary. It uses the public `FexuModel`, `App`, auth and SQLite APIs. It does not change those APIs.

The fixture is one user with `bench.read`, `bench.create`, `bench.update` and `bench.delete`, twenty categories, and ten items on each category. Every item stores `category_id` with `ON DELETE RESTRICT`. The release plan is fixed in `release_plan`:

| Phase | Requests | What it hits |
|---|---|---|
| Setup | not mixed into the request percentiles | auth tables, schema, 200 seeded rows, one login |
| Discover | 1, excluded from percentiles | `GET /api/v1/bench_items?limit=20` must return 20 rows, `has_more: true` and a `k1.` cursor |
| Warmup | 40 passes over the five read URLs | same URLs as the read phase, discarded |
| Sequential reads | 300 | list sorted by name, the following cursor page, one item, its category, and `category_id:eq` |
| Sequential writes | 40 | `PATCH` the same item name back to its seeded value |
| Missing session | 10 | `GET` the list with no cookie, status 401 |
| Missing row | 10 | `GET` a fixed unknown UUID, status 404 |
| Rejected limit | 10 | `GET ?limit=101`, status 422 |
| Concurrent reads | 4 workers, 40 list requests each | same sorted list URL |

Writes send `Origin: http://127.0.0.1:3000` and `x-csrf-token`. That origin is the development allow-list entry. The server itself listens on `127.0.0.1` with an ephemeral port. Login is one Argon2 check and is reported as `login_ns`, not as part of the read rate.

SQLite comes from `fexu::db::connect`, which sets `max_connections` to 1. The benchmark does not raise that limit. Concurrent latency therefore includes waiting for that single connection. The concurrent rate divides all worker requests by the parent wall clock.

## How a number is taken

- Latency is client wall time for one HTTP/1.1 request on a keep-alive TCP connection with `TCP_NODELAY`. It includes the server and loopback. It is not an in-process Axum call, and it is not a multi-machine network.
- Percentiles use nearest rank on the sorted samples: index `ceil(p * n) - 1`. `p50`, `p95` and `p99` are that rank. The minimum and maximum are the ends of the same list.
- Requests per second are `count / elapsed` for that phase only. Warmup, login, schema and seed are outside that elapsed time.
- Startup is the binary's first instant until the loopback socket is bound, after auth, schema and seed. `auth_setup_ns`, `schema_ns` and `seed_ns` are the pieces inside that interval.
- Memory on Windows is `K32GetProcessMemoryInfo` after the workload: current and peak working set. Other operating systems write `measured: false`. The script does not sample a second process.
- Build time is the script's wall clock around `cargo build --release -p fexu-bench-baseline`. A warm `target/` directory is incremental. The JSON says that. It is not a clean-build measurement unless the directory was empty.
- The binary refuses to write a result unless it was compiled without debug assertions.

A failed status aborts the run. The harness does not substitute another request or another status.

## Commands

From the repository root, on Windows:

```powershell
powershell -NoProfile -File scripts\benchmark-baseline.ps1 3
```

On Linux:

```sh
sh scripts/benchmark-baseline.sh 3
```

The argument is the number of fresh SQLite files. Each run creates `examples/bench-baseline/runs/<stamp>/` with `host.json`, `build.json`, `run-N.json` and `data-N/baseline.sqlite`. That directory is gitignored. `host.json` records CPU, memory, OS, `rustc -V` and the git commit. It does not record the computer name or the user name.

The correctness test is not a performance result:

```sh
cargo test -p fexu-bench-baseline --test harness
```

It uses `sqlite::memory:` and `check_plan`, which is smaller than the release plan and still requires `has_more`.

## Raw files

`host.json` and `build.json` come from the script. `run-N.json` comes from the binary and uses schema `1`.

| Field | Meaning |
|---|---|
| `profile` | `release` for a published sample |
| `database` | `sqlite` in this round |
| `plan` | the counts above |
| `startup_ready_ns` | process start through listen, including seed |
| `login_ns` | one successful login |
| `sequential_reads` | status, elapsed time, requests per second, `latency_ns` |
| `sequential_writes` | same shape for PATCH |
| `unauthorized`, `not_found`, `invalid` | the three error phases |
| `concurrent_reads` | present for the release plan, `null` for `check_plan` |
| `memory.measured` | `false` when this operating system has no sample |
| `not_measured` | names this run did not time |

Times are integer nanoseconds. Rates are JSON numbers. A missing external framework is a name in `not_measured`, not a zero.

## Sample

`examples/bench-baseline/samples/2026-11-17-windows/` is one Windows session of three release runs. Read it with its `host.json`. `git_commit` there is the commit that contains the harness. The sample files were added after that commit and do not change the binary. Do not treat the sample as a Linux number, a PostgreSQL number, or a comparison with Runique, Cot or Loco. Repeating the script on another machine will not reproduce the same integers. Startup in that sample is dominated by inserting the 200 seeded rows; `seed_ns` is the piece to read before quoting `startup_ready_ns` as listen time.

## Feature matrix

Sources for the other frameworks are the crates.io API responses saved on 2026-11-17 and the `master` README files fetched the same day. The published crate versions were `runique` 4.0.0 (2026-11-08), `cot` 0.8.0 (2026-11-09), `loco-rs` 1.3.1 and generator `loco` 1.3.1 (2026-11-06). The Runique README on `master` also says the workspace is `4.1.0-preview` and that 4.0.0 is the crates.io release. This matrix uses 4.0.0 for the published crate and quotes the README only where it describes that tree.

No setup-step count, line count, migration timing or deployment timing was recorded for any framework, including Fexu. The Fexu column for performance points at the sample above. The other performance cells are not measured.

| Topic | Fexu 0.1.2 | Runique 4.0.0 | Cot 0.8.0 | Loco 1.3.1 |
|---|---|---|---|---|
| Where this was checked | This repository | [crates.io/crates/runique](https://crates.io/crates/runique), [README](https://github.com/seb-alliot/runique/blob/master/README.md) | [crates.io/crates/cot](https://crates.io/crates/cot), [README](https://github.com/cot-rs/cot/blob/master/README.md) | [crates.io/crates/loco-rs](https://crates.io/crates/loco-rs), [crates.io/crates/loco](https://crates.io/crates/loco), [README](https://github.com/loco-rs/loco/blob/master/README.md) |
| MSRV on the published crate | Workspace `rust-version` is 1.89.0 | `rust_version` 1.94 | `rust_version` 1.94 | `rust_version` 1.94 |
| Databases named by the crate features | SQLite through rusqlite and PostgreSQL through SQLx. This baseline ran SQLite only | Features `sqlite`, `postgres`, `mysql`, `mariadb`. README says one backend feature at a time; `all-databases` exists for tooling | Default features include `sqlite`, `postgres` and `mysql` together. `db` is the shared feature | Default features include `db-sqlite` and `db-postgres` |
| Auth | Sessions, roles, CSRF and Argon2 in this repository. The baseline logs in once and then sends the session cookie | README describes Argon2, CSRF and encrypted database sessions. Not executed here | The 0.8.0 feature list has no feature named `auth`. No auth API was inferred from that, and none was executed | Default feature `auth`, with extra `auth_session` and `auth_oauth` features. Not executed here |
| ORM and migrations | SeaQuery, explicit migration files, not applied at process start. The baseline calls `bootstrap_schema` for its own tables and the existing auth migration API | README: SeaORM, `runique makemigrations`, `runique migration up/down/status`. Not executed here | README: its own ORM and automatically created migrations. Not executed here | README: model without handwritten SQL, and `cargo install sea-orm-cli` when a database is needed. Feature flags pull `sea-orm-migration`. Not executed here |
| Admin | Native admin in this repository. The baseline does not open it | README: typed `ModelAdmin`, admin described as preview. Feature `admin` exists. Not executed here | README says an admin panel is included. The 0.8.0 feature list has no feature named `admin`. Not executed here | Not described as a built-in admin in the README or feature list checked here |
| Forms and validation | Model field constraints and `FexuForm` exist in this repository. The baseline exercises JSON create/patch rules and a 422 limit, not an HTML form | README: `#[form]` and server-side validation. Not executed here | Not stated in the README fetched here | README: controllers handle parameters, body and validation. Not executed here |
| HTTP API and OpenAPI | Generated CRUD and OpenAPI 3.1 in this repository. The baseline calls the generated JSON routes | No OpenAPI feature on the 4.0.0 feature list. Not executed here | Features `openapi` and `swagger-ui`, not in the default set. Not executed here | No OpenAPI feature on the 1.3.1 feature list. README describes Axum controllers. Not executed here |
| Tests | Workspace tests on Windows and Linux CI, plus this harness. The ignored PostgreSQL jobs were not run for this page | README reports 3702 passing tests and a coverage snapshot dated 2026-11-08. That is the project's report. It was not re-run | Feature `test`. CI badge exists. The suite was not re-run | Features `testing`, `testing-core` and `testing-html`. The suite was not re-run |
| Deployment | Not timed. Existing docs describe the repository's own deployment files. Those files were not booted here | README shows `cargo run`. No container run was performed | README shows `cargo run`. No container run was performed | README shows `cargo loco start` after `loco new`. No container run was performed |
| Requests per second, latency, memory, startup | One Windows SQLite sample, three release runs, recorded under `examples/bench-baseline/samples/2026-11-17-windows/`. Not a capacity claim | Not measured | Not measured | Not measured |

## Limits

- PostgreSQL was not used. Issue work on the PostgreSQL test infrastructure is separate, and this harness does not open a server database.
- Linux and macOS were not timed in the committed sample. The shell script is there so a Linux run can write the same files. Until that file exists, those operating systems stay not measured.
- The build JSON is incremental if `target/` already contains a release build.
- Working set is the benchmark process after the run. It is not peak RSS of a long-lived server and not a container limit.
- External test counts and admin claims are quotations from the fetched pages. They are not results produced in this repository.
- Issue #95 stays open until the same application is built and timed on Runique, Cot and Loco, and until PostgreSQL is part of a measured run.
