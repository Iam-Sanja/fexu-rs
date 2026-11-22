#!/usr/bin/env python3
"""Shared helpers for published 0.1.2 fullstack consumer upgrade regressions."""

from __future__ import annotations

import json
import os
import re
import signal
import sqlite3
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from http.cookiejar import CookieJar
from pathlib import Path
from typing import Any


def patch_database_url_resolver(project_mod: Path) -> None:
    source = project_mod.read_text(encoding="utf-8")
    old = "    let db = fexu::db::connect(&settings.resolved_database_url()).await?;"
    new = (
        "    let database_url = settings.resolved_database_url()?;\n"
        "    let db = fexu::db::connect(&database_url).await?;"
    )
    if source.count(old) != 1:
        raise SystemExit(f"published fullstack source no longer matches pinned API: {project_mod}")
    project_mod.write_text(source.replace(old, new), encoding="utf-8")


def rewrite_fullstack_dependency(manifest: Path, fexu_crate: Path) -> None:
    text = manifest.read_text(encoding="utf-8")
    old = 'fexu = { version = "=0.1.2", features = ["admin-ui"] }'
    if text.count(old) != 1:
        raise SystemExit("published fullstack generator changed dependency contract")
    path = fexu_crate.as_posix()
    manifest.write_text(
        text.replace(old, f'fexu = {{ path = "{path}", features = ["admin-ui"] }}'),
        encoding="utf-8",
    )


def add_book_author_relation(book_model: Path) -> None:
    text = book_model.read_text(encoding="utf-8")
    if "author_id" in text:
        return
    needle = "    pub name: String,\n}"
    insert = (
        "    pub name: String,\n\n"
        '    #[field(foreign_key = "authors.id", on_delete = "restrict")]\n'
        "    pub author_id: Uuid,\n}"
    )
    if needle not in text:
        raise SystemExit(f"unexpected book model layout: {book_model}")
    book_model.write_text(text.replace(needle, insert), encoding="utf-8")


def ensure_csrf_key(project: Path) -> str:
    key_path = project / ".fexu" / "csrf.key"
    if key_path.is_file():
        try:
            secret = key_path.read_text(encoding="utf-8").strip()
        except UnicodeDecodeError:
            secret = key_path.read_bytes().hex()
        if len(secret) == 64 and re.fullmatch(r"[0-9a-fA-F]+", secret):
            return secret.lower()
    secret = os.urandom(32).hex()
    key_path.parent.mkdir(parents=True, exist_ok=True)
    key_path.write_text(secret + "\n", encoding="utf-8")
    return secret


@dataclass
class AuthSnapshot:
    password_hashes: dict[str, str]
    session_tokens: list[str]
    migration_rows: list[tuple[str, str]]
    user_rows: list[tuple[str, int, int, int]]
    role_assignments: list[tuple[str, str]]


_ROLE_ASSIGNMENTS_SQL = """
SELECT u."email", r."name"
FROM "_fexu_user_roles" ur
JOIN "_fexu_users" u ON ur."user_id" = u."id"
JOIN "_fexu_roles" r ON ur."role_id" = r."id"
ORDER BY u."email", r."name"
"""


def sqlite_snapshot(db_path: Path) -> AuthSnapshot:
    with sqlite3.connect(db_path) as conn:
        hashes = {
            row[0]: row[1]
            for row in conn.execute(
                'SELECT "email", "password_hash" FROM "_fexu_users" ORDER BY "email"'
            )
        }
        sessions = [
            row[0]
            for row in conn.execute(
                'SELECT "token" FROM "_fexu_sessions" ORDER BY "created_at"'
            )
        ]
        migrations = [
            (row[0], row[1])
            for row in conn.execute(
                'SELECT "id", "checksum" FROM "_fexu_migrations" ORDER BY "id"'
            )
        ]
        users = [
            (row[0], row[1], row[2], row[3])
            for row in conn.execute(
                'SELECT "email", "is_active", "is_staff", "is_superuser" FROM "_fexu_users" ORDER BY "email"'
            )
        ]
        roles = [tuple(row) for row in conn.execute(_ROLE_ASSIGNMENTS_SQL)]
    return AuthSnapshot(hashes, sessions, migrations, users, roles)


def is_postgres_database_url(url: str) -> bool:
    return url.startswith("postgres://") or url.startswith("postgresql://")


def require_psql() -> str:
    import shutil

    psql = shutil.which("psql")
    if not psql:
        raise SystemExit("BLOCKED: psql client is required for PostgreSQL fullstack upgrade")
    return psql


def psql_query(database_url: str, sql: str) -> list[tuple[str, ...]]:
    psql = require_psql()
    result = subprocess.run(
        [psql, database_url, "-v", "ON_ERROR_STOP=1", "-At", "-F", "\t", "-c", sql],
        check=True,
        capture_output=True,
        text=True,
    )
    rows: list[tuple[str, ...]] = []
    for line in result.stdout.splitlines():
        if not line.strip():
            continue
        rows.append(tuple(line.split("\t")))
    return rows


def psql_scalar(database_url: str, sql: str) -> str:
    rows = psql_query(database_url, sql)
    if len(rows) != 1 or len(rows[0]) != 1:
        raise SystemExit(f"unexpected scalar query result for: {sql[:80]}")
    return rows[0][0]


def postgres_snapshot(database_url: str) -> AuthSnapshot:
    if not is_postgres_database_url(database_url):
        raise SystemExit("postgres_snapshot requires a postgres:// DATABASE_URL")
    hashes = {
        row[0]: row[1]
        for row in psql_query(
            database_url,
            'SELECT "email", "password_hash" FROM "_fexu_users" ORDER BY "email"',
        )
    }
    sessions = [
        row[0]
        for row in psql_query(
            database_url,
            'SELECT "token" FROM "_fexu_sessions" ORDER BY "created_at"',
        )
    ]
    migrations = [
        (row[0], row[1])
        for row in psql_query(
            database_url,
            'SELECT "id", "checksum" FROM "_fexu_migrations" ORDER BY "id"',
        )
    ]
    users = [
        (row[0], int(row[1]), int(row[2]), int(row[3]))
        for row in psql_query(
            database_url,
            'SELECT "email", "is_active", "is_staff", "is_superuser" FROM "_fexu_users" ORDER BY "email"',
        )
    ]
    roles = [tuple(row) for row in psql_query(database_url, _ROLE_ASSIGNMENTS_SQL)]
    return AuthSnapshot(hashes, sessions, migrations, users, roles)


def snapshot_from_backend(db_path: Path | None, database_url: str | None) -> AuthSnapshot:
    if database_url:
        return postgres_snapshot(database_url)
    if db_path is None:
        raise SystemExit("snapshot requires db path or DATABASE_URL")
    return sqlite_snapshot(db_path)


def write_snapshot(path: Path, snap: AuthSnapshot) -> None:
    payload = {
        "password_hashes": snap.password_hashes,
        "session_tokens": snap.session_tokens,
        "migration_rows": snap.migration_rows,
        "user_rows": snap.user_rows,
        "role_assignments": snap.role_assignments,
    }
    path.write_text(json.dumps(payload, sort_keys=True, indent=2) + "\n", encoding="utf-8")


def load_snapshot(path: Path) -> AuthSnapshot:
    data = json.loads(path.read_text(encoding="utf-8"))
    return AuthSnapshot(
        data["password_hashes"],
        data["session_tokens"],
        [tuple(row) for row in data["migration_rows"]],
        [tuple(row) for row in data["user_rows"]],
        [tuple(row) for row in data["role_assignments"]],
    )


def patch_fexu_database_url(fexu_toml: Path, database_url: str) -> None:
    text = fexu_toml.read_text(encoding="utf-8")
    old = 'url = "sqlite://db.sqlite3?mode=rwc"'
    if text.count(old) != 1:
        raise SystemExit(f"expected single default sqlite url in {fexu_toml}")
    fexu_toml.write_text(text.replace(old, f'url = "{database_url}"'), encoding="utf-8")


def assert_snapshots_unchanged(before: AuthSnapshot, after: AuthSnapshot) -> None:
    if before.password_hashes != after.password_hashes:
        raise SystemExit("password hashes changed across upgrade")
    before_hist = [row for row in before.migration_rows if not row[0].startswith("catalog:add_")]
    after_hist = [row for row in after.migration_rows if not row[0].startswith("catalog:add_")]
    if before_hist != after_hist:
        raise SystemExit("historical migration checksums changed")
    if before.user_rows != after.user_rows:
        raise SystemExit("user flags changed unexpectedly")
    if before.role_assignments != after.role_assignments:
        raise SystemExit("role assignments changed unexpectedly")


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):  # type: ignore[no-untyped-def]
        return None


class HttpClient:
    def __init__(self, base: str) -> None:
        self.base = base.rstrip("/")
        self.jar = CookieJar()
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(self.jar),
            _NoRedirect(),
        )

    def request(
        self,
        method: str,
        path: str,
        headers: dict[str, str] | None = None,
        body: bytes | None = None,
    ) -> tuple[int, dict[str, str], bytes]:
        url = self.base + path
        req = urllib.request.Request(url, data=body, method=method)
        req.add_header("Accept", "text/html,application/json")
        if headers:
            for key, value in headers.items():
                req.add_header(key, value)
        try:
            with self.opener.open(req, timeout=30) as response:
                status = response.status
                resp_headers = {k.lower(): v for k, v in response.headers.items()}
                payload = response.read()
        except urllib.error.HTTPError as error:
            status = error.code
            resp_headers = {k.lower(): v for k, v in error.headers.items()}
            payload = error.read()
        return status, resp_headers, payload

    def cookie(self, name: str) -> str | None:
        for item in self.jar:
            if item.name == name:
                return item.value
        return None

    def login(self, email: str, password: str, origin: str) -> str:
        status, _, page = self.request("GET", "/admin/login")
        if status != 200:
            raise SystemExit(f"login form status {status}")
        token = self.cookie("fexu_login_csrf")
        if not token:
            raise SystemExit("missing fexu_login_csrf cookie")
        body = urllib.parse.urlencode(
            {
                "csrf_token": token,
                "email": email,
                "password": password,
                "next": "/admin",
            }
        ).encode()
        status, headers, _ = self.request(
            "POST",
            "/admin/login",
            headers={
                "Content-Type": "application/x-www-form-urlencoded",
                "Origin": origin,
                "Referer": f"{origin}/admin/login",
            },
            body=body,
        )
        if status not in (303, 302):
            raise SystemExit(f"login rejected with status {status}")
        session = self.cookie("fexu_session")
        if not session:
            set_cookie = headers.get("set-cookie", "")
            match = re.search(r"fexu_session=([^;]+)", set_cookie)
            session = match.group(1) if match else None
        if not session:
            raise SystemExit("login succeeded but fexu_session missing")
        return session

    def admin_get(self, path: str, session: str) -> int:
        status, headers, body = self.request(
            "GET",
            path,
            headers={"Cookie": f"fexu_session={session}"},
        )
        if "password" in body.decode("utf-8", errors="replace").lower() and path != "/admin/login":
            pass
        if "set-cookie" in headers and "fexu_session=;" in headers.get("set-cookie", ""):
            raise SystemExit(f"session cleared on {path}")
        return status


def run_admin_http_matrix(base: str, email: str, password: str, wrong: str) -> dict[str, Any]:
    origin = base
    client = HttpClient(base)
    session = client.login(email, password, origin)
    checks: dict[str, Any] = {"session_before_restart": session}

    for path in ("/admin",):
        status = client.admin_get(path, session)
        if status not in (200, 303, 302):
            raise SystemExit(f"authenticated path {path} returned {status}")

    bad = HttpClient(base)
    status, _, _ = bad.request("GET", "/admin")
    if status not in (302, 303, 401, 403):
        raise SystemExit(f"anonymous admin access unexpected status {status}")

    reject = HttpClient(base)
    reject.request("GET", "/admin/login")
    token = reject.cookie("fexu_login_csrf")
    if not token:
        raise SystemExit("csrf missing for bad login")
    body = urllib.parse.urlencode(
        {
            "csrf_token": token,
            "email": email,
            "password": wrong,
            "next": "/admin",
        }
    ).encode()
    status, headers, _ = reject.request(
        "POST",
        "/admin/login",
        headers={
            "Content-Type": "application/x-www-form-urlencoded",
            "Origin": origin,
            "Referer": f"{origin}/admin/login",
        },
        body=body,
    )
    if status not in (403, 401, 200):
        raise SystemExit(f"bad password status {status}")
    if reject.cookie("fexu_session"):
        raise SystemExit("bad password received session cookie")

    checks["bad_password"] = status
    checks["csrf_present"] = bool(token)
    return checks


def start_server(project: Path, port: int, database_url: str, csrf_secret: str) -> subprocess.Popen[Any]:
    env = os.environ.copy()
    env["DATABASE_URL"] = database_url
    env["FEXU_CSRF_SECRET"] = csrf_secret
    manifest = project / "fexu.toml"
    text = manifest.read_text(encoding="utf-8")
    manifest.write_text(
        text.replace("port = 3000", f"port = {port}"),
        encoding="utf-8",
    )
    process = subprocess.Popen(
        ["cargo", "run", "--quiet"],
        cwd=project,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    base = f"http://127.0.0.1:{port}"
    for _ in range(120):
        if process.poll() is not None:
            output = process.stdout.read() if process.stdout else ""
            raise SystemExit(f"server exited early:\n{output}")
        try:
            with urllib.request.urlopen(base + "/admin/login", timeout=2) as response:
                if response.status == 200:
                    return process
        except Exception:
            time.sleep(0.5)
    process.terminate()
    raise SystemExit("server did not answer /admin/login")


def stop_server(process: subprocess.Popen[Any]) -> None:
    if process.poll() is not None:
        return
    if os.name == "nt":
        process.terminate()
    else:
        process.send_signal(signal.SIGTERM)
    try:
        process.wait(timeout=20)
    except subprocess.TimeoutExpired:
        process.kill()


def seed_catalog_rows(db_path: Path, author_id: str, book_id: str) -> None:
    with sqlite3.connect(db_path) as conn:
        conn.execute(
            'INSERT INTO "authors" ("id", "name") VALUES (?, ?)',
            (author_id, "Ada Author"),
        )
        conn.execute(
            'INSERT INTO "books" ("id", "name", "author_id") VALUES (?, ?, ?)',
            (book_id, "Published V012 Book", author_id),
        )
        conn.commit()
        assert conn.execute('SELECT COUNT(*) FROM "authors"').fetchone()[0] == 1
        assert conn.execute('SELECT COUNT(*) FROM "books"').fetchone()[0] == 1


def seed_catalog_rows_pg(database_url: str, author_id: str, book_id: str) -> None:
    require_psql()
    psql = require_psql()
    statements = [
        f"INSERT INTO \"authors\" (\"id\", \"name\") VALUES ('{author_id}', 'Ada Author');",
        (
            f"INSERT INTO \"books\" (\"id\", \"name\", \"author_id\") "
            f"VALUES ('{book_id}', 'Published V012 Book', '{author_id}');"
        ),
    ]
    for sql in statements:
        subprocess.run(
            [psql, database_url, "-v", "ON_ERROR_STOP=1", "-c", sql],
            check=True,
            capture_output=True,
            text=True,
        )
    if psql_scalar(database_url, 'SELECT COUNT(*) FROM "authors"') != "1":
        raise SystemExit("authors seed count mismatch")
    if psql_scalar(database_url, 'SELECT COUNT(*) FROM "books"') != "1":
        raise SystemExit("books seed count mismatch")


def table_exists(database_url: str, table: str) -> bool:
    if not re.fullmatch(r"[a-z_]+", table):
        raise SystemExit(f"unexpected table name: {table}")
    return (
        psql_scalar(
            database_url,
            f"SELECT COUNT(*) FROM pg_tables WHERE schemaname = 'public' AND tablename = '{table}'",
        )
        == "1"
    )


def assert_post_upgrade_catalog(database_url: str, staff_email: str, role_name: str) -> None:
    snap = postgres_snapshot(database_url)
    if not snap.session_tokens:
        raise SystemExit("missing persisted session rows")
    if len(snap.password_hashes) < 2:
        raise SystemExit("missing seeded user password hashes")
    if not table_exists(database_url, "shelfs"):
        raise SystemExit("shelfs table missing after upgrade migration")
    assignment = (staff_email, role_name)
    if assignment not in snap.role_assignments:
        raise SystemExit(f"missing role assignment {assignment}: {snap.role_assignments}")
    book_author = psql_scalar(
        database_url,
        'SELECT COUNT(*) FROM "books" b JOIN "authors" a ON b."author_id" = a."id"',
    )
    if book_author != "1":
        raise SystemExit("catalog author/book relation broken after upgrade")


def _author_count(db_path: Path | None, database_url: str | None) -> int:
    if database_url:
        return int(psql_scalar(database_url, 'SELECT COUNT(*) FROM "authors"'))
    assert db_path is not None
    return int(sqlite3.connect(db_path).execute('SELECT COUNT(*) FROM "authors"').fetchone()[0])


def _migration_history_count(db_path: Path | None, database_url: str | None) -> int:
    if database_url:
        return int(psql_scalar(database_url, 'SELECT COUNT(*) FROM "_fexu_migrations"'))
    assert db_path is not None
    return int(
        sqlite3.connect(db_path).execute('SELECT COUNT(*) FROM "_fexu_migrations"').fetchone()[0]
    )


def assert_failed_migration_preserves_data(
    project: Path,
    cli: Path,
    db_path: Path | None,
    env: dict[str, str],
    database_url: str | None = None,
) -> None:
    migrations = sorted((project / "apps/catalog/migrations").glob("*.json"))
    if not migrations:
        raise SystemExit("missing catalog migrations")
    template = json.loads(migrations[-1].read_text(encoding="utf-8"))
    template["id"] = "catalog:9999_broken"
    template["name"] = "broken"
    tables = template.get("to_snapshot", {}).get("tables", [])
    if not tables:
        raise SystemExit("cannot build broken migration fixture")
    tables[0]["name"] = "authors"
    tables.append({"name": "authors", "columns": tables[0].get("columns", [])})
    broken = project / "apps/catalog/migrations/9999_broken.json"
    broken.write_text(json.dumps(template), encoding="utf-8")
    url = database_url or env.get("DATABASE_URL")
    pg_url = url if url and is_postgres_database_url(url) else None
    before_authors = _author_count(db_path, pg_url)
    before_history = _migration_history_count(db_path, pg_url)
    result = subprocess.run(
        [str(cli), "migrate", "up"],
        cwd=project,
        env=env,
        capture_output=True,
        text=True,
    )
    if result.returncode == 0:
        raise SystemExit("expected failed migration to be rejected")
    after_authors = _author_count(db_path, pg_url)
    after_history = _migration_history_count(db_path, pg_url)
    if before_authors != after_authors:
        raise SystemExit("failed migration changed author row count")
    if after_history != before_history:
        raise SystemExit("failed migration wrote history")
    broken.unlink(missing_ok=True)


if __name__ == "__main__":
    print("library module — import from upgrade scripts", file=sys.stderr)
    sys.exit(2)