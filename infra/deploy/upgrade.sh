# Sourced by server.sh with the deployment lock held and the exact images local.
upgrade_command() {
  "${compose[@]}" run --rm --no-deps -T --entrypoint /usr/local/bin/factory api workspace upgrade "$1"
}

wait_for_upgrade_check() {
  local attempt status
  for ((attempt = 1; attempt <= 20; attempt++)); do
    if upgrade_command check; then
      return 0
    else
      status=$?
    fi
    if ((status != 75 || attempt == 20)); then
      return "$status"
    fi
    sleep 15
  done
}

remove_upgrade_journal() {
  local path=$1
  rm -f -- "$path/previous/"{prepared,egress}.json "$path/candidate/"{prepared,egress}.json \
    "$path/bundle" "$path/previous-bundle" "$path/mutating" "$path/applied"
  rmdir -- "$path/previous" "$path/candidate" "$path"
}

close_upgrade_journal() {
  local closed
  closed=$(mktemp -d "$workspace_state/.upgrade-finished.XXXXXX") &&
    mv -T -- "$journal" "$closed/journal" &&
    remove_upgrade_journal "$closed/journal" && rmdir -- "$closed"
}

activate_preparation() {
  local source=$1 metadata temporary
  for metadata in prepared.json egress.json; do
    temporary=$(mktemp "$workspace_state/.$metadata.XXXXXX") || return
    cp -- "$source/$metadata" "$temporary" && mv -T -- "$temporary" "$workspace_state/$metadata" || return
  done
}

create_upgrade_journal() {
  local web previous staging
  web=$("${engine[@]}" inspect --format '{{if and (eq (index .Config.Labels "com.docker.compose.project") "factory-prod") (eq (index .Config.Labels "com.docker.compose.service") "caddy")}}{{range .Mounts}}{{if eq .Destination "/srv/factory-web"}}{{.Source}}{{end}}{{end}}{{end}}' factory-prod-caddy-1)
  previous=${web%/web-dist}
  if [[ "$previous" != "$installation/"* || "$web" != "$previous/web-dist" ||
      ! -x "$previous/infra/deploy/server.sh" || ! -f "$previous/release.env" ||
      "$(realpath -- "$previous")" != "$previous" ]]; then
    echo 'Cannot locate the installed release for rollback.' >&2
    return 1
  fi
  staging=$(mktemp -d "$workspace_state/.upgrade.XXXXXX")
  mkdir -- "$staging/previous" "$staging/candidate"
  if ! cp -- "$workspace_state/"{prepared,egress}.json "$staging/previous/" ||
    ! "${prepare[@]}" --stage "$staging/candidate"; then
    remove_upgrade_journal "$staging"
    return 1
  fi
  printf '%s\n' "$bundle" > "$staging/bundle"
  printf '%s\n' "$previous" > "$staging/previous-bundle"
  mv -T -- "$staging" "$journal"
}

restore_previous_release() {
  rm -f -- "$journal/applied" || return
  if [[ -e "$journal/mutating" ]]; then
    "${compose[@]}" stop --timeout 120 api &&
      upgrade_command rollback && activate_preparation "$journal/previous" || return
  fi
  "$previous_bundle/infra/deploy/server.sh" "$installation" compose \
    up -d --pull never --wait --wait-timeout 120 && close_upgrade_journal
}

upgrade_exit() {
  local status=$?
  trap - EXIT
  if ((status != 0)); then
    if restore_previous_release; then
      echo 'The update failed and the previous release was restored.' >&2
    else
      echo 'Automatic rollback could not finish. Retry deployment of the same bundle to resume the pending upgrade.' >&2
    fi
  fi
  exit "$status"
}

upgrade_installation() {
  journal="$workspace_state/upgrade"
  if [[ ! -e "$journal" ]]; then
    create_upgrade_journal
  fi
  if [[ "$(< "$journal/bundle")" != "$bundle" ]]; then
    echo 'An update is pending. Retry deployment of the recorded bundle first.' >&2
    return 1
  fi
  previous_bundle=$(< "$journal/previous-bundle")
  if [[ ! -e "$journal/mutating" && ! -e "$journal/applied" ]] && ! wait_for_upgrade_check; then
    close_upgrade_journal
    return 1
  fi

  trap upgrade_exit EXIT
  if [[ ! -e "$journal/applied" ]]; then
    if ! cmp -s "$journal/previous/prepared.json" "$journal/candidate/prepared.json" ||
      ! cmp -s "$journal/previous/egress.json" "$journal/candidate/egress.json"; then
      "${compose[@]}" stop --timeout 120 api
      upgrade_command apply
      activate_preparation "$journal/candidate"
    fi
    touch -- "$journal/applied"
  fi
  "${compose[@]}" up -d --pull never --wait --wait-timeout 120
  trap - EXIT
  close_upgrade_journal
}
