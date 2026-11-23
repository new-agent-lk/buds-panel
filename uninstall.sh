#!/bin/bash
set -euo pipefail
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
systemctl --user disable --now xiaomi-buds5-panel-gpui.service 2>/dev/null || true
systemctl --user disable --now xiaomi-buds5-tray.service 2>/dev/null || true
systemctl --user disable --now xiaomi-buds5-adaptive.service 2>/dev/null || true
systemctl --user disable --now xiaomi-buds5-panel.service 2>/dev/null || true
rm -f "$CONFIG_DIR/systemd/user/xiaomi-buds5-panel.service" "$CONFIG_DIR/systemd/user/xiaomi-buds5-tray.service" "$CONFIG_DIR/systemd/user/xiaomi-buds5-adaptive.service" "$CONFIG_DIR/systemd/user/xiaomi-buds5-panel-gpui.service"
rm -f "$CONFIG_DIR/autostart/xiaomi-buds5-tray.desktop" "$CONFIG_DIR/autostart/xiaomi-buds5-panel-gpui.desktop"
rm -rf "$DATA_DIR/gnome-shell/extensions/xiaomi-buds5-panel@kang"
rm -f "$HOME/.local/bin/xiaomi-buds5-service" "$HOME/.local/bin/xiaomi-buds5" "$HOME/.local/bin/xiaomi-buds5-tray" "$HOME/.local/bin/xiaomi-buds5-panel"
systemctl --user daemon-reload
