# Buds Panel v0.1.0

**A Linux control panel for Bluetooth earbuds — battery, ANC and codec switching.**

Buds Panel is a small Rust + GTK3 desktop app that talks directly to your Bluetooth earbuds
over their control channel. It shows per-earbud and charging-case battery, switches
noise-cancelling modes, and lets you pick the Bluetooth audio codec — from a popover that hangs
off your top bar, backed by a resident D-Bus service you can also script.

This is the first public release: **source-only** (build it yourself — see below).

## Highlights

- **Real battery, not a guess** — left / right / charging-case percentages read straight from
  the earbuds' RCSP control channel.
- **Noise cancelling in one tap** — off / ANC / transparency, plus a three-step ANC level.
- **Codec switching that actually verifies** — SBC, AAC, aptX, and (with the optional stack)
  aptX Adaptive. The service checks the new sink really came up, re-asserts default routing and
  volume, and rolls back automatically if a switch fails.
- **Live, not cached** — a resident service pushes state over D-Bus `StateChanged`; opening the
  panel triggers an immediate refresh.
- **Scriptable** — everything the UI does is a D-Bus method/`xiaomi-buds5` CLI call.
- **Optional aptX Adaptive / near-lossless** — plug in the out-of-tree high-quality stack
  (`docs/aptx-adaptive.md`); without it the panel cleanly falls back to the stock SBC/AAC codecs.
- **Optional GNOME extension** — a Quick Settings tile variant, community-maintained and opt-in
  (`./install.sh --with-gnome-extension`).

## Install (source build)

```bash
git clone https://github.com/new-agent-lk/buds-panel.git
cd buds-panel
./install.sh            # builds the Rust workspace and installs the D-Bus service + tray
```

Requirements: Rust (stable), `libgtk-3-dev`, `librsvg2-common`, BlueZ. See the README for details.

## What works today

- Tested on **Xiaomi Buds 5 (R1)**: battery, ANC, SBC/AAC/aptX/aptX-Adaptive codec switching.
- Other Xiaomi / Redmi earbuds speaking the JieLi RCSP protocol have a good chance of working;
  they are community-tested, not verified by the maintainer.

## Known limitations

- **Source-only for now** — no prebuilt packages yet (Flatpak / .deb / AUR are planned for v0.2).
- **aptX Adaptive is out-of-tree** — its stack carries patent-encumbered aptX binaries and is not
  shipped here; build or fetch it separately (`docs/aptx-adaptive.md`).
- **GNOME extension is experimental** — not verified on GNOME 43+; it is not installed by default.
- Only one device is actively developed. Multi-point works but the buds' control channel is
  unavailable to the host while a phone owns the link.

## Thanks

The RCSP authentication transform is a Rust port of
`redmi-buds-linux` (MIT, © Deniz Tunç); see `NOTICE`.
The optional codec stack lives in [`AKS-Logic/aptx-adaptive-linux`](https://github.com/AKS-Logic/aptx-adaptive-linux) (MIT).

---

**中文简介**：Buds Panel 是一个 Linux 下的蓝牙耳机控制面板（Rust + GTK3 + 常驻 D-Bus 服务），
可查看左右耳与充电盒电量、切换降噪三态、切换蓝牙音频编码（SBC/AAC/aptX，外接可选栈后支持
aptX Adaptive）。v0.1 仅发布源码，自行构建安装。首个验证机型为 Xiaomi Buds 5。
