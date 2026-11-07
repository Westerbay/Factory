#!/bin/sh
set -eu

# Executed by the official image only during initialization of an empty volume.
# Keep the runtime account separate from the PostgreSQL administrator.
psql --username postgres --dbname factory --no-psqlrc --set ON_ERROR_STOP=1 --single-transaction --file - <<'SQL'
\getenv password FACTORY_DB_PASSWORD
CREATE ROLE factory LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD :'password';
GRANT CREATE ON DATABASE factory TO factory;
GRANT CREATE ON SCHEMA public TO factory;
SET ROLE factory;
\i /usr/local/share/factory/schema.sql
RESET ROLE;
REVOKE CREATE ON DATABASE factory FROM factory;
REVOKE CREATE ON SCHEMA public FROM factory;
SQL
