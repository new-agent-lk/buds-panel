# Shared environment for the isolated aptX Adaptive PipeWire stack.
# This stack only owns the Buds 5 A2DP endpoint while active; the system
# PulseAudio units are stopped by the launcher and restored on exit.
# Repo root is the parent of this audio-adaptive/ dir, so the tree can live anywhere.
BUDS_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD="$BUDS_ROOT/runtime/pipewire-build"
RUNTIME="$HOME/.local/share/xiaomi-buds5-panel/adaptive-runtime"
LOG="$HOME/.local/share/xiaomi-buds5-panel/adaptive-logs"
STATE="$HOME/.local/share/xiaomi-buds5-panel/state"
# Earbuds MAC for the isolated stack: $XIAOMI_BUDS5_MAC overrides auto-detect.
BUDS_MAC="${XIAOMI_BUDS5_MAC:-$(bluetoothctl paired-devices 2>/dev/null | awk 'tolower($0) ~ /buds|airdots/ {print $2; exit}')}"
