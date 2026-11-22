#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib
import sqlite3
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))
lib = importlib.import_module("v012_fullstack_upgrade_lib")


def main() -> int:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="cmd", required=True)

    sub.add_parser("pick-port")

    p = sub.add_parser("patch-book")
    p.add_argument("book_model")

    p = sub.add_parser("seed")
    p.add_argument("db")
    p.add_argument("author_id")
    p.add_argument("book_id")

    p = sub.add_parser("csrf")
    p.add_argument("project")

    p = sub.add_parser("snap-count")
    p.add_argument("db")

    p = sub.add_parser("rewrite")
    p.add_argument("manifest")
    p.add_argument("fexu_crate")
    p.add_argument("project_mod")

    p = sub.add_parser("login")
    p.add_argument("base")
    p.add_argument("email")
    p.add_argument("password")

    p = sub.add_parser("http-matrix")
    p.add_argument("base")
    p.add_argument("email")
    p.add_argument("password")
    p.add_argument("wrong")

    p = sub.add_parser("session-check")
    p.add_argument("base")
    p.add_argument("session")

    p = sub.add_parser("post-upgrade-db")
    p.add_argument("db")

    p = sub.add_parser("failed-migration")
    p.add_argument("project")
    p.add_argument("cli")
    p.add_argument("db")

    args = parser.parse_args()
    if args.cmd == "pick-port":
        import socket

        sock = socket.socket()
        sock.bind(("127.0.0.1", 0))
        print(sock.getsockname()[1])
        sock.close()
        return 0
    if args.cmd == "patch-book":
        lib.add_book_author_relation(Path(args.book_model))
        return 0
    if args.cmd == "seed":
        lib.seed_catalog_rows(Path(args.db), args.author_id, args.book_id)
        return 0
    if args.cmd == "csrf":
        print(lib.ensure_csrf_key(Path(args.project)))
        return 0
    if args.cmd == "snap-count":
        snap = lib.sqlite_snapshot(Path(args.db))
        print(len(snap.migration_rows), len(snap.password_hashes))
        return 0
    if args.cmd == "rewrite":
        lib.rewrite_fullstack_dependency(Path(args.manifest), Path(args.fexu_crate))
        lib.patch_database_url_resolver(Path(args.project_mod))
        return 0
    if args.cmd == "login":
        client = lib.HttpClient(args.base)
        print(client.login(args.email, args.password, args.base))
        return 0
    if args.cmd == "http-matrix":
        lib.run_admin_http_matrix(args.base, args.email, args.password, args.wrong)
        return 0
    if args.cmd == "session-check":
        client = lib.HttpClient(args.base)
        status = client.admin_get("/admin", args.session)
        if status not in (200, 302, 303):
            raise SystemExit(f"old session invalid after upgrade: {status}")
        return 0
    if args.cmd == "post-upgrade-db":
        snap = lib.sqlite_snapshot(Path(args.db))
        if not snap.session_tokens:
            raise SystemExit("missing persisted session rows")
        if len(snap.password_hashes) < 2:
            raise SystemExit("missing seeded user password hashes")
        with sqlite3.connect(args.db) as conn:
            tables = {row[0] for row in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        if "shelfs" not in tables:
            raise SystemExit(f"shelfs table missing: {tables}")
        return 0
    if args.cmd == "failed-migration":
        import os

        env = os.environ.copy()
        env["DATABASE_URL"] = "sqlite://db.sqlite3?mode=rwc"
        lib.assert_failed_migration_preserves_data(Path(args.project), Path(args.cli), Path(args.db), env)
        return 0
    raise SystemExit(f"unknown command {args.cmd}")


if __name__ == "__main__":
    raise SystemExit(main())