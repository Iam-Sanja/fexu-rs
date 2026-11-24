#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib
import json
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

    p = sub.add_parser("rewrite")
    p.add_argument("manifest")
    p.add_argument("fexu_crate")
    p.add_argument("project_mod")
    p.add_argument("--baseline", choices=("0.1.2", "0.2.0"), default="0.1.2")

    p = sub.add_parser("repair-v020-resolver")
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
    p.add_argument("email")

    p = sub.add_parser("post-upgrade-db")
    p.add_argument("db")

    p_pg = sub.add_parser("post-upgrade-pg")
    p_pg.add_argument("database_url")
    p_pg.add_argument("staff_email")
    p_pg.add_argument("role_name")

    sub.add_parser("require-psql")

    p_patch_db = sub.add_parser("patch-database")
    p_patch_db.add_argument("fexu_toml")
    p_patch_db.add_argument("database_url")

    p_seed_pg = sub.add_parser("seed-pg")
    p_seed_pg.add_argument("database_url")
    p_seed_pg.add_argument("author_id")
    p_seed_pg.add_argument("book_id")

    p_snap_count = sub.add_parser("snap-count")
    p_snap_count.add_argument("db", nargs="?")
    p_snap_count.add_argument("--database-url", dest="database_url", default=None)

    p_snap_save = sub.add_parser("snap-save")
    p_snap_save.add_argument("path")
    p_snap_save.add_argument("db", nargs="?")
    p_snap_save.add_argument("--database-url", dest="database_url", default=None)

    p_snap_assert = sub.add_parser("snap-assert")
    p_snap_assert.add_argument("path")
    p_snap_assert.add_argument("db", nargs="?")
    p_snap_assert.add_argument("--database-url", dest="database_url", default=None)

    p = sub.add_parser("failed-migration")
    p.add_argument("project")
    p.add_argument("cli")
    p.add_argument("db", nargs="?")
    p.add_argument("--database-url", dest="database_url")

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
        snap = lib.snapshot_from_backend(
            Path(args.db) if args.db else None,
            args.database_url,
        )
        print(len(snap.migration_rows), len(snap.password_hashes))
        return 0
    if args.cmd == "repair-v020-resolver":
        lib.repair_v020_database_url_resolver(Path(args.project_mod))
        return 0
    if args.cmd == "rewrite":
        lib.rewrite_fullstack_dependency(Path(args.manifest), Path(args.fexu_crate), args.baseline)
        if args.baseline == "0.1.2":
            lib.patch_database_url_resolver(Path(args.project_mod))
        elif "let database_url = settings.resolved_database_url()?;" not in Path(args.project_mod).read_text(encoding="utf-8"):
            raise SystemExit("0.2.0 baseline resolver was not repaired before upgrade")
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
        # /admin can redirect even for authenticated users. Prove the exact
        # identity using a fresh client and the original cookie only.
        status, _, body = client.request(
            "GET", "/auth/me", headers={"Cookie": f"fexu_session={args.session}"}
        )
        if status != 200:
            raise SystemExit(f"original session authentication failed: HTTP {status}")
        user = json.loads(body)
        if user.get("email") != args.email or not all(
            user.get(flag) is True for flag in ("is_active", "is_staff", "is_superuser")
        ):
            raise SystemExit("original session returned the wrong identity or privileges")
        invalid = lib.HttpClient(args.base)
        status, _, _ = invalid.request(
            "GET", "/auth/me", headers={"Cookie": "fexu_session=invalid-regression-cookie"}
        )
        if status not in (401, 403):
            raise SystemExit(f"invalid session was not rejected: HTTP {status}")
        print("PASS: original session identity and privileges verified; invalid cookie rejected")
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
    if args.cmd == "post-upgrade-pg":
        lib.assert_post_upgrade_catalog(args.database_url, args.staff_email, args.role_name)
        return 0
    if args.cmd == "require-psql":
        lib.require_psql()
        return 0
    if args.cmd == "patch-database":
        lib.patch_fexu_database_url(Path(args.fexu_toml), args.database_url)
        return 0
    if args.cmd == "seed-pg":
        lib.seed_catalog_rows_pg(args.database_url, args.author_id, args.book_id)
        return 0
    if args.cmd == "snap-save":
        snap = lib.snapshot_from_backend(
            Path(args.db) if args.db else None,
            args.database_url,
        )
        lib.write_snapshot(Path(args.path), snap)
        return 0
    if args.cmd == "snap-assert":
        before = lib.load_snapshot(Path(args.path))
        after = lib.snapshot_from_backend(
            Path(args.db) if args.db else None,
            args.database_url,
        )
        lib.assert_snapshots_unchanged(before, after)
        return 0
    if args.cmd == "failed-migration":
        import os

        env = os.environ.copy()
        db_path = Path(args.db) if args.db else None
        database_url = args.database_url
        if database_url:
            env["DATABASE_URL"] = database_url
        elif db_path is not None:
            env["DATABASE_URL"] = "sqlite://db.sqlite3?mode=rwc"
        else:
            raise SystemExit("failed-migration requires db path or --database-url")
        lib.assert_failed_migration_preserves_data(
            Path(args.project),
            Path(args.cli),
            db_path,
            env,
            database_url=database_url,
        )
        return 0
    raise SystemExit(f"unknown command {args.cmd}")


if __name__ == "__main__":
    raise SystemExit(main())