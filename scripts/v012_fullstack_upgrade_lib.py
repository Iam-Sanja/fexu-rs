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
    return AuthSnapshot(hashes, sessions, migrations, users)


def assert_snapshots_unchanged(before: AuthSnapshot, after: AuthSnapshot) -> None:
    if before.password_hashes != after.password_hashes:
        raise SystemExit("password hashes changed across upgrade")
    before_hist = [row for row in before.migration_rows if not row[0].startswith("catalog:add_")]
    after_hist = [row for row in after.migration_rows if not row[0].startswith("catalog:add_")]
    if before_hist != after_hist:
        raise SystemExit("historical migration checksums changed")
    if before.user_rows != after.user_rows:
        raise SystemExit("user flags changed unexpectedly")


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


def assert_failed_migration_preserves_data(
    project: Path,
    cli: Path,
    db_path: Path,
    env: dict[str, str],
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
    before_authors = sqlite3.connect(db_path).execute('SELECT COUNT(*) FROM "authors"').fetchone()[0]
    before_history = sqlite3.connect(db_path).execute(
        'SELECT COUNT(*) FROM "_fexu_migrations"'
    ).fetchone()[0]
    result = subprocess.run(
        [str(cli), "migrate", "up"],
        cwd=project,
        env=env,
        capture_output=True,
        text=True,
    )
    if result.returncode == 0:
        raise SystemExit("expected failed migration to be rejected")
    after_authors = sqlite3.connect(db_path).execute('SELECT COUNT(*) FROM "authors"').fetchone()[0]
    after_history = sqlite3.connect(db_path).execute(
        'SELECT COUNT(*) FROM "_fexu_migrations"'
    ).fetchone()[0]
    if before_authors != after_authors:
        raise SystemExit("failed migration changed author row count")
    if after_history != before_history:
        raise SystemExit("failed migration wrote history")
    broken.unlink(missing_ok=True)


if __name__ == "__main__":
    print("library module — import from upgrade scripts", file=sys.stderr)
    sys.exit(2)