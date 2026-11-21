# Buds Panel — GNOME Shell extension (optional)

This directory contains an **optional, community-maintained** GNOME Shell 43 Quick
Settings extension. The primary UI of Buds Panel is the **GTK status tray**; you do
not need this extension to use the app.

## Caveats

- **Not installed by default.** Run `./install.sh --with-gnome-extension` to install
  and enable it, then log out and back in once (GNOME only scans extension
  directories at login).
- **Experimental / unverified.** Developed and tested only against **GNOME 43**. It
  may not load on newer GNOME releases without changes.
- **Needs SNI/AppIndicator support.** GNOME Shell has no built-in StatusNotifierItem
  host; the tile relies on the shell's Quick Settings mechanism and, for the
  indicator, an AppIndicator/SNI provider being present.
- **Community-maintained.** Bugs and PRs welcome, but it may lag behind the tray.

## How it works

The extension is a thin D-Bus client: it reads state from and sends commands to the
`io.github.kang.XiaomiBuds5` D-Bus service (the same one the tray and CLI use). It
never touches Bluetooth or audio directly.

- `metadata.json` — extension manifest (`uuid`, `shell-version: ["43"]`).
- `extension.js` — the Quick Settings tile.
- `stylesheet.css` — tile styling.
