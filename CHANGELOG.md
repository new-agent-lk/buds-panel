# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-11-23

The popover is now a GPUI window instead of a GTK popover; the GTK tray stays
available as a fallback.

### Added
- `xiaomi-buds5-panel`, a GPUI front-end and the new default popover: battery,
  noise mode/level, codec and audio-backend switching, volume slider, and a
  settings page. The default `./install.sh` installs it; `--ui=gtk` keeps the
  old GTK tray.
- Status notices in the panel: service offline, "switching… / switched to …"
  for codec and audio-backend changes, and one-shot error messages.
- The volume slider follows system volume changes made elsewhere (media keys,
  other apps) while the panel is open.

### Changed
- `install.sh` gained `--ui=gtk|gpui` (default `gpui`) and now installs the
  panel's systemd unit and autostart entry.
- CI installs the GPUI build dependencies, and `rust-toolchain.toml` pins the
  workspace to Rust 1.98.1.
- README documents the GPUI requirements (Vulkan/OpenGL, XWayland, build deps).

### Fixed
- The panel no longer freezes after repeated open/close: the window stays
  mapped and is parked off-screen instead of being unmapped and remapped, which
  raced the Vulkan swapchain and stalled the UI thread.
- Tray activation toggles the panel, and activations that arrive in the same
  tick keep their order.
- The panel hides reliably once the pointer leaves it.
- Noise-level writes are ignored while ANC is off instead of surfacing
  `invalid noise level`.

### Notes
- Known limitation: with the Zorin/Ubuntu appindicator extension a
  StatusNotifierItem without a DBusMenu only receives `Activate` on double
  click, so the panel opens on double click. The GTK tray behaved the same way.

## [0.1.0] - 2026-11-21

First public source release.

### Added
- Rust D-Bus service (`xiaomi-buds5-service`) exposing `io.github.kang.XiaomiBuds5`:
  battery, noise mode/level, codec switching, audio-backend switching, and live
  `StateChanged` signals.
- GTK3 status tray + popover (`xiaomi-buds5-tray`) with a hand-written
  StatusNotifierItem, battery/volume/noise controls and a codec picker.
- `xiaomi-buds5` command-line client: `status`, `noise`, `selftest`.
- Optional GNOME 43 Quick Settings extension.
- Optional out-of-tree high-quality audio stack (`audio-adaptive/`) that can drive
  an isolated PipeWire 1.7 + aptX Adaptive encoder; the panel falls back to the
  system audio server when it is absent.
- Automatic earbud discovery via `bluetoothctl` (override with `XIAOMI_BUDS5_MAC`).

### Notes
- Tested on Xiaomi Buds 5 (R1). Other JieLi RCSP-based earbuds may work but are
  not individually verified.
- aptX Adaptive is a Qualcomm patented technology and ships out of tree; see
  `docs/aptx-adaptive.md` and `NOTICE`.

[Unreleased]: https://github.com/new-agent-lk/buds-panel/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/new-agent-lk/buds-panel/releases/tag/v0.2.0
[0.1.0]: https://github.com/new-agent-lk/buds-panel/releases/tag/v0.1.0
