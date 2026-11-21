#!/bin/bash
# Start the isolated PipeWire 1.7 + aptX Adaptive stack for Xiaomi Buds 5.
# The system PulseAudio stack is stopped first; stop.sh restores it.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/audio-adaptive/env.sh"
# The isolated aptX Adaptive stack only makes sense with a paired earbud.
if [[ -z ${BUDS_MAC:-} ]]; then
  echo "未找到已配对的耳机（可用 XIAOMI_BUDS5_MAC 指定），无法启动 aptX Adaptive 栈" >&2
  exit 1
fi
# The optional stack is not shipped in the source repo (aptX patent isolation).
# If it is missing, fail loudly with install instructions instead of half-starting.
if [[ -z ${APTX_STACK:-} || ! -x "$BUILD/daemon/pipewire" ]]; then
  cat >&2 <<'EOF'
错误：未找到 aptX Adaptive 可选栈（自定义 PipeWire 1.7 构建 + 打补丁的 bluez5 插件）。
它不随源码仓库分发（aptX 专利隔离，详见 docs/aptx-adaptive.md）。安装方式二选一：
  1) 下载预编译栈：  scripts/fetch-aptx-stack.sh
  2) 自行构建：      git clone https://github.com/AKS-Logic/aptx-adaptive-linux
                     按其 README 构建后，export XIAOMI_BUDS5_APTX_RUNTIME=<解包/检出目录>
不装也没关系：面板默认用系统 pactl 的 SBC/AAC，会自动降级、无需本栈。
EOF
  exit 1
fi
if [[ -z ${APTX_PAYLOAD:-} ]]; then
  echo "错误：在 $APTX_STACK 下没找到插件载荷（payload/spa 或 experimental/run/spa）。栈不完整，请重新获取。" >&2
  exit 1
fi
mkdir -p "$RUNTIME" "$LOG" "$STATE"

# Refuse to run twice.
if [[ -f "$STATE/adaptive.pid" ]] && kill -0 "$(cat "$STATE/adaptive.pid")" 2>/dev/null; then
  echo "aptX Adaptive stack already running (pid $(cat "$STATE/adaptive.pid"))"
  exit 0
fi

# Record which backend was actually in use before replacing it. stop.sh uses
# this to restore exactly one sound server; starting both PulseAudio and
# PipeWire causes ALSA device contention and makes the wired headset vanish.
previous_backend="pulse"
if systemctl --user is-active --quiet pipewire.service 2>/dev/null; then
  previous_backend="pipewire"
fi
printf '%s\n' "$previous_backend" > "$STATE/previous-backend"

# Mask the stock Pulse socket while adaptive mode owns the standard socket;
# otherwise systemd socket activation can restart PulseAudio behind our back
# the moment a desktop app connects.
systemctl --user stop pulseaudio.socket pulseaudio.service \
  pipewire.socket pipewire.service pipewire-media-session.service 2>/dev/null || true
systemctl --user mask --runtime pulseaudio.socket pulseaudio.service >/dev/null 2>&1 || true
rm -rf "$RUNTIME"/*
mkdir -p "$RUNTIME"/{pipewire,pulse,spa/bluez5,wireplumber}
cp -a "$APTX_PAYLOAD/spa/bluez5/." "$RUNTIME/spa/bluez5/"
cp -a "$APTX_PAYLOAD/config/." "$RUNTIME/wireplumber/"

export SPA_PLUGIN_DIR="$RUNTIME/spa:$BUILD/spa-plugins"
export SPA_DATA_DIR="$BUILD/spa-data"
export ALSA_CONFIG_DIR="/usr/share/alsa"
# PipeWire 1.7's ALSA compatibility layer defaults to /usr/local/share,
# while Zorin/Ubuntu ships the ACP profile data under /usr/share. Without
# these overrides both the USB wired headset and the internal HDA card only
# expose the "pro-audio" profile and no normal PulseAudio sink is created.
export ACP_PATHS_DIR="/usr/share/alsa-card-profile/mixer/paths"
export ACP_PROFILES_DIR="/usr/share/alsa-card-profile/mixer/profile-sets"
export PIPEWIRE_MODULE_DIR="$BUILD/modules"
export PIPEWIRE_CONFIG_DIR="$BUILD/daemon"
export WIREPLUMBER_MODULE_DIR="$BUILD/wireplumber-modules"
export WIREPLUMBER_CONFIG_DIR="$RUNTIME/wireplumber:$BUILD/wireplumber-src/config"
export WIREPLUMBER_DATA_DIR="$BUILD/wireplumber-src"
export LD_LIBRARY_PATH="$BUILD/wireplumber-lib:$BUILD/pipewire-lib:${LD_LIBRARY_PATH:-}"
export PATH="$BUILD/daemon:$BUILD/src/tools:$BUILD/wireplumber-src:$BUILD/wireplumber-src/tools:$PATH"
export PIPEWIRE_RUNTIME_DIR="$RUNTIME/pipewire"
export PULSE_RUNTIME_PATH="$RUNTIME/pulse"
export APTX_ADAPTIVE_SLOT=aptx
export APTX_ADAPTIVE_DEFAULT_MODE=hq
export APTX_ADAPTIVE_THREE_PROFILES=1
# ST0 disabled: force host-timed (source-type 2); the sink then times playback itself instead of
# needing link-clock timestamps the host cannot supply (ADR 0001: source-type 0 => sink stays silent)
# export APTX_ADAPTIVE_ST0=1
export APTX_ADAPTIVE_START_LOW=1
export APTX_ADAPTIVE_HEADROOM_DB=1
export APTX_ADAPTIVE_SOURCE_TYPE_OVERRIDES="${BUDS_MAC}=2"
export APTX_ADAPTIVE_LOG_SOURCE_TYPE=1
export PIPEWIRE_LOG_SYSTEMD=false
export PIPEWIRE_DEBUG=2
export WIREPLUMBER_DEBUG=2

# The build's binaries link against their uninstalled build-tree paths. Keep
# the whole stack self-contained under this project so a /tmp cleanup or a
# reboot cannot break the service.
"$BUILD/daemon/pipewire" >"$LOG/pipewire.log" 2>&1 & PW_PID=$!
sleep 1
"$BUILD/wireplumber-src/wireplumber" >"$LOG/wireplumber.log" 2>&1 & WP_PID=$!
sleep 2
# pipewire-pulse-adaptive.conf listens on both the private runtime socket
# and the standard /run/user/$UID/pulse/native socket. Keeping the standard
# socket pointed at this server is what makes ordinary desktop apps play to
# the Buds instead of a stale PulseAudio socket.
"$BUILD/daemon/pipewire-pulse" -c pipewire-pulse-adaptive.conf >"$LOG/pipewire-pulse.log" 2>&1 & PULSE_PID=$!

printf '%s\n%s\n%s\n' "$PW_PID" "$WP_PID" "$PULSE_PID" > "$STATE/adaptive.pid"

# Wait for the private Pulse server and the Buds card.
for _ in $(seq 1 30); do
  if PULSE_SERVER="unix:$RUNTIME/pulse/native" pactl info >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

# Force a clean A2DP renegotiation under the new BlueZ endpoints.
bluetoothctl disconnect "$BUDS_MAC" >/dev/null 2>&1 || true
sleep 1
bluetoothctl connect "$BUDS_MAC" >/dev/null 2>&1 || true

for _ in $(seq 1 30); do
  card=$(PULSE_SERVER="unix:$RUNTIME/pulse/native" pactl list short cards 2>/dev/null | awk '/bluez_card\./ {print $2; exit}')
  if [[ -n ${card:-} ]]; then
    break
  fi
  sleep 1
done

if [[ -z ${card:-} ]]; then
  echo "aptX Adaptive stack started but the Buds card did not appear; restoring PulseAudio" >&2
  "$ROOT/audio-adaptive/stop.sh" >/dev/null 2>&1 || true
  exit 1
fi

# Prefer the HQ profile explicitly; the adaptive card's "best quality" profile
# does this automatically, but being explicit makes the result deterministic.
# The card appears before its sink, so wait for the sink before selecting HQ.
PULSE_SERVER="unix:$RUNTIME/pulse/native" pactl set-card-profile "$card" a2dp-sink-aptx_adaptive_hq >/dev/null 2>&1 || true
for _ in $(seq 1 20); do
  sink=$(PULSE_SERVER="unix:$RUNTIME/pulse/native" pactl list short sinks 2>/dev/null | awk '/bluez_output\./ {print $2; exit}')
  [[ -n ${sink:-} ]] && break
  sleep 0.5
done
if [[ -z ${sink:-} ]]; then
  echo "aptX Adaptive card is present but its A2DP sink did not appear; restoring PulseAudio" >&2
  "$ROOT/audio-adaptive/stop.sh" >/dev/null 2>&1 || true
  exit 1
fi
PULSE_SERVER="unix:$RUNTIME/pulse/native" pactl set-card-profile "$card" a2dp-sink-aptx_adaptive_hq >/dev/null 2>&1 || true
PULSE_SERVER="unix:$RUNTIME/pulse/native" pactl set-default-sink "$sink" >/dev/null 2>&1 || true

echo "aptX Adaptive stack is running. PULSE_SERVER=unix:$RUNTIME/pulse/native"
