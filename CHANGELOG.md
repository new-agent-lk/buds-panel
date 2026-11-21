# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/REPO_URL/buds-panel/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/REPO_URL/buds-panel/releases/tag/v0.1.0
