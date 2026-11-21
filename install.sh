#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
# AAC in the high-quality backend is built against FDK-AAC. The plugin is
# shipped in runtime/, but the runtime library is a system dependency.
if [ ! -e /lib/x86_64-linux-gnu/libfdk-aac.so.2 ] && [ ! -e /usr/lib/x86_64-linux-gnu/libfdk-aac.so.2 ]; then
    echo "缺少 AAC 运行库：请安装 libfdk-aac2（高质量模式才会显示 AAC）" >&2
fi
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
EXT_DIR="$DATA_DIR/gnome-shell/extensions/xiaomi-buds5-panel@kang"
SERVICE_DIR="$CONFIG_DIR/systemd/user"
AUTOSTART_DIR="$CONFIG_DIR/autostart"

install -d "$EXT_DIR" "$SERVICE_DIR" "$AUTOSTART_DIR" "$HOME/.local/bin"

# PipeWire 1.7's ALSA ACP layer searches /usr/local/share by default, while
# Zorin/Ubuntu installs card profiles under /usr/share. Keep this compatibility
# link so both the USB wired headset and the internal HDA card remain visible
# in high-quality mode. The startup script also sets ACP_*_DIR explicitly.
mkdir -p "$CONFIG_DIR/alsa-card-profile"
ln -sfn /usr/share/alsa-card-profile/mixer "$HOME/.config/alsa-card-profile/mixer"
install -m 0644 "$ROOT/extension/xiaomi-buds5-panel@kang/metadata.json" "$EXT_DIR/metadata.json"
install -m 0644 "$ROOT/extension/xiaomi-buds5-panel@kang/extension.js" "$EXT_DIR/extension.js"
install -m 0644 "$ROOT/extension/xiaomi-buds5-panel@kang/stylesheet.css" "$EXT_DIR/stylesheet.css"
install -m 0644 "$ROOT/systemd/xiaomi-buds5-panel.service" "$SERVICE_DIR/xiaomi-buds5-panel.service"
install -m 0644 "$ROOT/systemd/xiaomi-buds5-tray.service" "$SERVICE_DIR/xiaomi-buds5-tray.service"
sed "s|@ROOT@|$ROOT|g" "$ROOT/systemd/xiaomi-buds5-adaptive.service" > "$SERVICE_DIR/xiaomi-buds5-adaptive.service"
install -m 0644 "$ROOT/xiaomi-buds5-tray.desktop" "$AUTOSTART_DIR/xiaomi-buds5-tray.desktop"

# Build and install the Rust control daemon, tray and CLI (everything is Rust; no Python).
CARGO="${CARGO:-$HOME/.cargo/bin/cargo}"
echo "构建 Rust 后端与托盘…"
( cd "$ROOT" && "$CARGO" build --release )
install -m 0755 "$ROOT/target/release/xiaomi-buds5-service" "$HOME/.local/bin/xiaomi-buds5-service"
# Replace any old symlink to the Python wrapper before installing the binary.
rm -f "$HOME/.local/bin/xiaomi-buds5-tray"
install -m 0755 "$ROOT/target/release/xiaomi-buds5-tray" "$HOME/.local/bin/xiaomi-buds5-tray"
rm -f "$HOME/.local/bin/xiaomi-buds5"
install -m 0755 "$ROOT/target/release/xiaomi-buds5" "$HOME/.local/bin/xiaomi-buds5"
systemctl --user daemon-reload
systemctl --user enable --now xiaomi-buds5-panel.service
systemctl --user enable --now xiaomi-buds5-tray.service
# The adaptive stack is opt-in from the panel; do not enable it at login.
systemctl --user disable xiaomi-buds5-adaptive.service >/dev/null 2>&1 || true
if gnome-extensions list | grep -qx 'xiaomi-buds5-panel@kang'; then
    gnome-extensions enable 'xiaomi-buds5-panel@kang' || true
else
    echo "扩展已安装。GNOME 43 需要注销并重新登录后才会发现新扩展。"
fi
echo "服务状态："
systemctl --user --no-pager --lines=0 status xiaomi-buds5-panel.service || true
