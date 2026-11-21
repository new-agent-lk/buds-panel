#!/bin/bash
# Fetch the optional aptX Adaptive stack (custom PipeWire 1.7 build + patched
# bluez5 SPA plugin) into a local directory so the panel can offer aptX
# Adaptive HQ. This stack is NOT in the source repo (aptX patent isolation);
# see docs/aptx-adaptive.md. Without it the panel simply uses system SBC/AAC.
#
# Usage:
#   scripts/fetch-aptx-stack.sh --url <tarball-url>   # prebuilt Release asset
#   scripts/fetch-aptx-stack.sh --from <dir>          # copy an existing stack tree
#   scripts/fetch-aptx-stack.sh                       # show this help
#
# Options:
#   --target <dir>   install location (default: $XIAOMI_BUDS5_APTX_RUNTIME
#                    or ~/.local/share/xiaomi-buds5-panel/aptx-stack)
#
# Expected layout under <target> (from a Release asset or an existing build):
#   pipewire-build/            # PipeWire/WirePlumber build tree (daemons+libs)
#   payload/spa/bluez5/*.so    # the patched bluez5 codec plugin
#   payload/config/            # wireplumber config drop-ins
# (Or experimental/run/{spa,config} instead of payload/.)
set -euo pipefail

TARGET="${XIAOMI_BUDS5_APTX_RUNTIME:-$HOME/.local/share/xiaomi-buds5-panel/aptx-stack}"
SRC=""
URL=""

usage() { sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; }

has_layout() {
  [[ -x "$1/pipewire-build/daemon/pipewire" ]] || return 1
  [[ -d "$1/payload/spa" || -d "$1/experimental/run/spa" ]]
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --from)   SRC="${2:?--from 需要目录}"; shift 2 ;;
    --url)    URL="${2:?--url 需要链接}"; shift 2 ;;
    --target) TARGET="${2:?--target 需要目录}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "未知参数：$1（见 --help）" >&2; exit 2 ;;
  esac
done

if [[ -z $SRC && -z $URL ]]; then usage; exit 2; fi

mkdir -p "$TARGET"
if [[ -n $SRC ]]; then
  [[ -d $SRC ]] || { echo "错误：--from 目录不存在：$SRC" >&2; exit 1; }
  echo "从 $SRC 复制栈到 $TARGET …"
  cp -a "$SRC/." "$TARGET/"
elif [[ -n $URL ]]; then
  echo "下载 $URL …"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  curl -fL "$URL" -o "$tmp/stack.tar" || { echo "下载失败" >&2; exit 1; }
  tar -xf "$tmp/stack.tar" -C "$TARGET"
  # Unpack into a single top-level dir? Flatten it into TARGET.
  if ! has_layout "$TARGET"; then
    inner="$(find "$TARGET" -maxdepth 2 -name pipewire-build -type d | head -n1)"
    if [[ -n $inner ]]; then
      top="$(dirname "$inner")"
      shopt -s dotglob
      mv "$top"/* "$TARGET"/ && rmdir "$top" 2>/dev/null || true
      shopt -u dotglob
    fi
  fi
fi

if ! has_layout "$TARGET"; then
  echo "错误：$TARGET 下未找到完整栈（需要 pipewire-build/daemon/pipewire 与 payload/spa 或 experimental/run/spa）。" >&2
  exit 1
fi

echo
echo "aptX Adaptive 可选栈已就绪：$TARGET"
echo "启用方式（让服务看到它）："
echo "  export XIAOMI_BUDS5_APTX_RUNTIME=\"$TARGET\""
echo "然后在面板『声音设置』里选择音频后端 adaptive / 协议 aptX Adaptive。"
