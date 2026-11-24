//! D-Bus bridge: everything gio/glib runs on one dedicated glib thread.
//! It owns the StatusNotifierItem, the service client, the StateChanged
//! subscription, a 3s refresh tick, the CLI "show" socket, and a 100ms poll
//! that drains the two GPUI -> glib flume channels.
//!
//! Channels (see docs/gpui-migration.md §4):
//!   glib -> GPUI : Event::{State, Show}
//!   GPUI -> glib : SvcCmd::{Refresh}
//!   GPUI -> glib : SniCmd::{SetStatus}
//! State is resent through one channel; UI commands travel the other two.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use flume::{Receiver, Sender};
use glib::ToVariant;

use crate::model::{self, State};

pub const BUS: &str = "io.github.kang.XiaomiBuds5";
pub const PATH: &str = "/io/github/kang/XiaomiBuds5";
pub const IFACE: &str = "io.github.kang.XiaomiBuds5";

const SNI_XML: &str = r#"
<node>
  <interface name="org.kde.StatusNotifierItem">
    <property name="Category" type="s" access="read"/>
    <property name="Id" type="s" access="read"/>
    <property name="Title" type="s" access="read"/>
    <property name="Status" type="s" access="read"/>
    <property name="WindowId" type="i" access="read"/>
    <property name="IconThemePath" type="s" access="read"/>
    <property name="Menu" type="o" access="read"/>
    <property name="ItemIsMenu" type="b" access="read"/>
    <property name="IconName" type="s" access="read"/>
    <property name="IconPixmap" type="a(iiay)" access="read"/>
    <property name="OverlayIconName" type="s" access="read"/>
    <property name="OverlayIconPixmap" type="a(iiay)" access="read"/>
    <property name="AttentionIconName" type="s" access="read"/>
    <property name="AttentionIconPixmap" type="a(iiay)" access="read"/>
    <property name="AttentionMovieName" type="s" access="read"/>
    <method name="ContextMenu"><arg name="x" type="i" direction="in"/><arg name="y" type="i" direction="in"/></method>
    <method name="Activate"><arg name="x" type="i" direction="in"/><arg name="y" type="i" direction="in"/></method>
    <method name="SecondaryActivate"><arg name="x" type="i" direction="in"/><arg name="y" type="i" direction="in"/></method>
    <method name="Scroll"><arg name="delta" type="i" direction="in"/><arg name="orientation" type="s" direction="in"/></method>
    <signal name="NewIcon"/>
    <signal name="NewStatus"><arg name="status" type="s"/></signal>
    <signal name="NewTitle"/>
  </interface>
</node>
"#;

/// glib -> GPUI
#[derive(Debug, Clone)]
pub enum Event {
    State(State),
    /// Tray icon activated (SNI Activate): toggle at/near this icon position.
    Show(Option<(i32, i32)>),
    /// CLI "show" socket: raise without toggling.
    Raise,
    /// Device just connected: show the popover without being asked (AirBuddy-style).
    AutoShow,
    /// The service did not answer: the UI says "服务离线" instead of stale values.
    Offline,
}

/// GPUI -> glib
#[derive(Debug, Clone)]
pub enum SvcCmd {
    Refresh,
    SetNoiseMode(u32),
    SetNoiseLevel(u32, u32),
    SetAudioBackend(String, String),
    Connect,
}

/// GPUI -> glib
#[derive(Debug, Clone, Copy)]
pub enum SniCmd {
    SetStatus(bool),
}

/// Forward a state update, and on the connect edge raise the popover
/// (AirBuddy-style "buds just connected" card) plus a low-battery notification.
fn emit(
    tx: &Sender<Event>,
    st: State,
    prev: &std::rc::Rc<std::cell::Cell<bool>>,
    low: &std::rc::Rc<std::cell::Cell<bool>>,
) {
    let now = st.connected || st.bluetooth_connected;
    if now && !prev.replace(now) {
        let _ = tx.send(Event::AutoShow);
    } else if !now {
        prev.set(false);
    }

    let min_pct = st
        .battery
        .iter()
        .flatten()
        .filter(|(pct, _)| *pct >= 0)
        .map(|(pct, _)| *pct)
        .min()
        .unwrap_or(100);
    let charging = st.battery.iter().flatten().any(|(_, ch)| *ch);
    if min_pct <= 20 && !charging && !low.get() {
        low.set(true);
        let _ = std::process::Command::new("notify-send")
            .args([
                "-a",
                "Xiaomi Buds 5",
                "-i",
                "audio-headphones",
                "耳机电量低",
                &format!("剩余 {min_pct}%，建议充电"),
            ])
            .spawn();
    } else if min_pct > 25 || charging {
        low.set(false);
    }

    let _ = tx.send(Event::State(st));
}

fn fetch_state(conn: &gio::DBusConnection) -> Option<State> {
    let reply = conn
        .call_sync(
            Some(BUS),
            PATH,
            "org.freedesktop.DBus.Properties",
            "GetAll",
            Some(&(IFACE,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            5000,
            gio::Cancellable::NONE,
        )
        .ok()?;
    let (dict,) = reply.get::<(HashMap<String, glib::Variant>,)>()?;
    Some(model::parse_state(&dict))
}

struct StatusNotifier {
    conn: gio::DBusConnection,
    status: Arc<Mutex<String>>,
}

// Custom AirPods-style earbuds icon, served via IconPixmap (raw ARGB, network
// byte order). Shared byte-for-byte with the GTK tray crate.
static ICON_24: &[u8] = include_bytes!("../../../shared/tray-24.argb");
static ICON_32: &[u8] = include_bytes!("../../../shared/tray-32.argb");
static ICON_48: &[u8] = include_bytes!("../../../shared/tray-48.argb");

fn icon_pixmaps() -> Vec<(i32, i32, Vec<u8>)> {
    [(24i32, ICON_24), (32, ICON_32), (48, ICON_48)]
        .iter()
        .map(|(s, d)| (*s, *s, d.to_vec()))
        .collect()
}

fn sni_prop(name: &str, status: &Arc<Mutex<String>>) -> glib::Variant {
    match name {
        "Category" => "Hardware".to_variant(),
        "Id" => "xiaomi-buds5-panel".to_variant(),
        "Title" => "Xiaomi Buds 5".to_variant(),
        "Status" => status.lock().unwrap().clone().to_variant(),
        "WindowId" => 0i32.to_variant(),
        "IconThemePath" => "".to_variant(),
        "Menu" => glib::variant::ObjectPath::try_from("/NO_DBUSMENU")
            .unwrap()
            .to_variant(),
        "ItemIsMenu" => false.to_variant(),
        "IconName" => "".to_variant(),
        "IconPixmap" => icon_pixmaps().to_variant(),
        "OverlayIconName" => "".to_variant(),
        "OverlayIconPixmap" => Vec::<(i32, i32, Vec<u8>)>::new().to_variant(),
        "AttentionIconName" => "".to_variant(),
        "AttentionIconPixmap" => Vec::<(i32, i32, Vec<u8>)>::new().to_variant(),
        "AttentionMovieName" => "".to_variant(),
        _ => "".to_variant(),
    }
}

fn register_watcher(conn: &gio::DBusConnection) -> Result<(), glib::Error> {
    // Register the unique name, not the well-known one: the extension keys items
    // by that id and keeps the old item for 500ms after its owner dies. Reusing
    // the well-known name makes a restart re-register into the dying item, which
    // then deletes the new icon. The unique name gives each process its own id.
    let Some(unique) = conn.unique_name() else {
        return Ok(());
    };
    conn.call_sync(
        Some("org.kde.StatusNotifierWatcher"),
        "/StatusNotifierWatcher",
        "org.kde.StatusNotifierWatcher",
        "RegisterStatusNotifierItem",
        Some(&(unique.as_str(),).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        3000,
        gio::Cancellable::NONE,
    )
    .map(|_| ())
}

impl StatusNotifier {
    fn new(
        conn: &gio::DBusConnection,
        ui_tx: Sender<Event>,
    ) -> Result<Arc<StatusNotifier>, glib::Error> {
        let node = gio::DBusNodeInfo::for_xml(SNI_XML)?;
        let iface = node.interfaces()[0].clone();
        let status = Arc::new(Mutex::new("Active".to_string()));
        // One object path only: exposing the ayatana path alongside it made
        // GNOME's appindicator extension list the same item twice, which shows
        // up as two tray icons.
        for path in ["/StatusNotifierItem"] {
            let tx = ui_tx.clone();
            let st_get = status.clone();
            conn.register_object(
                path,
                &iface,
                move |_c, _s, _p, _i, method, params, inv| {
                    if method == "Activate" || method == "SecondaryActivate" {
                        let (x, y) = params.get::<(i32, i32)>().unwrap_or((0, 0));
                        let _ = tx.send(Event::Show(Some((x, y))));
                        inv.return_value(None);
                    } else if method == "ContextMenu" || method == "Scroll" {
                        inv.return_value(None);
                    } else {
                        inv.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method);
                    }
                },
                move |_c, _s, _p, _i, prop| sni_prop(prop, &st_get),
                move |_c, _s, _p, _i, _prop, _v| false,
            )?;
        }
        // gio 0.18 exposes RequestName only; flags 4 = DO_NOT_QUEUE so a running
        // tray (or a second instance) makes this instance fail to own the name.
        let _ = conn.call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "RequestName",
            Some(&("org.kde.StatusNotifierItem-xiaomi-buds5", 4u32).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            3000,
            gio::Cancellable::NONE,
        );
        if let Err(e) = register_watcher(conn) {
            eprintln!("[xiaomi-buds5-panel] SNI registration failed: {e}");
        }
        Ok(Arc::new(StatusNotifier {
            conn: conn.clone(),
            status,
        }))
    }

    fn set_status(&self, s: &str) {
        if !matches!(s, "Active" | "Passive" | "NeedsAttention") {
            return;
        }
        let mut cur = self.status.lock().unwrap();
        if *cur == s {
            return;
        }
        *cur = s.to_string();
        drop(cur);
        let _ = self.conn.emit_signal(
            None,
            "/StatusNotifierItem",
            "org.kde.StatusNotifierItem",
            "NewStatus",
            Some(&(s,).to_variant()),
        );
    }
}

extern "C" {
    fn getuid() -> u32;
}

/// Listen on the CLI "show" socket so `xiaomi-buds5 panel show` (or the same
/// socket the old tray used) can raise the popover.
fn start_socket(tx: Sender<Event>) {
    use std::io::Read;
    let uid = unsafe { getuid() };
    let path = format!("/tmp/xiaomi-buds5-panel-{uid}.sock");
    let _ = std::fs::remove_file(&path);
    match std::os::unix::net::UnixListener::bind(&path) {
        Ok(listener) => {
            std::thread::spawn(move || {
                for mut stream in listener.incoming().flatten() {
                    let mut buf = [0u8; 32];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if String::from_utf8_lossy(&buf[..n]).trim() == "show" {
                        let _ = tx.send(Event::Raise);
                    }
                }
            });
        }
        Err(e) => eprintln!("[xiaomi-buds5-panel] socket bind failed: {e}"),
    }
}

/// Spawn the dedicated glib thread. Returns its join handle (kept so `main`
/// can stay alive as long as the thread runs).
pub fn spawn(
    ui_tx: Sender<Event>,
    svc_rx: Receiver<SvcCmd>,
    sni_rx: Receiver<SniCmd>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        {
            let conn = match gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("[xiaomi-buds5-panel] session bus unavailable: {e}");
                    return;
                }
            };

            let sni = match StatusNotifier::new(&conn, ui_tx.clone()) {
                Ok(s) => Some(s),
                Err(e) => {
                    eprintln!("[xiaomi-buds5-panel] SNI init failed: {e}");
                    None
                }
            };

            let prev_conn = std::rc::Rc::new(std::cell::Cell::new(false));
            let low_sent = std::rc::Rc::new(std::cell::Cell::new(false));

            match fetch_state(&conn) {
                Some(st) => {
                    eprintln!("[xiaomi-buds5-panel] initial state: {st:?}");
                    // Seed the edge detector: starting up while already connected
                    // must not pop the card (only a real connect does).
                    prev_conn.set(st.connected || st.bluetooth_connected);
                    emit(&ui_tx, st, &prev_conn, &low_sent);
                }
                None => {
                    eprintln!("[xiaomi-buds5-panel] service offline at startup");
                    let _ = ui_tx.send(Event::Offline);
                }
            }

            // StateChanged -> re-read full state and push to the UI.
            {
                let tx = ui_tx.clone();
                let c = conn.clone();
                let prev_conn = prev_conn.clone();
                let low_sent = low_sent.clone();
                conn.signal_subscribe(
                    Some(BUS),
                    Some(IFACE),
                    Some("StateChanged"),
                    Some(PATH),
                    None,
                    gio::DBusSignalFlags::NONE,
                    move |_c, _s, _p, _i, _sig, _params| match fetch_state(&c) {
                        Some(st) => emit(&tx, st, &prev_conn, &low_sent),
                        None => {
                            let _ = tx.send(Event::Offline);
                        }
                    },
                );
            }

            // 3s periodic refresh (covers missed signals / reconnect).
            {
                let tx = ui_tx.clone();
                let c = conn.clone();
                let prev_conn = prev_conn.clone();
                let low_sent = low_sent.clone();
                glib::timeout_add_seconds_local(3, move || {
                    match fetch_state(&c) {
                        Some(st) => emit(&tx, st, &prev_conn, &low_sent),
                        None => {
                            let _ = tx.send(Event::Offline);
                        }
                    }
                    glib::ControlFlow::Continue
                });
            }

            // Drain GPUI -> glib commands every 100ms.
            {
                let tx = ui_tx.clone();
                let c = conn.clone();
                let sni2 = sni.clone();
                let prev_conn = prev_conn.clone();
                let low_sent = low_sent.clone();
                glib::timeout_add_local(Duration::from_millis(100), move || {
                    while let Ok(cmd) = svc_rx.try_recv() {
                        match cmd {
                            SvcCmd::Refresh => {}
                            SvcCmd::SetNoiseMode(mode) => {
                                if let Err(e) = c.call_sync(
                                    Some(BUS),
                                    PATH,
                                    IFACE,
                                    "SetNoiseMode",
                                    Some(&(mode,).to_variant()),
                                    None,
                                    gio::DBusCallFlags::NONE,
                                    5000,
                                    gio::Cancellable::NONE,
                                ) {
                                    eprintln!("[xiaomi-buds5-panel] SetNoiseMode failed: {e}");
                                }
                            }
                            SvcCmd::SetNoiseLevel(mode, level) => {
                                if let Err(e) = c.call_sync(
                                    Some(BUS),
                                    PATH,
                                    IFACE,
                                    "SetNoiseLevel",
                                    Some(&(mode, level).to_variant()),
                                    None,
                                    gio::DBusCallFlags::NONE,
                                    5000,
                                    gio::Cancellable::NONE,
                                ) {
                                    eprintln!("[xiaomi-buds5-panel] SetNoiseLevel failed: {e}");
                                }
                            }
                            SvcCmd::SetAudioBackend(backend, codec) => {
                                // Backend/codec switch takes 5–15s on the device.
                                if let Err(e) = c.call_sync(
                                    Some(BUS),
                                    PATH,
                                    IFACE,
                                    "SetAudioBackend",
                                    Some(&(backend, codec).to_variant()),
                                    None,
                                    gio::DBusCallFlags::NONE,
                                    75000,
                                    gio::Cancellable::NONE,
                                ) {
                                    eprintln!("[xiaomi-buds5-panel] SetAudioBackend failed: {e}");
                                }
                            }
                            SvcCmd::Connect => {
                                if let Err(e) = c.call_sync(
                                    Some(BUS),
                                    PATH,
                                    IFACE,
                                    "Connect",
                                    None,
                                    None,
                                    gio::DBusCallFlags::NONE,
                                    5000,
                                    gio::Cancellable::NONE,
                                ) {
                                    eprintln!("[xiaomi-buds5-panel] Connect failed: {e}");
                                }
                            }
                        }
                        // Re-read after every command so the UI reflects whatever
                        // the device/service accepted (and clears any error).
                        if let Some(st) = fetch_state(&c) {
                            emit(&tx, st, &prev_conn, &low_sent);
                        }
                    }
                    while let Ok(cmd) = sni_rx.try_recv() {
                        match cmd {
                            SniCmd::SetStatus(active) => {
                                if let Some(s) = &sni2 {
                                    s.set_status(if active { "Active" } else { "Passive" });
                                }
                            }
                        }
                    }
                    glib::ControlFlow::Continue
                });
            }

            start_socket(ui_tx.clone());

            glib::MainLoop::new(None, false).run();
        }
    })
}
