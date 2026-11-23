mod dbus;
mod model;
mod ui;

use gpui_kit::*;

fn main() {
    // GPUI popup needs a frameless, absolutely-positioned window; GNOME/Wayland
    // forces server-side decorations (and forbids absolute placement), so run on
    // XWayland (X11 backend) whenever a display is available.
    // shortcut: trusts the session's XAUTHORITY cookie, falling back to the
    // newest mutter Xwayland auth file when the inherited one is stale.
    if std::env::var_os("DISPLAY").is_some() {
        std::env::remove_var("WAYLAND_DISPLAY");
        // The panel has no text inputs; drop the XIM (ibus) hook so zed-xim does
        // not panic with `UnsupportedEncoding` when starting on XWayland.
        std::env::set_var("XMODIFIERS", "");
        let auth_ok = std::env::var_os("XAUTHORITY")
            .map(|p| std::path::Path::new(&p).exists())
            .unwrap_or(false);
        if !auth_ok {
            let dir = std::env::var_os("XDG_RUNTIME_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("/run/user/1000"));
            if let Ok(entries) = std::fs::read_dir(dir) {
                let newest = entries
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.starts_with(".mutter-Xwaylandauth."))
                            .unwrap_or(false)
                    })
                    .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok());
                if let Some(p) = newest {
                    std::env::set_var("XAUTHORITY", p);
                }
            }
        }
    }

    // Three flume channels bridging the glib thread and the GPUI thread.
    let (ui_tx, ui_rx) = flume::unbounded();
    let (svc_tx, svc_rx) = flume::unbounded();
    let (sni_tx, sni_rx) = flume::unbounded();

    let _glib_thread = dbus::spawn(ui_tx, svc_rx, sni_rx);

    application().with_assets(assets::Assets).run(move |cx| {
        init(cx);
        // The component root paints the theme background across the window,
        // which showed up as a white square behind the popover's rounded card.
        let theme = gpui_kit::component::Theme::global_mut(cx);
        theme.tokens.background = gpui_kit::transparent_black().into();
        // ...and it outlines the surface with `window_border`, which read as a
        // dark ring around the card once the background went transparent.
        theme.window_border = gpui_kit::transparent_black();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                // Parked off-screen from the first frame. GPUI maps the window
                // once at creation and the popover stays mapped from then on.
                point(px(-10000.), px(-10000.)),
                // 380x580 including the 24px transparent shadow margin (see
                // ui::render).
                size(px(380.), px(580.)),
            ))),
            // PopUp creates the window override-redirect from the start, so
            // Mutter never maps or places it. Flipping the bit after creation
            // is too late: GPUI has already mapped it.
            kind: WindowKind::PopUp,
            // Don't steal keyboard focus at login; the panel shows only when
            // the tray icon is clicked.
            focus: false,
            window_decorations: Some(WindowDecorations::Client),
            window_background: WindowBackgroundAppearance::Transparent,
            is_resizable: false,
            ..Default::default()
        };
        open_window(options, cx, |_window, cx| {
            cx.new(|cx| ui::Panel::new(cx, ui_rx, svc_tx, sni_tx))
        })
        .expect("failed to open panel window");
    });

    // The GPUI run loop returned (window closed / quit): end the process so the
    // still-running glib thread does not keep us alive.
    std::process::exit(0);
}
