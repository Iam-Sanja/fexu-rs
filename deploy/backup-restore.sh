#!/bin/sh
# Backup/restore rehearsal only; never use this script against production.
set -eu
exec python3 - "$@" <<'PY'
import os
import re
import subprocess
import sys
import tempfile
from urllib.parse import urlsplit

def database_name(url):
    # Restrict libpq to explicit loopback TCP endpoints and simple disposable
    # database names. Reject connection-string options, encoded host/name,
    # multi-host lists, socket paths and ambiguous authority.
    if any(ch.isspace() or ord(ch) < 32 or ord(ch) == 127 for ch in url):
        raise ValueError
    if "?" in url or "#" in url or url.count("@") != 1:
        raise ValueError
    parsed = urlsplit(url)
    if parsed.scheme not in ("postgres", "postgresql"):
        raise ValueError
    authority = parsed.netloc.rsplit("@", 1)[1]
    if not re.fullmatch(r"(?:127\.0\.0\.1|\[::1\]):[1-9][0-9]{0,4}", authority):
        raise ValueError
    if not parsed.username or parsed.port is None or not 1 <= parsed.port <= 65535:
        raise ValueError
    if not re.fullmatch(r"/fexu_test_[A-Za-z0-9_]+", parsed.path) or len(parsed.path[1:]) > 63:
        raise ValueError
    # Refuse malformed escapes and NUL in user info as well.
    userinfo = parsed.netloc.rsplit("@", 1)[0]
    if re.search(r"%(?![0-9A-Fa-f]{2})|%00", userinfo):
        raise ValueError
    return parsed.path[1:]

if len(sys.argv) != 3:
    sys.exit("usage: backup-restore.sh SOURCE_URL TARGET_URL")
source, target = sys.argv[1:]
try:
    source_name, target_name = database_name(source), database_name(target)
    if source_name == target_name:
        raise ValueError
except ValueError:
    sys.exit("refusing: use distinct fexu_test_* databases with explicit loopback IP and port; no query or fragment")

# libpq environment variables (including PGHOSTADDR and PGSERVICE) must not
# redirect either validated URI. Supply connection credentials in the URI
# or use a local password file; never print either URL.
env = {key: value for key, value in os.environ.items() if not key.upper().startswith("PG")}
with tempfile.TemporaryDirectory(prefix="fexu-backup-") as work:
    dump = os.path.join(work, "backup.dump")
    try:
        subprocess.run(["pg_dump", "--no-password", "--format=custom", "--file", dump, "--dbname", source], env=env, check=True)
        subprocess.run(["pg_restore", "--no-password", "--single-transaction", "--exit-on-error", "--no-owner", "--no-privileges", "--dbname", target, dump], env=env, check=True)
    except subprocess.CalledProcessError:
        sys.exit("backup/restore rehearsal failed")
print("restore finished")
PY
