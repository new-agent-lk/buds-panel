#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"

# The GNOME Shell extension is optional and community-maintained; it is NOT
# installed or enabled by default. Pass --with-gnome-extension to include it.
WITH_GNOME_EXT=0
# Front-end: gpui (new panel, default) or gtk (old tray, kept as a fallback).
UI="${XIAOMI_BUDS5_UI:-gpui}"
for arg in "$@"; do
    case "$arg" in
        --with-gnome-extension) WITH_GNOME_EXT=1 ;;
        --ui=gtk|--ui=gpui) UI="${arg#--ui=}" ;;
        --ui=*) echo "无效的 --ui 值：${arg#--ui=}（可选 gtk 或 gpui）" >&2; exit 2 ;;
        -h|--help)
            echo "用法: $0 [--ui=gtk|gpui] [--with-gnome-extension]"
            echo "  --ui=gtk|gpui            选择前端：gpui（默认，新面板）或 gtk（旧托盘，可回滚）"
            echo "  --with-gnome-extension  同时安装并启用 GNOME Shell 扩展（可选、实验性、GNOME 43+ 未验证；默认不装）"
            exit 0
            ;;
        *) echo "未知参数: $arg（见 --help）" >&2; exit 2 ;;
    esac
done
case "$UI" in gtk|gpui) ;; *) echo "无效的前端：$UI（可选 gtk 或 gpui）" >&2; exit 2 ;; esac

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

install -d "$SERVICE_DIR" "$AUTOSTART_DIR" "$HOME/.local/bin"

# PipeWire 1.7's ALSA ACP layer searches /usr/local/share by default, while
# Zorin/Ubuntu installs card profiles under /usr/share. Keep this compatibility
# link so both the USB wired headset and the internal HDA card remain visible
# in high-quality mode. The startup script also sets ACP_*_DIR explicitly.
mkdir -p "$CONFIG_DIR/alsa-card-profile"
ln -sfn /usr/share/alsa-card-profile/mixer "$HOME/.config/alsa-card-profile/mixer"
if [ "$WITH_GNOME_EXT" = 1 ]; then
    install -d "$EXT_DIR"
    install -m 0644 "$ROOT/extension/xiaomi-buds5-panel@kang/metadata.json" "$EXT_DIR/metadata.json"
    install -m 0644 "$ROOT/extension/xiaomi-buds5-panel@kang/extension.js" "$EXT_DIR/extension.js"
    install -m 0644 "$ROOT/extension/xiaomi-buds5-panel@kang/stylesheet.css" "$EXT_DIR/stylesheet.css"
fi
install -m 0644 "$ROOT/systemd/xiaomi-buds5-panel.service" "$SERVICE_DIR/xiaomi-buds5-panel.service"
install -m 0644 "$ROOT/systemd/xiaomi-buds5-tray.service" "$SERVICE_DIR/xiaomi-buds5-tray.service"
install -m 0644 "$ROOT/systemd/xiaomi-buds5-panel-gpui.service" "$SERVICE_DIR/xiaomi-buds5-panel-gpui.service"
sed "s|@ROOT@|$ROOT|g" "$ROOT/systemd/xiaomi-buds5-adaptive.service" > "$SERVICE_DIR/xiaomi-buds5-adaptive.service"

# Build and install the Rust control daemon, tray and CLI (everything is Rust; no Python).
CARGO="${CARGO:-$HOME/.cargo/bin/cargo}"
echo "构建 Rust 后端与前端…"
( cd "$ROOT" && "$CARGO" build --release )
install -m 0755 "$ROOT/target/release/xiaomi-buds5-service" "$HOME/.local/bin/xiaomi-buds5-service"
# Replace any old symlink to the Python wrapper before installing the binary.
rm -f "$HOME/.local/bin/xiaomi-buds5-tray"
install -m 0755 "$ROOT/target/release/xiaomi-buds5-tray" "$HOME/.local/bin/xiaomi-buds5-tray"
rm -f "$HOME/.local/bin/xiaomi-buds5"
install -m 0755 "$ROOT/target/release/xiaomi-buds5" "$HOME/.local/bin/xiaomi-buds5"
rm -f "$HOME/.local/bin/xiaomi-buds5-panel"
install -m 0755 "$ROOT/target/release/xiaomi-buds5-panel" "$HOME/.local/bin/xiaomi-buds5-panel"
systemctl --user daemon-reload
systemctl --user enable --now xiaomi-buds5-panel.service
if [ "$UI" = "gpui" ]; then
    # New GPUI panel. Stop the GTK tray first: both own the same SNI name and socket.
    systemctl --user disable --now xiaomi-buds5-tray.service >/dev/null 2>&1 || true
    systemctl --user enable --now xiaomi-buds5-panel-gpui.service
    install -m 0644 "$ROOT/xiaomi-buds5-panel-gpui.desktop" "$AUTOSTART_DIR/xiaomi-buds5-panel-gpui.desktop"
    rm -f "$AUTOSTART_DIR/xiaomi-buds5-tray.desktop"
else
    systemctl --user disable --now xiaomi-buds5-panel-gpui.service >/dev/null 2>&1 || true
    systemctl --user enable --now xiaomi-buds5-tray.service
    install -m 0644 "$ROOT/xiaomi-buds5-tray.desktop" "$AUTOSTART_DIR/xiaomi-buds5-tray.desktop"
    rm -f "$AUTOSTART_DIR/xiaomi-buds5-panel-gpui.desktop"
fi
# The adaptive stack is opt-in from the panel; do not enable it at login.
systemctl --user disable xiaomi-buds5-adaptive.service >/dev/null 2>&1 || true
if [ "$WITH_GNOME_EXT" = 1 ]; then
    if gnome-extensions list | grep -qx 'xiaomi-buds5-panel@kang'; then
        gnome-extensions enable 'xiaomi-buds5-panel@kang' || true
    else
        echo "扩展已安装。GNOME 43 需要注销并重新登录后才会发现新扩展。"
    fi
else
    echo "未安装 GNOME 扩展（如需：./install.sh --with-gnome-extension）。当前前端：$UI。"
fi
echo "服务状态："
systemctl --user --no-pager --lines=0 status xiaomi-buds5-panel.service || true
if [ "$UI" = "gpui" ]; then
    systemctl --user --no-pager --lines=0 status xiaomi-buds5-panel-gpui.service || true
else
    systemctl --user --no-pager --lines=0 status xiaomi-buds5-tray.service || true
fi
