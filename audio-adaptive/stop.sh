#!/bin/bash
# Stop the isolated adaptive stack and restore exactly one working backend.
# PulseAudio and PipeWire must never run together on this machine: they both
# claim the same ALSA cards, which made the wired USB headset disappear.
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/audio-adaptive/env.sh"
BACKEND_STATE="$STATE/previous-backend"

if [[ -f "$STATE/adaptive.pid" ]]; then
  mapfile -t adaptive_pids < "$STATE/adaptive.pid"
  for pid in "${adaptive_pids[@]}"; do
    [[ -n "$pid" ]] && kill "$pid" 2>/dev/null || true
  done
  rm -f "$STATE/adaptive.pid"
else
  adaptive_pids=()
fi

# Give the private daemons time to close their ALSA/Pulse sockets before the
# original backend starts. Do not wait forever if a process is stuck.
for _ in $(seq 1 20); do
  any_alive=0
  for pid in "${adaptive_pids[@]}"; do
    kill -0 "$pid" 2>/dev/null && any_alive=1
  done
  [[ "$any_alive" -eq 0 ]] && break
  sleep 0.25
done

systemctl --user unmask --runtime pulseaudio.socket pulseaudio.service >/dev/null 2>&1 || true

backend="pulse"
[[ -f "$BACKEND_STATE" ]] && backend=$(cat "$BACKEND_STATE" 2>/dev/null || echo pulse)

# The system package here has PipeWire core but no pipewire-pulse and no
# WirePlumber. A bare PipeWire process cannot serve Pulse clients or expose
# ALSA sinks, so restore PulseAudio unless a complete PipeWire stack is
# actually installed.
if [[ "$backend" == "pipewire" ]] && command -v pipewire-pulse >/dev/null 2>&1 \
   && { command -v wireplumber >/dev/null 2>&1 || systemctl --user cat wireplumber.service >/dev/null 2>&1; }; then
  systemctl --user start pipewire.socket pipewire.service
  systemctl --user start wireplumber.service 2>/dev/null || systemctl --user start pipewire-media-session.service 2>/dev/null || true
  systemctl --user start pipewire-pulse.socket pipewire-pulse.service 2>/dev/null || true
  echo "restored system PipeWire"
else
  systemctl --user start pulseaudio.socket pulseaudio.service 2>/dev/null || true
  sleep 2
  [[ -n ${BUDS_MAC:-} ]] && pactl send-message "/card/bluez_card.${BUDS_MAC//:/_}/bluez" switch-codec '"aptx"' >/dev/null 2>&1 || true
  echo "restored system PulseAudio / aptX Classic"
fi
