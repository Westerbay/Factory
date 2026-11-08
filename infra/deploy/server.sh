#!/usr/bin/env bash
set -euo pipefail
umask 077

if [[ $# -lt 2 || ${1:-} != /* ]]; then
  echo "Usage: $0 <absolute installation directory> deploy|compose [compose arguments]" >&2
  exit 2
fi

installation=$(cd "$1" && pwd -P)
command=$2
shift 2
bundle=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
export FACTORY_STATE_DIR="$installation/state"
export FACTORY_BOOTSTRAP_DIR="$installation/bootstrap"

# Resolve the exact image references from the release, independently of the shell.
unset FACTORY_API_IMAGE FACTORY_WORKSPACE_IMAGE FACTORY_EGRESS_IMAGE
while IFS='=' read -r key value; do
  case "$key" in
    FACTORY_API_IMAGE|FACTORY_WORKSPACE_IMAGE|FACTORY_EGRESS_IMAGE) ;;
    *) echo "Unexpected entry in release.env" >&2; exit 2 ;;
  esac
  if [[ ! "$value" =~ ^ghcr\.io/[a-z0-9_./-]+@sha256:[0-9a-f]{64}$ || -n "${!key-}" ]]; then
    echo "Invalid or repeated image reference in release.env" >&2
    exit 2
  fi
  export "$key=$value"
done < "$bundle/release.env"
: "${FACTORY_API_IMAGE:?Missing release API image}"
: "${FACTORY_WORKSPACE_IMAGE:?Missing release workspace image}"
: "${FACTORY_EGRESS_IMAGE:?Missing release egress image}"

unset FACTORY_WEB_ORIGIN FACTORY_API_ORIGIN
unset FACTORY_GITHUB_CLIENT_ID FACTORY_GITHUB_CLIENT_SECRET
unset FACTORY_DB_PASSWORD FACTORY_POSTGRES_PASSWORD
compose=(docker --host unix:///var/run/docker.sock compose
  --project-name factory-prod --project-directory "$bundle/infra/deploy"
  --env-file "$installation/.env" --file "$bundle/infra/deploy/compose.yml")
if [[ -f "$installation/compose.override.yml" ]]; then
  compose+=(--file "$installation/compose.override.yml")
fi

case "$command" in
  compose)
    exec "${compose[@]}" "$@"
    ;;
  deploy)
    [[ $# -eq 0 ]] || exit 2
    ;;
  *)
    echo "Unknown command: $command" >&2
    exit 2
    ;;
esac

if [[ "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
  echo "Production bundles require a Linux x86-64 host" >&2
  exit 1
fi

exec 9>"$installation/deploy.lock"
flock -n 9 || { echo "Another Factory deployment is in progress" >&2; exit 1; }
for asset in index.html sw.js manifest.webmanifest; do
  if [[ ! -s "$bundle/web-dist/$asset" ]]; then
    echo "Missing web release file: $asset" >&2
    exit 1
  fi
done
mkdir -p "$FACTORY_STATE_DIR"

# Keep first-boot files at stable host paths, even when changing releases.
if [[ ! -d "$FACTORY_BOOTSTRAP_DIR" ]]; then
  staging=$(mktemp -d "$installation/bootstrap.XXXXXX")
  trap 'rm -f -- "$staging/init-postgres.sh" "$staging/schema.sql"; rmdir -- "$staging"' EXIT
  install -m 755 "$bundle/infra/deploy/init-postgres.sh" "$staging/init-postgres.sh"
  install -m 644 "$bundle/crates/database/schema.sql" "$staging/schema.sql"
  chmod 755 "$staging"
  mv -T -- "$staging" "$FACTORY_BOOTSTRAP_DIR"
  trap - EXIT
fi

"${compose[@]}" pull
docker --host unix:///var/run/docker.sock image pull "$FACTORY_WORKSPACE_IMAGE"
docker --host unix:///var/run/docker.sock image pull "$FACTORY_EGRESS_IMAGE"

"${compose[@]}" stop api
node "$bundle/scripts/environments/prepare.mjs" --directory "$FACTORY_STATE_DIR/.factory/environments" \
  --images "$FACTORY_WORKSPACE_IMAGE" "$FACTORY_EGRESS_IMAGE"
"${compose[@]}" up -d --pull never --wait --wait-timeout 120
printf 'Factory API is ready. Verify HTTPS and test sign-in.\n'
