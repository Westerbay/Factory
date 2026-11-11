#!/usr/bin/env bash
set -euo pipefail
umask 077

# The production script owns a fixed Compose project and local socket.
if [[ ${GITHUB_ACTIONS:-} != true || ${RUNNER_ENVIRONMENT:-} != github-hosted ||
      "$(uname -sm)" != 'Linux x86_64' || $EUID != 0 || $# -ne 2 ]]; then
  echo 'Run this smoke test as root on a disposable Linux x86-64 GitHub-hosted runner with two verified release bundles.' >&2
  exit 2
fi

docker=(docker --host unix:///var/run/docker.sock)
label=label=com.docker.compose.project=factory-prod
for resource in container volume network; do
  if [[ $resource == container ]]; then
    existing=$("${docker[@]}" container ls --all --quiet --filter "$label")
  else
    existing=$("${docker[@]}" "$resource" ls --quiet --filter "$label")
  fi
  [[ -z "$existing" ]] || { echo 'Refusing to reuse an existing Factory deployment.' >&2; exit 1; }
done
[[ -z "$("${docker[@]}" volume ls --quiet --filter 'name=^factory-prod_')" ]]

previous=$(realpath "$1")
candidate=$(realpath "$2")
fixture=$(mktemp -d "${RUNNER_TEMP:?}/factory-upgrade.XXXXXX")
origin=https://factory.upgrade.test
export COMPOSE_PROGRESS=plain
mkdir "$fixture/previous" "$fixture/candidate"
tar -xzf "$previous" -C "$fixture/previous"
tar -xzf "$candidate" -C "$fixture/candidate"

cat > "$fixture/.env" <<'ENV'
FACTORY_WEB_ORIGIN=https://factory.upgrade.test
FACTORY_GITHUB_CLIENT_ID=fixture-only
FACTORY_GITHUB_CLIENT_SECRET=fixture-only
FACTORY_POSTGRES_PASSWORD=fixture-postgres-only
FACTORY_DB_PASSWORD=fixture-api-only
ENV
cat > "$fixture/compose.override.yml" <<'COMPOSE'
services:
  caddy:
    ports: !override
      - "127.0.0.1:443:443"
    volumes:
      - ${FACTORY_BOOTSTRAP_DIR}/Caddyfile:/etc/caddy/Caddyfile:ro
COMPOSE

install -d -m 0755 "$fixture/bootstrap"
install -m 0755 "$fixture/previous/infra/deploy/init-postgres.sh" "$fixture/bootstrap/init-postgres.sh"
install -m 0644 "$fixture/previous/crates/database/schema.sql" "$fixture/bootstrap/schema.sql"
configure_tls() {
  { printf '{\n\tlocal_certs\n}\n\n'; cat "$fixture/$1/infra/deploy/Caddyfile"; } > "$fixture/bootstrap/Caddyfile"
  chmod 0644 "$fixture/bootstrap/Caddyfile"
}
run_release() {
  local version=$1
  shift
  "$fixture/$version/infra/deploy/server.sh" "$fixture" "$@"
}
configure_tls previous
run_release previous deploy

# Seed only the database created above by the disposable Compose installation.
seed_member() {
  local subject=$1 token_hash
  session=$(node -e 'console.log(require("node:crypto").randomBytes(32).toString("base64url"))')
  token_hash=$(printf '%s' "$session" | sha256sum | cut -d ' ' -f1)
  csrf=$(node -e 'console.log(require("node:crypto").randomBytes(32).toString("base64url"))')
  user_id=$(node -e 'console.log(require("node:crypto").randomUUID())')
  run_release previous compose exec -T postgres psql -X -U postgres -d factory -v ON_ERROR_STOP=1 \
    -v "user_id=$user_id" -v "token_hash=$token_hash" -v "csrf=$csrf" -v "subject=$subject" <<'SQL'
INSERT INTO factory_identity.users (id, display_name, role, status, access_enabled_at)
VALUES (:'user_id', 'Upgrade Fixture', 'member', 'active', clock_timestamp());
INSERT INTO factory_identity.external_identities (user_id, provider, issuer, subject, handle)
VALUES (:'user_id', 'github', 'https://github.com', :'subject', 'upgrade-fixture');
INSERT INTO factory_identity.sessions (token_hash, user_id, csrf_token, expires_at)
VALUES (decode(:'token_hash', 'hex'), :'user_id', :'csrf', clock_timestamp() + interval '30 minutes');
SQL
}

request() {
  curl --noproxy '*' --resolve 'factory.upgrade.test:443:127.0.0.1' --insecure \
    --fail --silent --show-error --max-time 8 --cookie "factory_session=$session" \
    -H "Origin: $origin" -H "X-CSRF-Token: $csrf" "$@"
}
wait_for_workspace() {
  local expected=$1 observed
  for attempt in $(seq 1 60); do
    if observed=$(request "$origin/api/environment"); then
      if [[ "$(jq -r .status <<< "$observed")" == "$expected" ]]; then
        return 0
      fi
    fi
    sleep 2
  done
  printf 'Expected workspace %s, last observation: %s\n' "$expected" "${observed:-missing}" >&2
  return 1
}
seed_member 2
request -X POST "$origin/api/environment/initialize" >/dev/null
wait_for_workspace running
request -X POST "$origin/api/environment/stop" >/dev/null
wait_for_workspace stopped
stopped_session=$session
stopped_csrf=$csrf

seed_member 1
wait_for_workspace not_created
request -X POST "$origin/api/environment/initialize" >/dev/null
wait_for_workspace running
state="$fixture/state/.factory/environments"
installation=$(jq -er .installationId "$state/installation.json")
running_workspace() {
  "${docker[@]}" container ls --quiet --no-trunc \
    --filter "label=io.factory.installation=$installation" --filter "label=io.factory.user=$user_id"
}
workspace=$(running_workspace)
[[ "$workspace" =~ ^[0-9a-f]{64}$ ]]
home_volume() {
  "${docker[@]}" inspect --format '{{range .Mounts}}{{if eq .Destination "/home/factory"}}{{.Name}}{{end}}{{end}}' "$workspace"
}
volume=$(home_volume)
[[ -n "$volume" ]]
"${docker[@]}" exec "$workspace" sh -ec 'printf factory-upgrade-witness > /home/factory/Documents/upgrade-witness'
image_ref() { sed -n "s/^$2=//p" "$fixture/$1/release.env"; }
image_id() { "${docker[@]}" image inspect --format '{{.Id}}' "$1"; }
previous_workspace=$workspace
previous_workspace_image=$("${docker[@]}" inspect --format '{{.Image}}' "$workspace")
configure_tls candidate
run_release candidate deploy
candidate_api_id=$(image_id "$(image_ref candidate FACTORY_API_IMAGE)")
[[ "$("${docker[@]}" inspect --format '{{.Image}}' factory-prod-api-1)" == "$candidate_api_id" ]]
request "$origin/" | cmp - "$fixture/candidate/web-dist/index.html"
wait_for_workspace running
workspace=$(running_workspace)
candidate_workspace_image=$(image_id "$(image_ref candidate FACTORY_WORKSPACE_IMAGE)")
[[ "$("${docker[@]}" inspect --format '{{.Image}}' "$workspace")" == "$candidate_workspace_image" ]]
if [[ "$previous_workspace_image" == "$candidate_workspace_image" ]]; then
  [[ "$workspace" == "$previous_workspace" ]]
else
  [[ "$workspace" != "$previous_workspace" ]]
fi
[[ "$(home_volume)" == "$volume" ]]
"${docker[@]}" exec "$workspace" sh -ec 'test "$(cat /home/factory/Documents/upgrade-witness)" = factory-upgrade-witness'
session=$stopped_session
csrf=$stopped_csrf
wait_for_workspace stopped
[[ ! -e "$state/upgrade" ]]
printf 'Upgrade restored the running workspace, preserved its home and session, and kept the stopped workspace stopped.\n'
