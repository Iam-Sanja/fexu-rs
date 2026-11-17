#!/bin/sh
# Creates the runtime role used by the example app. The owner role stays
# available for migrations. This password is a local default, not a secret.
set -eu

app_password="${FEXU_APP_PASSWORD:-fexu_app_dev_only}"
db_name="${POSTGRES_DB:?POSTGRES_DB is required}"
owner="${POSTGRES_USER:?POSTGRES_USER is required}"

case "$app_password" in
  *[!A-Za-z0-9_]*)
    echo "FEXU_APP_PASSWORD must contain only letters, digits, and underscores." >&2
    exit 1
    ;;
esac
case "$db_name" in
  *[!A-Za-z0-9_]*)
    echo "POSTGRES_DB must contain only letters, digits, and underscores." >&2
    exit 1
    ;;
esac
case "$owner" in
  *[!A-Za-z0-9_]*)
    echo "POSTGRES_USER must contain only letters, digits, and underscores." >&2
    exit 1
    ;;
esac

psql -v ON_ERROR_STOP=1 --username "$owner" --dbname "$db_name" <<SQL
DO \$\$
BEGIN
  IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'fexu_app') THEN
    CREATE ROLE fexu_app LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION
      PASSWORD '${app_password}';
  END IF;
END
\$\$;
GRANT CONNECT ON DATABASE ${db_name} TO fexu_app;
GRANT USAGE ON SCHEMA public TO fexu_app;
ALTER DEFAULT PRIVILEGES FOR ROLE ${owner} IN SCHEMA public
  GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO fexu_app;
ALTER DEFAULT PRIVILEGES FOR ROLE ${owner} IN SCHEMA public
  GRANT USAGE, SELECT ON SEQUENCES TO fexu_app;
SQL
