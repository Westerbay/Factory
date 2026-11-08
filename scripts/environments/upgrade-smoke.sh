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
session=$(node -e 'console.log(require("node:crypto").randomBytes(32).toString("base64url"))')
token_hash=$(printf '%s' "$session" | sha256sum | cut -d ' ' -f1)
csrf=$(node -e 'console.log(require("node:crypto").randomBytes(32).toString("base64url"))')
user_id=$(node -e 'console.log(require("node:crypto").randomUUID())')
run_release previous compose exec -T postgres psql -X -U postgres -d factory -v ON_ERROR_STOP=1 \
  -v "user_id=$user_id" -v "token_hash=$token_hash" -v "csrf=$csrf" <<'SQL'
INSERT INTO factory_identity.users (id, display_name, role, status, access_enabled_at)
VALUES (:'user_id', 'Upgrade Fixture', 'member', 'active', clock_timestamp());
INSERT INTO factory_identity.external_identities (user_id, provider, issuer, subject, handle)
VALUES (:'user_id', 'github', 'https://github.com', '1', 'upgrade-fixture');
INSERT INTO factory_identity.sessions (token_hash, user_id, csrf_token, expires_at)
VALUES (decode(:'token_hash', 'hex'), :'user_id', :'csrf', clock_timestamp() + interval '30 minutes');
SQL

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
wait_for_workspace not_created
request -X POST "$origin/api/environment/initialize" >/dev/null
wait_for_workspace running
state="$fixture/state/.factory/environments"
installation=$(jq -er .installationId "$state/installation.json")
workspace=$("${docker[@]}" container ls --quiet --no-trunc \
  --filter "label=io.factory.installation=$installation" --filter "label=io.factory.user=$user_id")
[[ "$workspace" =~ ^[0-9a-f]{64}$ ]]
home_volume() {
  "${docker[@]}" inspect --format '{{range .Mounts}}{{if eq .Destination "/home/factory"}}{{.Name}}{{end}}{{end}}' "$workspace"
}
volume=$(home_volume)
[[ -n "$volume" ]]
"${docker[@]}" exec "$workspace" sh -ec 'printf factory-upgrade-witness > /home/factory/Documents/upgrade-witness'
api_started() {
  "${docker[@]}" inspect --format '{{.Id}} {{.State.StartedAt}}' factory-prod-api-1
}
old_api=$(api_started)
old_state=$(sha256sum "$state/prepared.json" "$state/egress.json")
refuse_update() {
  local reason=$1
  if run_release candidate deploy > "$fixture/refusal.log" 2>&1; then
    echo 'Unexpectedly deployed an incompatible candidate.' >&2
    return 1
  fi
  if ! grep -Fq "$reason" "$fixture/refusal.log"; then
    cat "$fixture/refusal.log" >&2
    return 1
  fi
  request "$origin/health/ready" >/dev/null
  [[ "$(api_started)" == "$old_api" ]]
  [[ "$(sha256sum "$state/prepared.json" "$state/egress.json")" == "$old_state" ]]
}

refuse_update 'Factory workspaces are active'
wait_for_workspace running
request -X POST "$origin/api/environment/stop" >/dev/null
wait_for_workspace stopped

# Test the real candidate refusal when a shared runtime image changed. The
# subsequent compatible-path test reuses the baseline runtime with the new API.
image_ref() { sed -n "s/^$2=//p" "$fixture/$1/release.env"; }
workspace_ref=$(image_ref previous FACTORY_WORKSPACE_IMAGE)
egress_ref=$(image_ref previous FACTORY_EGRESS_IMAGE)
image_id() { "${docker[@]}" image inspect --format '{{.Id}}' "$1"; }
candidate_workspace_id=$(image_id "$(image_ref candidate FACTORY_WORKSPACE_IMAGE)")
previous_workspace_id=$(image_id "$workspace_ref")
candidate_egress_id=$(image_id "$(image_ref candidate FACTORY_EGRESS_IMAGE)")
previous_egress_id=$(image_id "$egress_ref")
if [[ "$candidate_workspace_id" != "$previous_workspace_id" ]]; then
  refuse_update 'changes the prepared workspace image or policy'
elif [[ "$candidate_egress_id" != "$previous_egress_id" ]]; then
  refuse_update 'incompatible with this preparation'
fi
candidate_api_ref=$(image_ref candidate FACTORY_API_IMAGE)
printf 'FACTORY_API_IMAGE=%s\nFACTORY_WORKSPACE_IMAGE=%s\nFACTORY_EGRESS_IMAGE=%s\n' \
  "$candidate_api_ref" "$workspace_ref" "$egress_ref" > "$fixture/candidate/runtime.env"
mv -T "$fixture/candidate/runtime.env" "$fixture/candidate/release.env"
configure_tls candidate
run_release candidate deploy
candidate_api_id=$(image_id "$candidate_api_ref")
[[ "$("${docker[@]}" inspect --format '{{.Image}}' factory-prod-api-1)" == "$candidate_api_id" ]]
request "$origin/" | cmp - "$fixture/candidate/web-dist/index.html"
wait_for_workspace stopped
request -X POST "$origin/api/environment/start" >/dev/null
wait_for_workspace running
[[ "$(home_volume)" == "$volume" ]]
"${docker[@]}" exec "$workspace" sh -ec 'test "$(cat /home/factory/Documents/upgrade-witness)" = factory-upgrade-witness'
printf 'Upgrade preserved the existing workspace, home volume and session.\n'
