#!/usr/bin/env bash
# Real published v0.1.2 producer and development source consumer.
set -euo pipefail
repo="$(cd "$(dirname "\${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
chmod 700 "$work"
export CARGO_TARGET_DIR="$repo/target"
export CARGO_BUILD_JOBS="\${CARGO_BUILD_JOBS:-2}"
export DATABASE_URL="sqlite://$work/legacy.sqlite3?mode=rwc"
export FEXU_TEST_FIXTURE="$work/fixture"
export FEXU_TEST_PASSWORD="$(python3 -c 'import secrets; print("V012pass!" + secrets.token_hex(20))')"
mkdir -p "$work/old/src" "$work/new/src"
cat > "$work/old/Cargo.toml" <<'TOML'
[package]
name = "published-auth-upgrade-consumer"
version = "0.1.0"
edition = "2021"
[dependencies]
fexu-auth = "=0.1.2"
fexu-db = "=0.1.2"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
TOML
python3 - "$work/new/Cargo.toml" "$repo" <<'PY'
from pathlib import Path
import json
import sys
root = Path(sys.argv[2])
auth = json.dumps((root / "crates/fexu-auth").as_posix())
db = json.dumps((root / "crates/fexu-db").as_posix())
Path(sys.argv[1]).write_text(f'''[package]
name = "development-auth-upgrade-consumer"
version = "0.1.0"
edition = "2021"
[dependencies]
fexu-auth = {{ path = {auth} }}
fexu-db = {{ path = {db} }}
tokio = {{ version = "1", features = ["rt-multi-thread", "macros"] }}
''')
PY
cat > "$work/old/src/main.rs" <<'RS'
use fexu_auth::{apply_migrations, AuthConfig, AuthService};
use std::error::Error;
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let db = fexu_db::connect(&std::env::var("DATABASE_URL")?).await?;
    assert_eq!(apply_migrations(&db).await?.len(), 6);
    let auth = AuthService::new(db.clone(), AuthConfig::development());
    let password = std::env::var("FEXU_TEST_PASSWORD")?;
    let email = "existing-admin@example.test";
    let user = auth.create_user(email, &password, true, true).await?;
    auth.grant_permission("release-operator", "reports.read").await?;
    auth.assign_role(email, "release-operator").await?;
    let session = auth.login(email, &password, "published-v012").await?;
    let authenticated = auth.authenticate_token(&session.token).await?;
    assert_eq!(authenticated.id, user.id);
    auth.authorize(&authenticated, Some("reports"), "read").await?;
    std::fs::write(std::env::var("FEXU_TEST_FIXTURE")?,
        format!("{}\n{}\n{}\n", user.id, authenticated.session_id, session.token))?;
    drop(auth);
    db.close().await?;
    println!("PASS: published 0.1.2 persisted historical account and session");
    Ok(())
}
RS
cat > "$work/new/src/main.rs" <<'RS'
use fexu_auth::{apply_migrations, AuthConfig, AuthError, AuthService};
use std::error::Error;
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let fixture = std::fs::read_to_string(std::env::var("FEXU_TEST_FIXTURE")?)?;
    let mut lines = fixture.lines();
    let user_id = lines.next().ok_or("missing user")?;
    let session_id = lines.next().ok_or("missing session")?;
    let token = lines.next().ok_or("missing token")?;
    assert!(lines.next().is_none(), "unexpected fixture content");
    let db = fexu_db::connect(&std::env::var("DATABASE_URL")?).await?;
    assert!(apply_migrations(&db).await?.is_empty(), "reapplied historical auth migrations");
    let auth = AuthService::new(db.clone(), AuthConfig::development());
    let restored = auth.authenticate_token(token).await?;
    assert_eq!(restored.id.to_string(), user_id);
    assert_eq!(restored.session_id.to_string(), session_id);
    assert!(restored.is_active && restored.is_staff && restored.is_superuser);
    auth.authorize(&restored, Some("reports"), "read").await?;
    let email = "existing-admin@example.test";
    assert!(matches!(auth.login(email, "bad-credentials", "reject").await,
        Err(AuthError::InvalidCredentials)));
    let fresh = auth.login(email, &std::env::var("FEXU_TEST_PASSWORD")?, "accept").await?;
    assert_eq!(fresh.user.id.to_string(), user_id);
    assert_ne!(fresh.token, token);
    assert!(auth.list_users().await?.iter().any(|u| u.id.to_string() == user_id));
    drop(auth);
    db.close().await?;
    println!("PASS: source consumer preserved historical admin, password and session");
    Ok(())
}
RS
cargo run --quiet --manifest-path "$work/old/Cargo.toml"
chmod 600 "$FEXU_TEST_FIXTURE"
cargo run --quiet --manifest-path "$work/new/Cargo.toml"
python3 - "$work/legacy.sqlite3" <<'PY'
import sqlite3
import sys
with sqlite3.connect(sys.argv[1]) as db:
    assert db.execute("PRAGMA quick_check").fetchone()[0] == "ok"
PY
echo "PASS: published 0.1.2 -> development auth binary upgrade on isolated SQLite"
