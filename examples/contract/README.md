# Contract suite

Independent checks for the public Fexu API. This package adds no application routes.

## Executed by default

`cargo test -p fexu-contract --test cross_backend sqlite_contract_login_matches_the_browser_form`

SQLite memory database. The test builds auth, the native admin, and `require_browser_writes`, then sends the login form through `TestClient`. It checks one `Referrer-Policy: same-origin`, forced `nosniff` and `DENY`, a fresh cookie jar, a missing token, a mismatched token, a foreign origin, a wrong password, and a successful login.

## Ignored unless CI or a local admin URL is present

`postgres_contract_login_matches_sqlite` uses `DisposablePostgres`. It creates `fexu_test_` plus 32 hex characters and drops only that database. An ignored result is not a pass. The PostgreSQL job runs it with `--exact` and fails unless the output contains `1 passed`.

The same job runs `postgres_fixture_parallel_create_isolate_and_cleanup` the same way. That test is the #117 regression: a session may sit on `template1` while fixtures clone `template0`.

## Not covered here

No PostgreSQL server was listening on this workstation, so the ignored PostgreSQL tests were not executed locally. This suite does not generate a project with `fexu new`, does not open a TLS listener, does not bind a LAN socket, and does not start Docker. The Playwright spec `tests/browser/login-form.spec.js` posts the myshop login form from a fresh profile. Myshop does not call `require_browser_writes`; the contract test above does.
