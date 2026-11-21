#!/bin/bash
# Foreground supervisor for systemd: starts the isolated stack, then waits.
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/audio-adaptive/env.sh"

cleanup() {
  "$ROOT/audio-adaptive/stop.sh" >/dev/null 2>&1 || true
  exit 0
}
trap cleanup INT TERM

# start.sh exits non-zero when the Buds endpoint cannot be brought up. In that
# case its failure path already restored the previous backend; exit instead of
# keeping a dead supervisor alive and presenting a false "active" state.
if ! "$ROOT/audio-adaptive/start.sh"; then
  exit 1
fi
while true; do sleep 1; done
