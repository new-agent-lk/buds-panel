// GTK3 status tray + popover for the Xiaomi Buds 5 panel.
// Rust port of the former Python tray.py / sni.py; same D-Bus contract.
use glib::ToVariant;
use gtk::prelude::*;
use gtk::{cairo, gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::Command;
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};
use std::time::Duration;

extern "C" {
    fn getuid() -> u32;
}

const BUS: &str = "io.github.kang.XiaomiBuds5";
const PATH: &str = "/io/github/kang/XiaomiBuds5";
const IFACE: &str = "io.github.kang.XiaomiBuds5";
const MODES: [&str; 3] = ["关闭", "降噪", "通透"];
const LEVELS: [&str; 3] = ["舒适", "均衡", "深度"];

const CSS: &str = r#"
window { background: transparent; }
.buds-popover {
    background: #f5f5f7; color: #1d1d1f;
    border-radius: 22px;
    border: 1px solid rgba(0,0,0,0.10);
    padding: 16px 16px 12px 16px;
}
.buds-name { font-size: 18px; font-weight: 700; color: #1d1d1f; }
.buds-sub { font-size: 11px; color: #7a7a7e; }
.buds-close {
    background: rgba(0,0,0,0.06); color: #555;
    border-radius: 99px; min-width: 27px; min-height: 27px;
    padding: 0; font-size: 17px;
}
.buds-card { background: white; border-radius: 16px; padding: 18px; border: 1px solid rgba(0,0,0,0.055); }
.buds-hero { background: white; border-radius: 16px; padding: 20px 18px; }
.buds-hero-icon { color: #007aff; }
.buds-hero-title { font-size: 20px; font-weight: 700; }
.buds-hero-status { font-size: 12px; color: #6e6e73; }
.buds-battery-value { font-size: 19px; font-weight: 700; }
.buds-battery-label { font-size: 10px; color: #8e8e93; }
.buds-section { font-size: 12px; font-weight: 700; color: #1d1d1f; }
.buds-muted { color: #8e8e93; font-size: 11px; }
.buds-value { color: #6e6e73; font-size: 12px; }
.buds-seg { background: rgba(118,118,128,0.12); border-radius: 11px; padding: 4px; }
.buds-seg button { background: transparent; border: none; border-radius: 8px; padding: 9px 8px; color: #1d1d1f; font-size: 12px; }
.buds-seg button.selected { background: white; color: #007aff; font-weight: 700; box-shadow: 0 1px 3px rgba(0,0,0,0.16); }
.buds-codec-grid { background: rgba(118,118,128,0.10); border-radius: 12px; padding: 6px; }
.buds-codec { background: transparent; border: none; border-radius: 9px; padding: 10px 8px; font-size: 11px; }
.buds-codec.selected { background: white; color: #007aff; font-weight: 700; }
.buds-secondary { background: rgba(118,118,128,0.12); border: none; border-radius: 10px; padding: 8px 10px; color: #1d1d1f; }
scale trough { min-height: 7px; border-radius: 99px; background: rgba(118,118,128,0.18); }
scale highlight { border-radius: 99px; background: #007aff; }
scale slider { min-width: 16px; min-height: 16px; background: white; border-radius: 99px; box-shadow: 0 1px 3px rgba(0,0,0,0.25); }
.buds-hero { box-shadow: 0 1px 4px rgba(0,0,0,0.06); }
.buds-seg button { transition-property: background-color, color; transition-duration: 140ms; transition-timing-function: ease; }
.buds-seg button:hover { background-color: rgba(120,120,128,0.10); }
.buds-codec { transition-property: background-color, color; transition-duration: 140ms; transition-timing-function: ease; }
.buds-codec:hover { background-color: rgba(120,120,128,0.10); }
.buds-secondary { transition-property: background-color; transition-duration: 140ms; }
.buds-secondary:hover { background-color: rgba(118,118,128,0.22); }
.buds-close { transition-property: background-color; transition-duration: 140ms; }
.buds-close:hover { background-color: rgba(0,0,0,0.13); }
"#;

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

enum Msg {
    Show(Option<(i32, i32)>),
    Refresh,
    SwitchResult(Result<(), String>),
}
type Queue = Arc<Mutex<VecDeque<Msg>>>;

#[derive(Default, Clone)]
struct State {
    connected: bool,
    has_cache: bool,
    bluetooth_connected: bool,
    name: String,
    battery: [Option<(i32, bool)>; 3],
    noise_mode: u32,
    noise_level: u32,
    codec: String,
    audio_backend: String,
    last_error: String,
    codecs: Vec<(String, String)>,
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}
fn adaptive_runtime() -> String {
    format!(
        "{}/.local/share/xiaomi-buds5-panel/adaptive-runtime",
        home()
    )
}
fn adaptive_state() -> String {
    format!(
        "{}/.local/share/xiaomi-buds5-panel/state/adaptive.pid",
        home()
    )
}
fn has_adaptive() -> bool {
    Path::new(&adaptive_state()).exists()
}

fn pactl(args: &[&str]) -> String {
    let mut c = Command::new("pactl");
    c.args(args);
    if has_adaptive() {
        c.env(
            "PULSE_SERVER",
            format!("unix:{}/pulse/native", adaptive_runtime()),
        );
    }
    match c.output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => String::new(),
    }
}

/// The paired earbuds' MAC, from $XIAOMI_BUDS5_MAC or the first paired
/// buds/airdots device — no hardcoded address, so this works on any machine.
fn buds_address() -> Option<String> {
    if let Ok(a) = std::env::var("XIAOMI_BUDS5_MAC") {
        let a = a.trim();
        if !a.is_empty() {
            return Some(a.to_string());
        }
    }
    let out = Command::new("bluetoothctl")
        .args(["paired-devices"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut best: Option<(u8, String)> = None;
    for line in text.lines() {
        let parts: Vec<&str> = line.splitn(3, ' ').collect();
        if parts.len() != 3 || parts[0] != "Device" {
            continue;
        }
        let name = parts[2].to_lowercase();
        if !name.contains("buds") && !name.contains("airdots") {
            continue;
        }
        let score = if name.contains("buds 5") { 0 } else { 1 };
        match &best {
            Some((s, _)) if *s <= score => {}
            _ => best = Some((score, parts[1].to_string())),
        }
    }
    best.map(|(_, a)| a)
}

fn buds_connected() -> bool {
    let Some(mac) = buds_address() else {
        return false;
    };
    Command::new("bluetoothctl")
        .args(["info", &mac])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("Connected: yes"))
        .unwrap_or(false)
}

fn current_sink() -> String {
    for line in pactl(&["list", "short", "sinks"]).lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 && parts[1].starts_with("bluez_") {
            return parts[1].to_string();
        }
    }
    if let Some(mac) = buds_address() {
        let u = mac.replace(':', "_");
        return if has_adaptive() {
            format!("bluez_output.{u}.1")
        } else {
            format!("bluez_sink.{u}.a2dp_sink")
        };
    }
    "@DEFAULT_SINK@".to_string()
}

fn sink_volume_pct(sink: &str) -> i32 {
    for part in pactl(&["get-sink-volume", sink]).split_whitespace() {
        if let Some(p) = part.strip_suffix('%') {
            if let Ok(v) = p.parse::<i32>() {
                return v;
            }
        }
    }
    0
}

fn set_sink_volume(sink: &str, pct: i32) {
    let vol = format!("{}%", pct);
    let mut c = Command::new("pactl");
    c.args(["set-sink-volume", sink, &vol]);
    if has_adaptive() {
        c.env(
            "PULSE_SERVER",
            format!("unix:{}/pulse/native", adaptive_runtime()),
        );
    }
    let _ = c.status();
}

fn codec_label(name: &str) -> String {
    let label = match name {
        "sbc" => "SBC",
        "sbc_xq" => "SBC-XQ",
        "aac" => "AAC",
        "aptx" => "aptX Classic",
        "aptx_hd" => "aptX HD",
        "aptx_adaptive_hq" => "aptX Adaptive HQ",
        "aptx_adaptive" => "aptX Adaptive",
        "aptx_adaptive_ll" => "aptX Adaptive LL",
        "ldac_hq" => "LDAC HQ",
        "ldac_sq" => "LDAC SQ",
        "ldac_mq" => "LDAC MQ",
        _ => return name.to_string(),
    };
    label.to_string()
}

fn parse_state(dict: &HashMap<String, glib::Variant>) -> State {
    let gb = |k: &str| -> bool { dict.get(k).and_then(|v| v.get::<bool>()).unwrap_or(false) };
    let gu = |k: &str| -> u32 { dict.get(k).and_then(|v| v.get::<u32>()).unwrap_or(0) };
    let gs = |k: &str| -> String {
        dict.get(k)
            .and_then(|v| v.get::<String>())
            .unwrap_or_default()
    };
    let mut battery = [None, None, None];
    if let Some(bv) = dict
        .get("Battery")
        .and_then(|v| v.get::<HashMap<String, glib::Variant>>())
    {
        for (i, key) in ["left", "right", "case"].iter().enumerate() {
            battery[i] = bv.get(*key).and_then(|v| v.get::<(i32, bool)>());
        }
    }
    let codecs = dict
        .get("Codecs")
        .and_then(|v| v.get::<Vec<(String, String)>>())
        .unwrap_or_default();
    State {
        connected: gb("Connected"),
        has_cache: gb("HasCachedState"),
        bluetooth_connected: gb("BluetoothConnected"),
        name: gs("Name"),
        battery,
        noise_mode: gu("NoiseMode"),
        noise_level: gu("NoiseLevel"),
        codec: gs("Codec"),
        audio_backend: gs("AudioBackend"),
        last_error: gs("LastError"),
        codecs,
    }
}

fn label(text: &str, css: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.style_context().add_class(css);
    l
}

fn set_selected<W: IsA<gtk::Widget>>(w: &W, selected: bool) {
    let ctx = w.style_context();
    if selected {
        ctx.add_class("selected");
    } else {
        ctx.remove_class("selected");
    }
}

// ---- Hero icon: embedded SVG (pair of wireless earbuds) ----
fn buds_icon() -> gtk::Image {
    let loader = gdk_pixbuf::PixbufLoader::new();
    let _ = loader.write(include_bytes!("../assets/airpods.svg"));
    let _ = loader.close();
    let img = gtk::Image::from_pixbuf(loader.pixbuf().as_ref());
    img.set_pixel_size(52);
    img
}

// ---- Tray icon pixmaps: custom AirPods-style earbuds, served via IconPixmap ----
// Raw ARGB (network byte order) so the image is the same on any host/theme and
// needs no runtime SVG/PNG decoding. Shared with the GPUI panel crate.
static ICON_24: &[u8] = include_bytes!("../../shared/tray-24.argb");
static ICON_32: &[u8] = include_bytes!("../../shared/tray-32.argb");
static ICON_48: &[u8] = include_bytes!("../../shared/tray-48.argb");

fn icon_pixmaps() -> Vec<(i32, i32, Vec<u8>)> {
    [(24i32, ICON_24), (32, ICON_32), (48, ICON_48)]
        .iter()
        .map(|(s, d)| (*s, *s, d.to_vec()))
        .collect()
}

// ---- StatusNotifierItem server -------------------------------------------------

struct StatusNotifier {
    conn: gio::DBusConnection,
    status: Arc<Mutex<String>>,
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
    conn.call_sync(
        Some("org.kde.StatusNotifierWatcher"),
        "/StatusNotifierWatcher",
        "org.kde.StatusNotifierWatcher",
        "RegisterStatusNotifierItem",
        Some(&("org.kde.StatusNotifierItem-xiaomi-buds5",).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        3000,
        gio::Cancellable::NONE,
    )
    .map(|_| ())
}

impl StatusNotifier {
    fn new(conn: &gio::DBusConnection, queue: Queue) -> Result<Rc<StatusNotifier>, glib::Error> {
        let node = gio::DBusNodeInfo::for_xml(SNI_XML)?;
        let iface = node.interfaces()[0].clone();
        let status = Arc::new(Mutex::new("Active".to_string()));
        for path in [
            "/StatusNotifierItem",
            "/org/ayatana/NotificationItem/xiaomi_buds5_panel",
        ] {
            let q = queue.clone();
            let st_get = status.clone();
            conn.register_object(
                path,
                &iface,
                move |_c, _s, _p, _i, method, params, inv| {
                    if method == "Activate" || method == "SecondaryActivate" {
                        let (x, y) = params.get::<(i32, i32)>().unwrap_or((0, 0));
                        q.lock().unwrap().push_back(Msg::Show(Some((x, y))));
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
        // gio 0.18 does not expose bus_own_name_on_connection with closures
        // (the dbus module is private), so own the name via RequestName.
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
            eprintln!("[xiaomi-buds5-tray] SNI registration failed: {}", e);
        }
        Ok(Rc::new(StatusNotifier {
            conn: conn.clone(),
            status,
        }))
    }

    fn set_status(&self, s: &str) {
        if s != "Active" && s != "Passive" && s != "NeedsAttention" {
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

// ---- App ----------------------------------------------------------------------

struct App {
    window: gtk::Window,
    root: gtk::Box,
    title: gtk::Label,
    status: gtk::Label,
    hero_title: gtk::Label,
    hero_protocol: gtk::Label,
    hero_status: gtk::Label,
    battery_cells: HashMap<&'static str, gtk::Label>,
    case_note: gtk::Label,
    volume: gtk::Scale,
    volume_value: gtk::Label,
    noise_summary: gtk::Label,
    mode_buttons: Vec<gtk::Button>,
    level_buttons: Vec<gtk::Button>,
    level_box: gtk::Box,
    main_page: gtk::Box,
    settings_page: gtk::Box,
    backend_buttons: Vec<(&'static str, gtk::Button)>,
    codec_label: gtk::Label,
    codec_grid: gtk::Grid,
    header: gtk::EventBox,
    close_btn: gtk::Button,
    settings_btn: gtk::Button,
    reconnect_btn: gtk::Button,
    back_btn: gtk::Button,
    syncing: Cell<bool>,
    state: RefCell<State>,
    pending: RefCell<Option<(String, String)>>,
    anchor: RefCell<Option<(i32, i32)>>,
    conn: gio::DBusConnection,
    queue: Queue,
    me: RefCell<Weak<App>>,
    sni: RefCell<Option<Rc<StatusNotifier>>>,
}

impl App {
    fn this(&self) -> Rc<App> {
        self.me.borrow().upgrade().expect("app self ref")
    }

    fn new(conn: gio::DBusConnection, queue: Queue) -> Rc<App> {
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_decorated(false);
        window.set_resizable(true);
        window.set_skip_taskbar_hint(true);
        window.set_skip_pager_hint(true);
        window.set_keep_above(true);
        window.set_type_hint(gdk::WindowTypeHint::PopupMenu);
        window.set_app_paintable(true);
        if let Some(screen) = gtk::prelude::WidgetExt::screen(&window) {
            if let Some(visual) = screen.rgba_visual() {
                if screen.is_composited() {
                    window.set_visual(Some(&visual));
                }
            }
        }
        window.connect_draw(|_w, cr| {
            cr.set_operator(cairo::Operator::Source);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
            let _ = cr.paint();
            glib::Propagation::Proceed
        });

        let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        outer.set_size_request(410, -1);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.style_context().add_class("buds-popover");
        outer.pack_start(&root, false, false, 0);
        window.add(&outer);

        // Header (fixed across pages, also the drag handle)
        let header = gtk::EventBox::new();
        header.set_visible_window(false);
        let head_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title_box = gtk::Box::new(gtk::Orientation::Vertical, 1);
        let title = label("Xiaomi Buds 5", "buds-name");
        let status = label("正在连接…", "buds-sub");
        title_box.pack_start(&title, false, false, 0);
        title_box.pack_start(&status, false, false, 0);
        head_row.pack_start(&title_box, true, true, 0);
        let close_btn = gtk::Button::with_label("×");
        close_btn.set_relief(gtk::ReliefStyle::None);
        close_btn.style_context().add_class("buds-close");
        head_row.pack_end(&close_btn, false, false, 0);
        header.add(&head_row);
        header.set_margin_bottom(4);
        root.pack_start(&header, false, false, 0);

        // Page host
        let stack = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.pack_start(&stack, false, false, 0);

        // ---- main page ----
        let main_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let hero = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        hero.style_context().add_class("buds-hero");
        let hero_icon = buds_icon();
        hero.pack_start(&hero_icon, false, false, 0);
        let htext = gtk::Box::new(gtk::Orientation::Vertical, 3);
        let hero_title = label("Xiaomi Buds 5", "buds-hero-title");
        let hero_status = label("正在连接…", "buds-hero-status");
        let hero_protocol = label("协议：—", "buds-hero-status");
        htext.pack_start(&hero_title, false, false, 0);
        htext.pack_start(&hero_status, false, false, 0);
        htext.pack_start(&hero_protocol, false, false, 0);
        hero.pack_start(&htext, true, true, 0);
        main_page.pack_start(&hero, false, false, 0);

        let battery = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        battery.style_context().add_class("buds-card");
        let mut battery_cells: HashMap<&'static str, gtk::Label> = HashMap::new();
        for (key, caption) in [("left", "左耳"), ("right", "右耳"), ("case", "充电盒")] {
            let cell = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let value = label("—", "buds-battery-value");
            cell.pack_start(&value, false, false, 0);
            cell.pack_start(&label(caption, "buds-battery-label"), false, false, 0);
            battery.pack_start(&cell, true, true, 0);
            battery_cells.insert(key, value);
        }
        main_page.pack_start(&battery, false, false, 0);
        let case_note = label("充电盒未上报时会显示“未检测”", "buds-muted");
        case_note.set_margin_start(4);
        main_page.pack_start(&case_note, false, false, 0);

        let volume_card = gtk::Box::new(gtk::Orientation::Vertical, 11);
        volume_card.style_context().add_class("buds-card");
        let vrow = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        vrow.pack_start(&label("音量", "buds-section"), true, true, 0);
        let volume_value = label("0%", "buds-value");
        vrow.pack_end(&volume_value, false, false, 0);
        volume_card.pack_start(&vrow, false, false, 0);
        let scale_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        scale_row.pack_start(
            &gtk::Image::from_icon_name(Some("audio-volume-low-symbolic"), gtk::IconSize::Menu),
            false,
            false,
            0,
        );
        let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
        volume.set_draw_value(false);
        volume.set_hexpand(true);
        scale_row.pack_start(&volume, true, true, 0);
        scale_row.pack_start(
            &gtk::Image::from_icon_name(Some("audio-volume-high-symbolic"), gtk::IconSize::Menu),
            false,
            false,
            0,
        );
        volume_card.pack_start(&scale_row, false, false, 0);
        main_page.pack_start(&volume_card, false, false, 0);

        let noise = gtk::Box::new(gtk::Orientation::Vertical, 12);
        noise.style_context().add_class("buds-card");
        let noise_head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        noise_head.pack_start(&label("降噪控制", "buds-section"), true, true, 0);
        let noise_summary = label("关闭", "buds-muted");
        noise_head.pack_end(&noise_summary, false, false, 0);
        noise.pack_start(&noise_head, false, false, 0);
        let mode_buttons = segmented(&noise, &MODES);
        let level_box = gtk::Box::new(gtk::Orientation::Vertical, 9);
        let level_buttons = segmented(&level_box, &LEVELS);
        noise.pack_start(&level_box, false, false, 0);
        main_page.pack_start(&noise, false, false, 0);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let settings_btn = gtk::Button::with_label("声音设置");
        settings_btn.set_relief(gtk::ReliefStyle::None);
        settings_btn.style_context().add_class("buds-secondary");
        let reconnect_btn = gtk::Button::with_label("连接电脑");
        reconnect_btn.set_relief(gtk::ReliefStyle::None);
        reconnect_btn.style_context().add_class("buds-secondary");
        actions.pack_start(&settings_btn, true, true, 0);
        actions.pack_start(&reconnect_btn, true, true, 0);
        main_page.pack_start(&actions, false, false, 0);

        // ---- settings page ----
        let settings_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let shead = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let back_btn = gtk::Button::with_label("‹ 返回");
        back_btn.set_relief(gtk::ReliefStyle::None);
        back_btn.style_context().add_class("buds-secondary");
        shead.pack_start(&back_btn, false, false, 0);
        shead.pack_start(&label("声音设置", "buds-name"), true, true, 0);
        settings_page.pack_start(&shead, false, false, 0);

        let scard = gtk::Box::new(gtk::Orientation::Vertical, 12);
        scard.style_context().add_class("buds-card");
        scard.pack_start(&label("音频后端", "buds-section"), false, false, 0);
        let mut backend_buttons: Vec<(&'static str, gtk::Button)> = Vec::new();
        let backend_seg = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        backend_seg.style_context().add_class("buds-seg");
        for (key, text) in [("pulse", "兼容"), ("adaptive", "高质量")] {
            let button = gtk::Button::with_label(text);
            button.set_relief(gtk::ReliefStyle::None);
            button.set_hexpand(true);
            backend_seg.pack_start(&button, true, true, 0);
            backend_buttons.push((key, button));
        }
        scard.pack_start(&backend_seg, false, false, 0);
        let codec_label = label("蓝牙协议", "buds-section");
        scard.pack_start(&codec_label, false, false, 0);
        let codec_grid = gtk::Grid::new();
        codec_grid.set_column_spacing(5);
        codec_grid.set_row_spacing(5);
        codec_grid.set_column_homogeneous(true);
        codec_grid.style_context().add_class("buds-codec-grid");
        scard.pack_start(&codec_grid, false, false, 0);
        settings_page.pack_start(&scard, false, false, 0);

        stack.pack_start(&main_page, false, false, 0);
        stack.pack_start(&settings_page, false, false, 0);
        settings_page.hide();

        apply_css();

        let app = Rc::new(App {
            window,
            root,
            title,
            status,
            hero_title,
            hero_protocol,
            hero_status,
            battery_cells,
            case_note,
            volume,
            volume_value,
            noise_summary,
            mode_buttons,
            level_buttons,
            level_box,
            main_page,
            settings_page,
            backend_buttons,
            codec_label,
            codec_grid,
            header,
            close_btn,
            settings_btn,
            reconnect_btn,
            back_btn,
            syncing: Cell::new(false),
            state: RefCell::new(State::default()),
            pending: RefCell::new(None),
            anchor: RefCell::new(None),
            conn,
            queue,
            me: RefCell::new(Weak::new()),
            sni: RefCell::new(None),
        });
        *app.me.borrow_mut() = Rc::downgrade(&app);
        app
    }

    fn wire(self: &Rc<Self>) {
        let a = self.clone();
        self.window.connect_delete_event(move |_w, _e| {
            a.window.hide();
            glib::Propagation::Stop
        });
        let a = self.clone();
        self.close_btn.connect_clicked(move |_| a.window.hide());
        let a = self.clone();
        self.header.connect_button_press_event(move |_w, ev| {
            if ev.button() == 1 {
                let (x, y) = ev.root();
                a.window.begin_move_drag(1, x as i32, y as i32, ev.time());
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        let a = self.clone();
        self.volume
            .connect_value_changed(move |s| a.on_volume_changed(s));
        let a = self.clone();
        self.settings_btn
            .connect_clicked(move |_| a.show_settings(true));
        let a = self.clone();
        self.reconnect_btn.connect_clicked(move |_| {
            let _ = a.call("Connect", None, 5000);
        });
        let a = self.clone();
        self.back_btn
            .connect_clicked(move |_| a.show_settings(false));
        for (i, b) in self.mode_buttons.iter().enumerate() {
            let a = self.clone();
            b.connect_clicked(move |_| a.set_mode(i as i32));
        }
        for (i, b) in self.level_buttons.iter().enumerate() {
            let a = self.clone();
            b.connect_clicked(move |_| a.set_level(i as i32));
        }
        for (key, b) in &self.backend_buttons {
            let a = self.clone();
            let key = key.to_string();
            b.connect_clicked(move |_| a.set_backend(&key));
        }
    }

    fn set_sni_status(&self, active: bool) {
        if let Some(s) = self.sni.borrow().as_ref() {
            s.set_status(if active { "Active" } else { "Passive" });
        }
    }

    fn initial(&self) {
        self.set_sni_status(buds_connected());
        self.refresh();
    }

    fn dispatch(&self, msg: Msg) {
        match msg {
            Msg::Show(anchor) => self.show_if_buds(anchor),
            Msg::Refresh => self.refresh(),
            Msg::SwitchResult(Ok(())) => {
                self.refresh();
                self.resolve_pending();
            }
            Msg::SwitchResult(Err(e)) => {
                self.status.set_text(&format!("切换失败：{}", e));
                *self.pending.borrow_mut() = None;
            }
        }
    }

    fn tick(&self) {
        let available = buds_connected();
        self.set_sni_status(available);
        if !available {
            self.window.hide();
        } else if self.window.is_visible() {
            self.refresh();
            self.update_volume();
        }
    }

    fn call(
        &self,
        method: &str,
        params: Option<glib::Variant>,
        timeout_ms: i32,
    ) -> Result<(), String> {
        let timeout = if method == "SetAudioBackend" {
            75000
        } else {
            timeout_ms
        };
        match self.conn.call_sync(
            Some(BUS),
            PATH,
            IFACE,
            method,
            params.as_ref(),
            None,
            gio::DBusCallFlags::NONE,
            timeout,
            gio::Cancellable::NONE,
        ) {
            Ok(_) => {}
            Err(e) => {
                self.status.set_text(&format!("错误：{}", e));
                return Err(e.to_string());
            }
        }
        self.refresh();
        Ok(())
    }

    fn fetch_state(&self) -> Option<State> {
        let reply = self
            .conn
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
        Some(parse_state(&dict))
    }

    fn refresh(&self) {
        match self.fetch_state() {
            Some(state) => self.update(&state),
            None => self.status.set_text("服务离线"),
        }
    }

    fn update(&self, state: &State) {
        *self.state.borrow_mut() = state.clone();
        let connected = state.connected;
        let has_cache = state.has_cache;
        let name = if state.name.is_empty() {
            "Xiaomi Buds 5".to_string()
        } else {
            state.name.clone()
        };
        self.title.set_text(&name);
        self.hero_title.set_text(&name);
        let bt = state.bluetooth_connected || connected;
        if !connected && !has_cache {
            self.status.set_text(if bt {
                "蓝牙已连接 · 等待音频"
            } else {
                "未连接电脑"
            });
            self.hero_status.set_text(if bt {
                "手机可能在播放"
            } else {
                "未连接电脑"
            });
            for v in self.battery_cells.values() {
                v.set_text("—");
            }
            self.update_volume();
            self.refresh_codec(state);
            self.resolve_pending();
            return;
        }
        for (i, key) in ["left", "right", "case"].iter().enumerate() {
            let (pct, chg) = state.battery[i].unwrap_or((-1, false));
            let mut text = if pct < 0 {
                "未检测".to_string()
            } else {
                format!("{}%", pct)
            };
            if chg && pct >= 0 {
                text.push_str(" ↑");
            }
            if let Some(l) = self.battery_cells.get(key) {
                l.set_text(&text);
            }
        }
        let mode = state.noise_mode as usize;
        let level = state.noise_level as usize;
        let mode_text = MODES
            .get(mode)
            .map(|s| s.to_string())
            .unwrap_or_else(|| "未知".to_string());
        if connected {
            self.status.set_text(&format!("已连接 · {}", mode_text));
            self.hero_status
                .set_text(&format!("已连接 · {}", mode_text));
        } else if bt {
            self.status.set_text("蓝牙已连接 · 等待音频");
            self.hero_status.set_text("蓝牙已连接 · 手机可能在播放");
        } else {
            self.status.set_text("未连接电脑");
            self.hero_status.set_text("未连接电脑 · 手机可能在使用耳机");
        }
        self.noise_summary.set_text(&mode_text);
        self.update_protocol_badge(&state.codec);
        let case_pct = state.battery[2].map(|(p, _)| p).unwrap_or(-1);
        self.case_note.set_visible(case_pct < 0);
        self.syncing.set(true);
        for (i, b) in self.mode_buttons.iter().enumerate() {
            set_selected(b, i == mode);
        }
        for (i, b) in self.level_buttons.iter().enumerate() {
            set_selected(b, mode == 1 && i == level);
            b.set_sensitive(mode == 1);
        }
        self.syncing.set(false);
        self.level_box.set_visible(mode == 1);
        self.update_volume();
        self.refresh_codec(state);
        self.resolve_pending();
        let a = self.this();
        glib::idle_add_local(move || {
            a.resize_to_content();
            glib::ControlFlow::Break
        });
    }

    fn render_main(&self, state: &State) {
        let connected = state.connected;
        let has_cache = state.has_cache;
        let bt = state.bluetooth_connected || connected;
        for (i, key) in ["left", "right", "case"].iter().enumerate() {
            let (pct, chg) = state.battery[i].unwrap_or((-1, false));
            let mut text = if pct < 0 {
                "未检测".to_string()
            } else {
                format!("{}%", pct)
            };
            if chg && pct >= 0 {
                text.push_str(" ↑");
            }
            if let Some(l) = self.battery_cells.get(key) {
                l.set_text(&text);
            }
        }
        if !connected && !has_cache {
            self.status.set_text(if bt {
                "蓝牙已连接 · 等待音频"
            } else {
                "未连接电脑"
            });
            self.hero_status.set_text(if bt {
                "手机可能在播放"
            } else {
                "未连接电脑"
            });
            return;
        }
        let mode = state.noise_mode as usize;
        let mode_text = MODES
            .get(mode)
            .map(|s| s.to_string())
            .unwrap_or_else(|| "未知".to_string());
        if connected {
            self.status.set_text(&format!("已连接 · {}", mode_text));
            self.hero_status
                .set_text(&format!("已连接 · {}", mode_text));
        } else if bt {
            self.status.set_text("蓝牙已连接 · 等待音频");
            self.hero_status.set_text("手机可能在播放");
        } else {
            self.status.set_text("未连接电脑");
            self.hero_status.set_text("未连接电脑 · 手机可能在使用耳机");
        }
        self.noise_summary.set_text(&mode_text);
        self.update_protocol_badge(&state.codec);
    }

    fn update_protocol_badge(&self, codec: &str) {
        if codec.is_empty() {
            self.hero_protocol.set_text("协议：读取中…");
            return;
        }
        let mut text = codec_label(codec);
        if codec.starts_with("aptx_adaptive") {
            text.push_str(" · 48 kHz/24-bit");
        }
        self.hero_protocol.set_text(&format!("协议：{}", text));
    }

    fn refresh_codec(&self, state: &State) {
        self.set_codec_ui(&state.codec, &state.codecs);
    }

    fn set_codec_ui(&self, current: &str, codecs: &[(String, String)]) {
        self.syncing.set(true);
        for child in self.codec_grid.children() {
            unsafe {
                child.destroy();
            }
        }
        for (index, (name, desc)) in codecs.iter().enumerate() {
            let text = if codec_label(name) == *name && !desc.is_empty() {
                desc.clone()
            } else {
                codec_label(name)
            };
            let button = gtk::Button::with_label(&text);
            button.set_relief(gtk::ReliefStyle::None);
            button.set_hexpand(true);
            button.style_context().add_class("buds-codec");
            if name == current {
                button.style_context().add_class("selected");
            }
            let a = self.this();
            let n = name.clone();
            button.connect_clicked(move |_| a.codec_changed(&n));
            self.codec_grid
                .attach(&button, (index % 2) as i32, (index / 2) as i32, 1, 1);
        }
        let adaptive = has_adaptive();
        for (key, button) in &self.backend_buttons {
            set_selected(button, (*key == "adaptive") == adaptive);
        }
        let cur_label = if current.is_empty() {
            "—".to_string()
        } else {
            codec_label(current)
        };
        self.codec_label
            .set_text(&format!("蓝牙协议 · {}", cur_label));
        self.codec_grid.show_all();
        self.syncing.set(false);
    }

    fn resolve_pending(&self) {
        let pending = self.pending.borrow().clone();
        let Some((kind, target)) = pending else {
            return;
        };
        let state = self.state.borrow().clone();
        let current = if kind == "codec" {
            state.codec.clone()
        } else {
            state.audio_backend.clone()
        };
        if !state.last_error.is_empty() {
            self.status
                .set_text(&format!("切换失败：{}", state.last_error));
            *self.pending.borrow_mut() = None;
        } else if current == target {
            let text = if kind == "codec" {
                codec_label(&target)
            } else if target == "adaptive" {
                "aptX Adaptive".to_string()
            } else {
                "PulseAudio".to_string()
            };
            self.status.set_text(&format!("已切换到 {}", text));
            *self.pending.borrow_mut() = None;
        }
    }

    fn set_mode(&self, mode: i32) {
        if self.syncing.get() {
            return;
        }
        let _ = self.call("SetNoiseMode", Some((mode as u32,).to_variant()), 5000);
    }

    fn set_level(&self, level: i32) {
        if self.syncing.get() {
            return;
        }
        let _ = self.call(
            "SetNoiseLevel",
            Some((1u32, level as u32).to_variant()),
            5000,
        );
    }

    fn set_backend(&self, backend: &str) {
        if self.state.borrow().audio_backend == backend {
            self.status.set_text("已是该音频后端");
            return;
        }
        *self.pending.borrow_mut() = Some(("backend".to_string(), backend.to_string()));
        self.status.set_text("正在切换音频后端，可能需要 5–15 秒…");
        self.spawn_switch(backend, "");
    }

    fn codec_changed(&self, codec: &str) {
        let backend = {
            let b = self.state.borrow().audio_backend.clone();
            if b.is_empty() {
                "pulse".to_string()
            } else {
                b
            }
        };
        if self.state.borrow().codec == codec {
            self.status
                .set_text(&format!("已是 {}", codec_label(codec)));
            return;
        }
        *self.pending.borrow_mut() = Some(("codec".to_string(), codec.to_string()));
        self.status
            .set_text(&format!("正在切换编码：{}…", codec_label(codec)));
        self.spawn_switch(&backend, codec);
    }

    fn spawn_switch(&self, backend: &str, codec: &str) {
        let queue = self.queue.clone();
        let b = backend.to_string();
        let c = codec.to_string();
        std::thread::spawn(move || {
            let res = (|| -> Result<(), String> {
                let conn = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
                    .map_err(|e| e.to_string())?;
                conn.call_sync(
                    Some(BUS),
                    PATH,
                    IFACE,
                    "SetAudioBackend",
                    Some(&(b, c).to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    75000,
                    gio::Cancellable::NONE,
                )
                .map_err(|e| e.to_string())?;
                Ok(())
            })();
            queue.lock().unwrap().push_back(Msg::SwitchResult(res));
        });
    }

    fn on_volume_changed(&self, scale: &gtk::Scale) {
        if self.syncing.get() {
            return;
        }
        let value = scale.value() as i32;
        self.volume_value.set_text(&format!("{}%", value));
        set_sink_volume(&current_sink(), value);
    }

    fn update_volume(&self) {
        let value = sink_volume_pct(&current_sink());
        self.syncing.set(true);
        self.volume.set_value(value as f64);
        self.syncing.set(false);
        self.volume_value.set_text(&format!("{}%", value));
    }

    fn show_settings(&self, show: bool) {
        if show {
            self.main_page.hide();
            self.settings_page.show_all();
        } else {
            self.settings_page.hide();
            self.main_page.show_all();
            let state = self.state.borrow().clone();
            self.render_main(&state);
        }
        let a = self.this();
        glib::idle_add_local(move || {
            a.resize_to_content();
            glib::ControlFlow::Break
        });
        for ms in [60u64, 180, 350] {
            let a = self.this();
            glib::timeout_add_local(Duration::from_millis(ms), move || {
                a.resize_to_content();
                glib::ControlFlow::Break
            });
        }
    }

    fn pointer_position(&self) -> (i32, i32) {
        gdk::Display::default()
            .and_then(|d| d.default_seat())
            .and_then(|s| s.pointer())
            .map(|dev| {
                let (_sc, x, y) = dev.position();
                (x, y)
            })
            .unwrap_or((0, 0))
    }

    fn valid_anchor(&self, anchor: Option<(i32, i32)>) -> Option<(i32, i32)> {
        let (x, y) = anchor?;
        if x <= 0 && y <= 0 {
            return None;
        }
        let display = gdk::Display::default()?;
        for i in 0..display.n_monitors() {
            if let Some(mon) = display.monitor(i) {
                let g = mon.geometry();
                if g.x() <= x && x < g.x() + g.width() && g.y() <= y && y < g.y() + g.height() {
                    return Some((x, y));
                }
            }
        }
        None
    }

    fn position_popup(&self, anchor: Option<(i32, i32)>) {
        let anchor = self.valid_anchor(anchor);
        let display = match gdk::Display::default() {
            Some(d) => d,
            None => return,
        };
        let mon = anchor
            .and_then(|(x, y)| display.monitor_at_point(x, y))
            .or_else(|| {
                let (x, y) = self.pointer_position();
                display.monitor_at_point(x, y)
            })
            .or_else(|| display.monitor(0));
        let Some(mon) = mon else { return };
        let geo = mon.geometry();
        let work = mon.workarea();
        let width = 410;
        let height = self.window.preferred_height().1.min(720);
        self.window.resize(width, height);
        let (ax, ay) = anchor.unwrap_or_else(|| self.pointer_position());
        let mut x = (ax - width / 2)
            .max(work.x() + 12)
            .min(work.x() + work.width() - width - 12);
        let below = work.y() + work.height() - ay;
        let above = ay - work.y();
        let mut y = if below >= height + 24 || below >= above {
            (ay + 12).min(work.y() + work.height() - height - 12)
        } else {
            (ay - height - 12).max(work.y() + 12)
        };
        x = x.max(geo.x() + 8).min(geo.x() + geo.width() - width - 8);
        y = y.max(geo.y() + 8).min(geo.y() + geo.height() - height - 8);
        self.window.move_(x, y);
        *self.anchor.borrow_mut() = anchor;
    }

    fn resize_to_content(&self) {
        if !self.window.is_visible() {
            return;
        }
        let (_, preferred_h) = self.root.preferred_height();
        let allocated_h = self.root.allocation().height();
        let card_h = preferred_h.max(allocated_h);
        let total = (card_h + 12).min(780);
        self.window.resize(430, total);
        self.window.set_size_request(430, total);
        let anchor = *self.anchor.borrow();
        self.position_popup(anchor);
    }

    fn fade_in(&self) {
        let root = self.root.clone();
        root.set_opacity(0.0);
        let frame = Rc::new(Cell::new(0i32));
        glib::timeout_add_local(Duration::from_millis(16), move || {
            let f = frame.get() + 1;
            frame.set(f);
            let t = (f as f64 / 13.0).min(1.0);
            let eased = 1.0 - (1.0 - t) * (1.0 - t); // ease-out
            root.set_opacity(eased);
            if f >= 13 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    fn show_near_titlebar(&self, anchor: Option<(i32, i32)>) {
        self.window.show_all();
        self.show_settings(false);
        *self.anchor.borrow_mut() = anchor;
        self.position_popup(anchor);
        self.window.present();
        self.fade_in();
        let a = self.this();
        glib::idle_add_local(move || {
            a.resize_to_content();
            glib::ControlFlow::Break
        });
        let a = self.this();
        glib::timeout_add_local(Duration::from_millis(120), move || {
            a.resize_to_content();
            glib::ControlFlow::Break
        });
    }

    fn show_if_buds(&self, anchor: Option<(i32, i32)>) {
        if !buds_connected() {
            self.window.hide();
            self.set_sni_status(false);
            return;
        }
        self.set_sni_status(true);
        // Ask the daemon to re-read the device now; fresh values arrive via
        // StateChanged, so the panel is not stuck on the cached snapshot.
        let _ = self.call("Refresh", None, 2000);
        self.refresh();
        self.show_near_titlebar(anchor);
    }
}

fn segmented(parent: &gtk::Box, names: &[&str]) -> Vec<gtk::Button> {
    let seg = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    seg.style_context().add_class("buds-seg");
    let mut buttons = Vec::new();
    for name in names {
        let button = gtk::Button::with_label(name);
        button.set_relief(gtk::ReliefStyle::None);
        button.set_hexpand(true);
        seg.pack_start(&button, true, true, 0);
        buttons.push(button);
    }
    parent.pack_start(&seg, false, false, 0);
    buttons
}

fn apply_css() {
    let provider = gtk::CssProvider::new();
    let _ = provider.load_from_data(CSS.as_bytes());
    if let Some(screen) = gdk::Screen::default() {
        gtk::StyleContext::add_provider_for_screen(
            &screen,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn start_socket_listener(queue: Queue) {
    let uid = unsafe { getuid() };
    let path = format!("/tmp/xiaomi-buds5-panel-{}.sock", uid);
    let _ = std::fs::remove_file(&path);
    match UnixListener::bind(&path) {
        Ok(listener) => {
            std::thread::spawn(move || {
                use std::io::Read;
                for mut c in listener.incoming().flatten() {
                    let mut buf = [0u8; 32];
                    let n = c.read(&mut buf).unwrap_or(0);
                    if String::from_utf8_lossy(&buf[..n]).trim() == "show" {
                        queue.lock().unwrap().push_back(Msg::Show(None));
                    }
                }
            });
        }
        Err(e) => eprintln!("[xiaomi-buds5-tray] socket bind failed: {}", e),
    }
}

fn main() {
    if gtk::init().is_err() {
        std::process::exit(1);
    }
    let conn = match gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[xiaomi-buds5-tray] session bus unavailable: {}", e);
            std::process::exit(1);
        }
    };
    let queue: Queue = Arc::new(Mutex::new(VecDeque::new()));

    let app = App::new(conn.clone(), queue.clone());
    match StatusNotifier::new(&conn, queue.clone()) {
        Ok(sni) => *app.sni.borrow_mut() = Some(sni),
        Err(e) => eprintln!("[xiaomi-buds5-tray] SNI init failed: {}", e),
    }
    app.wire();
    app.initial();

    let q_sig = queue.clone();
    conn.signal_subscribe(
        Some(BUS),
        Some(IFACE),
        Some("StateChanged"),
        Some(PATH),
        None,
        gio::DBusSignalFlags::NONE,
        move |_c, _s, _p, _i, _sig, _params| {
            q_sig.lock().unwrap().push_back(Msg::Refresh);
        },
    );

    let q_poll = queue.clone();
    let weak = Rc::downgrade(&app);
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let msgs: Vec<Msg> = {
            let mut q = q_poll.lock().unwrap();
            q.drain(..).collect()
        };
        if let Some(app) = weak.upgrade() {
            for m in msgs {
                app.dispatch(m);
            }
        }
        glib::ControlFlow::Continue
    });

    let weak2 = Rc::downgrade(&app);
    glib::timeout_add_seconds_local(3, move || {
        if let Some(app) = weak2.upgrade() {
            app.tick();
        }
        glib::ControlFlow::Continue
    });

    start_socket_listener(queue.clone());

    gtk::main();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_state_reads_battery_and_codecs() {
        let mut dict: HashMap<String, glib::Variant> = HashMap::new();
        dict.insert("Connected".into(), true.to_variant());
        dict.insert("Name".into(), "Xiaomi Buds 5".to_variant());
        dict.insert("NoiseMode".into(), 1u32.to_variant());
        dict.insert("NoiseLevel".into(), 2u32.to_variant());
        dict.insert("Codec".into(), "aptx_adaptive_ll".to_variant());
        dict.insert("AudioBackend".into(), "adaptive".to_variant());
        let mut batt: HashMap<String, glib::Variant> = HashMap::new();
        batt.insert("left".into(), (80i32, false).to_variant());
        batt.insert("right".into(), (82i32, true).to_variant());
        batt.insert("case".into(), (-1i32, false).to_variant());
        dict.insert("Battery".into(), batt.to_variant());
        let codecs: Vec<(String, String)> = vec![
            ("sbc".into(), "SBC".into()),
            ("aptx_adaptive_ll".into(), "aptX Adaptive LL".into()),
        ];
        dict.insert("Codecs".into(), codecs.to_variant());

        let s = parse_state(&dict);
        assert!(s.connected);
        assert_eq!(s.noise_mode, 1);
        assert_eq!(s.noise_level, 2);
        assert_eq!(s.codec, "aptx_adaptive_ll");
        assert_eq!(s.battery[0], Some((80, false)));
        assert_eq!(s.battery[1], Some((82, true)));
        assert_eq!(s.battery[2], Some((-1, false)));
        assert_eq!(s.codecs.len(), 2);
    }

    #[test]
    fn codec_label_known_and_fallback() {
        assert_eq!(codec_label("sbc"), "SBC");
        assert_eq!(codec_label("aptx"), "aptX Classic");
        assert_eq!(codec_label("mystery"), "mystery");
    }
}
