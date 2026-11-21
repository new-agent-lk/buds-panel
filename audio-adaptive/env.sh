# Shared environment for the isolated aptX Adaptive PipeWire stack.
# This stack only owns the Buds 5 A2DP endpoint while active; the system
# PulseAudio units are stopped by the launcher and restored on exit.
# Repo root is the parent of this audio-adaptive/ dir, so the tree can live anywhere.
BUDS_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# The aptX Adaptive stack (a custom PipeWire 1.7 build + a patched bluez5 SPA
# plugin) is NOT shipped in the source repo, to keep the aptX patent-encumbered
# binary out of the public tree. Resolve where it lives, in priority order:
#   1. $XIAOMI_BUDS5_APTX_RUNTIME  — a Release asset you unpacked, or a submodule checkout
#   2. ~/.local/share/xiaomi-buds5-panel/aptx-stack — default target of scripts/fetch-aptx-stack.sh
#   3. $BUDS_ROOT/runtime           — a stack you built in place
# Empty means "not installed"; start.sh then exits with instructions so the
# panel cleanly falls back to the system SBC/AAC codecs.
if [[ -n ${XIAOMI_BUDS5_APTX_RUNTIME:-} ]]; then
  APTX_STACK="$XIAOMI_BUDS5_APTX_RUNTIME"
elif [[ -d "$HOME/.local/share/xiaomi-buds5-panel/aptx-stack/pipewire-build" ]]; then
  APTX_STACK="$HOME/.local/share/xiaomi-buds5-panel/aptx-stack"
elif [[ -d "$BUDS_ROOT/runtime/pipewire-build" ]]; then
  APTX_STACK="$BUDS_ROOT/runtime"
else
  APTX_STACK=""
fi
# BUILD = the PipeWire/WirePlumber build tree (daemons, modules, libs).
BUILD="${APTX_STACK:+$APTX_STACK/pipewire-build}"
# APTX_PAYLOAD = the patched plugin .so + wireplumber config to copy into $RUNTIME.
if [[ -d "$APTX_STACK/payload/spa" ]]; then
  APTX_PAYLOAD="$APTX_STACK/payload"                       # Release/fetch layout
elif [[ -d "$APTX_STACK/experimental/run/spa" ]]; then
  APTX_PAYLOAD="$APTX_STACK/experimental/run"              # submodule/Release with experimental/run
elif [[ -d "$BUDS_ROOT/experimental-aptx-adaptive/run/spa" ]]; then
  APTX_PAYLOAD="$BUDS_ROOT/experimental-aptx-adaptive/run" # in-repo dev layout
else
  APTX_PAYLOAD=""
fi

RUNTIME="$HOME/.local/share/xiaomi-buds5-panel/adaptive-runtime"
LOG="$HOME/.local/share/xiaomi-buds5-panel/adaptive-logs"
STATE="$HOME/.local/share/xiaomi-buds5-panel/state"
# Earbuds MAC for the isolated stack: $XIAOMI_BUDS5_MAC overrides auto-detect.
BUDS_MAC="${XIAOMI_BUDS5_MAC:-$(bluetoothctl paired-devices 2>/dev/null | awk 'tolower($0) ~ /buds|airdots/ {print $2; exit}')}"
