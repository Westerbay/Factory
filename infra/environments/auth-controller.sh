#!/bin/sh
set -eu
# No caller-selected command, PID, home or lifetime. Timeout owns only this process group.
test "$#" -eq 0
umask 077
export HOME=/home/factory CODEX_HOME=/home/factory/.codex
export PATH=/usr/local/bin:/usr/bin:/bin
mkdir -p "$CODEX_HOME"
chmod 0700 "$CODEX_HOME"
if test -f "$CODEX_HOME/auth.json"; then chmod 0600 "$CODEX_HOME/auth.json"; fi
# The private tmpfs is per container. Exit 75 means a controller is already active.
# Closing Docker stdin is best effort; the watchdog also bounds orphan recovery.
exec /usr/bin/timeout --signal=TERM --kill-after=5s 595s \
  /usr/bin/flock --nonblock --conflict-exit-code=75 --no-fork /tmp/factory-auth.lock \
  /opt/codex/bin/codex -c 'cli_auth_credentials_store="file"' app-server --listen stdio://
