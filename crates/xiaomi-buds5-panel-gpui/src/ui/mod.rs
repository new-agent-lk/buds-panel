//! The popover skeleton. P1 renders the first screen from live State
//! (battery / noise / volume) in an AirPods-style sheet: a #f5f5f7 rounded
//! card holding white sub-cards. Positioning, show/hide animation and the
//! remaining pages come in later phases.
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use flume::{Receiver, Sender};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};

use crate::dbus::{Event, SniCmd, SvcCmd};
use crate::model::{self, State, LEVELS, MODES};

// AirPods palette (see crates/xiaomi-buds5-tray/src/main.rs CSS).
const BG: u32 = 0xf5f5f7;
const CARD: u32 = 0xffffff;
const INK: u32 = 0x1d1d1f;
const SUB: u32 = 0x6e6e73;
const MUTED: u32 = 0x8e8e93;
const ACCENT: u32 = 0x007aff;
const SEG_TRACK: u32 = 0x76768020; // rgba(118,118,128,0.12)
const HAIRLINE: u32 = 0x0000000e; // rgba(0,0,0,0.055)

/// Chip click handler: index of the picked option, plus the app context.
type PickFn = Rc<dyn Fn(usize, &mut App)>;

/// Which page the popover is showing.
#[derive(Clone, Copy, PartialEq)]
enum Page {
    Main,
    Settings,
}

pub struct Panel {
    state: State,
    volume_pct: i32,
    volume: Entity<SliderState>,
    svc_tx: Sender<SvcCmd>,
    sni_tx: Sender<SniCmd>,
    page: Page,
    /// Bumped on every Show so the fade-in animation replays.
    fade_gen: u64,
    /// Whether the popover is currently shown.
    visible: bool,
    /// A show that still needs its X move applied (retried by the poll loop).
    pending_pos: Option<(i32, i32)>,
    /// When the current Show started (for the pointer-leave grace period).
    show_ts: std::time::Instant,
    /// Last tray-icon position we anchored to (reused for auto-shows).
    last_pos: Option<(i32, i32)>,
    /// Root-coordinate rect (x, y, w, h) the popover occupies.
    rect: (i32, i32, i32, i32),
    /// Whether the pointer has been inside the popover since it was shown.
    pointer_inside: bool,
    /// Keep the current microphone as the default input (AirBuddy-style lock).
    mic_lock: bool,
    /// The source we pin while the lock is on.
    mic_source: Option<String>,
    /// Backend/codec switch awaiting confirmation: (is_codec, target, label).
    pending: Option<(bool, String, String)>,
    /// One-line result of the last action: (text, is_error).
    notice: Option<(String, bool)>,
    /// The service did not answer the last refresh.
    offline: bool,
    /// Last `LastError` already reported, so a sticky error is not re-shown.
    last_error_seen: String,
    /// When the slider last set the sink itself; external reads are ignored for
    /// a moment after, so a drag or click does not flicker back to the old value.
    vol_local_ts: std::time::Instant,
}

impl Panel {
    pub fn new(
        cx: &mut Context<Self>,
        ui_rx: Receiver<Event>,
        svc_tx: Sender<SvcCmd>,
        sni_tx: Sender<SniCmd>,
    ) -> Self {
        // Popover behaviour is polled so pointer-leave and retried moves share
        // one clock. The UI channel is drained in the same tick: a burst of
        // Activate events must toggle in arrival order, not race a second task.
        cx.spawn(async move |weak: WeakEntity<Panel>, cx: &mut AsyncApp| {
            let mut tick: u32 = 0;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                tick = tick.wrapping_add(1);
                let res = weak.update_in(cx, |this, window, cx| {
                    while let Ok(ev) = ui_rx.try_recv() {
                        this.handle_event(ev, window, cx);
                    }
                    if this.pending_pos.is_some() {
                        this.apply_show(window, cx);
                    }
                    if !this.visible {
                        return false;
                    }
                    let elapsed = this.show_ts.elapsed();
                    let inside = pointer_pos().is_some_and(|(px, py)| {
                        let (x, y, w, h) = this.rect;
                        px >= x && px < x + w && py >= y && py < y + h
                    });
                    if inside {
                        this.pointer_inside = true;
                    }
                    // The window never gets focus under override-redirect, so
                    // dismissal is pointer-leave or a second tray activation
                    // (toggle in handle_event).
                    if this.pointer_inside && !inside && elapsed >= Duration::from_millis(300) {
                        this.hide(window, cx);
                    }
                    this.visible
                });
                let Ok(visible) = res else {
                    break;
                };
                // Media keys change the sink behind our back; the slider only
                // shows what pactl reports, so re-read while the panel is open.
                if visible && tick.is_multiple_of(4) {
                    let vol = cx
                        .background_executor()
                        .spawn(async { model::read_volume_pct() })
                        .await;
                    let _ = weak.update_in(cx, |this, window, cx| {
                        if this.visible
                            && this.vol_local_ts.elapsed() >= Duration::from_millis(400)
                            && vol != this.volume_pct
                        {
                            this.volume_pct = vol;
                            this.volume
                                .update(cx, |st, cx| st.set_value(vol as f32, window, cx));
                            cx.notify();
                        }
                    });
                }
            }
        })
        .detach();

        // Volume slider: dragging updates the thumb instantly; a worker thread
        // coalesces the burst of change events into a single `pactl` call so the
        // drag stays smooth (spawning pactl per pixel would stutter).
        let volume = cx.new(|_| SliderState::new().min(0.).max(100.).step(1.));
        let (vol_tx, vol_rx) = flume::unbounded::<i32>();
        std::thread::spawn(move || {
            while let Ok(mut v) = vol_rx.recv() {
                while let Ok(nv) = vol_rx.try_recv() {
                    v = nv;
                }
                std::thread::sleep(Duration::from_millis(25));
                while let Ok(nv) = vol_rx.try_recv() {
                    v = nv;
                }
                model::set_volume_pct(v);
            }
        });
        cx.subscribe(&volume, {
            let vol_tx = vol_tx.clone();
            move |this: &mut Panel,
                  _entity: Entity<SliderState>,
                  ev: &SliderEvent,
                  cx: &mut Context<Panel>| {
                let v = match ev {
                    SliderEvent::Change(v) | SliderEvent::Release(v) => v.start(),
                };
                this.vol_local_ts = std::time::Instant::now();
                this.volume_pct = v.round() as i32;
                let _ = vol_tx.send(this.volume_pct);
                cx.notify();
            }
        })
        .detach();

        // Initial volume read (pactl is a subprocess: keep it off the UI thread).
        cx.spawn(async move |weak, cx| {
            let vol = cx
                .background_executor()
                .spawn(async { model::read_volume_pct() })
                .await;
            weak.update_in(cx, |this, window, cx| {
                this.volume_pct = vol;
                this.volume
                    .update(cx, |st, cx| st.set_value(vol as f32, window, cx));
                cx.notify();
            })
            .ok();
        })
        .detach();

        // The window is created override-redirect and parked off-screen by
        // main.rs; it stays mapped for the whole process lifetime.
        Panel {
            state: State::default(),
            volume_pct: 0,
            volume,
            svc_tx,
            sni_tx,
            page: Page::Main,
            fade_gen: 0,
            visible: false,
            pending_pos: None,
            show_ts: std::time::Instant::now(),
            last_pos: None,
            rect: (0, 0, 0, 0),
            pointer_inside: false,
            mic_lock: false,
            mic_source: None,
            pending: None,
            notice: None,
            offline: false,
            last_error_seen: String::new(),
            vol_local_ts: std::time::Instant::now(),
        }
    }

    fn handle_event(&mut self, ev: Event, window: &mut Window, cx: &mut Context<Self>) {
        match ev {
            Event::State(s) => {
                let connected = s.connected || s.bluetooth_connected;
                self.offline = false;
                // `LastError` stays set until the next successful command, so
                // report each new error once instead of on every 3s refresh.
                if s.last_error != self.last_error_seen {
                    self.last_error_seen = s.last_error.clone();
                    if !s.last_error.is_empty() {
                        self.pending = None;
                        self.notice = Some((format!("切换失败：{}", s.last_error), true));
                    }
                }
                self.resolve_pending(&s);
                self.state = s;
                if self.mic_lock && connected {
                    if let Some(src) = self.mic_source.clone() {
                        cx.background_executor()
                            .spawn(async move { model::enforce_default_source(&src) })
                            .detach();
                    }
                }
                cx.notify();
            }
            Event::Show(pos) => {
                if self.visible {
                    self.hide(window, cx);
                } else {
                    // The appindicator extension sends icon-local coordinates
                    // (a few pixels), so fall back to the pointer, which is on
                    // the icon at click time.
                    let pos = pos
                        .filter(|(x, y)| *x > 40 || *y > 40)
                        .or_else(pointer_pos)
                        .or(self.last_pos);
                    self.show_at(pos, window, cx);
                }
            }
            Event::Raise => {
                self.show_at(pointer_pos().or(self.last_pos), window, cx);
            }
            Event::AutoShow => {
                let pos = self.last_pos.or_else(|| {
                    let (sw, _) = screen_size();
                    Some((sw - 240, 30))
                });
                self.show_at(pos, window, cx);
            }
            Event::Offline => {
                self.offline = true;
                cx.notify();
            }
        }
    }

    /// Start a backend/codec switch and say so at once: the D-Bus call blocks
    /// the glib thread for up to 75s, so the UI cannot wait for it to return.
    fn begin_switch(&mut self, is_codec: bool, target: &str, label: &str, cx: &mut Context<Self>) {
        let cur_backend = if self.state.audio_backend.is_empty() {
            "pulse".to_string()
        } else {
            self.state.audio_backend.clone()
        };
        let (same, backend, codec) = if is_codec {
            (self.state.codec == target, cur_backend, target.to_string())
        } else {
            (cur_backend == target, target.to_string(), String::new())
        };
        if same {
            self.notice = Some((format!("已是 {label}"), false));
            cx.notify();
            return;
        }
        self.pending = Some((is_codec, target.to_string(), label.to_string()));
        self.notice = Some((
            if is_codec {
                format!("正在切换编码：{label}…")
            } else {
                "正在切换音频后端，可能需要 5–15 秒…".to_string()
            },
            false,
        ));
        let _ = self.svc_tx.send(SvcCmd::SetAudioBackend(backend, codec));
        cx.notify();
    }

    /// Confirm a pending switch once the service reports the new value.
    fn resolve_pending(&mut self, s: &State) {
        let Some((is_codec, target, label)) = self.pending.clone() else {
            return;
        };
        let cur = if is_codec {
            s.codec.clone()
        } else if s.audio_backend.is_empty() {
            "pulse".to_string()
        } else {
            s.audio_backend.clone()
        };
        if cur == target {
            self.pending = None;
            self.notice = Some((format!("已切换到 {label}"), false));
        }
    }

    fn show_at(&mut self, pos: Option<(i32, i32)>, window: &mut Window, cx: &mut Context<Self>) {
        let pos = pos.or(self.last_pos).or_else(|| {
            let (sw, _) = screen_size();
            Some((sw - 240, 30))
        });
        if let Some(pos) = pos {
            self.last_pos = Some(pos);
            self.pending_pos = Some(pos);
            self.show_ts = std::time::Instant::now();
            self.apply_show(window, cx);
        }
    }

    /// Move the window on screen; keep retrying from the poll loop if X is
    /// briefly unavailable, but do not report it shown until the move stuck.
    fn apply_show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((x, y)) = self.pending_pos else {
            return;
        };
        let Some(rect) = reposition(window, x, y) else {
            if self.show_ts.elapsed() >= Duration::from_millis(1200) {
                self.pending_pos = None;
                self.visible = false;
            }
            return;
        };
        self.rect = rect;
        self.pending_pos = None;
        self.visible = true;
        self.pointer_inside = false;
        self.show_ts = std::time::Instant::now();
        self.fade_gen = self.fade_gen.wrapping_add(1);
        window.activate_window();
        cx.notify();
        let _ = self.sni_tx.send(SniCmd::SetStatus(true));
    }

    fn hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.visible = false;
        self.pending_pos = None;
        self.pointer_inside = false;
        // Do not resurrect a "正在切换…" from ten minutes ago on the next open.
        self.notice = None;
        park(window);
        cx.notify();
    }

    /// One-line feedback for the last action; empty when there is nothing to say.
    fn notice_line(&self) -> Div {
        let Some((msg, err)) = &self.notice else {
            return div();
        };
        div().w_full().px(px(4.)).child(text(
            msg.clone(),
            10.,
            if *err { 0xd70015 } else { SUB },
            false,
        ))
    }

    /// The in-popover settings page: audio backend + Bluetooth codec selection.
    fn render_settings(&self, cx: &mut Context<Self>) -> Div {
        let s = &self.state;
        let backend = if s.audio_backend == "adaptive" { 1 } else { 0 };

        let back = div()
            .id("back")
            .h(px(24.))
            .px(px(10.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .bg(rgba(SEG_TRACK))
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                this.page = Page::Main;
                cx.notify();
            }))
            .child(text("‹ 返回", 12., INK, false));
        let head = div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(back)
            .child(text("声音设置", 16., INK, true));

        // Backend switch: empty codec means "keep current codec" in the service
        // (matches the GTK tray, which sends "" when only the backend changes).
        let me = cx.entity();
        let me_backend = me.clone();
        let backend_pick: PickFn = Rc::new(move |i, cx| {
            let (target, label) = if i == 0 {
                ("pulse", "PulseAudio")
            } else {
                ("adaptive", "aptX Adaptive")
            };
            me_backend.update(cx, |this, cx| this.begin_switch(false, target, label, cx));
        });
        let backend_card = white_card()
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(12.))
            .child(text("音频后端", 12., INK, true))
            .child(segmented(
                "backend",
                &["兼容", "高质量"],
                backend,
                false,
                backend_pick,
            ));

        // Codec grid, two per row; selecting keeps the current backend.
        let mut grid = div().flex().flex_col().gap(px(4.));
        for chunk in s.codecs.chunks(2) {
            let mut row = div().flex().gap(px(4.));
            for (key, label) in chunk {
                let on = *key == s.codec;
                let me = me.clone();
                let k = key.clone();
                let l = label.clone();
                row = row.child(
                    div()
                        .flex_1()
                        .h(px(34.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(9.))
                        .bg(if on {
                            rgba(0x007aff22)
                        } else {
                            rgba(SEG_TRACK)
                        })
                        .id(format!("codec-{key}"))
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            let (me, k, l) = (me.clone(), k.clone(), l.clone());
                            me.update(cx, |this, cx| this.begin_switch(true, &k, &l, cx));
                        })
                        .child(text(label.clone(), 11., if on { ACCENT } else { INK }, on)),
                );
            }
            if chunk.len() == 1 {
                row = row.child(div().flex_1());
            }
            grid = grid.child(row);
        }
        let codec_card = white_card()
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(12.))
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(text("蓝牙协议", 12., INK, true))
                    .child(text(codec_label(s), 11., MUTED, false)),
            )
            .child(grid);

        // Microphone lock: stop PipeWire from moving the default input to the
        // headset on connect (which silently switches the mic mid-call).
        let on = self.mic_lock;
        let lock_btn = div()
            .id("mic-lock")
            .h(px(26.))
            .px(px(12.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(7.))
            .bg(if on {
                rgba(0x007aff22)
            } else {
                rgba(SEG_TRACK)
            })
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                if this.mic_lock {
                    this.mic_lock = false;
                    this.mic_source = None;
                } else {
                    this.mic_lock = true;
                    let task = cx.background_executor().spawn(async {
                        let src = model::fallback_source();
                        if let Some(s) = src.clone() {
                            model::enforce_default_source(&s);
                        }
                        src
                    });
                    cx.spawn(async move |weak, cx| {
                        let src = task.await;
                        weak.update(cx, |this, cx| {
                            this.mic_source = src;
                            cx.notify();
                        })
                        .ok();
                    })
                    .detach();
                }
                cx.notify();
            }))
            .child(text(
                if on { "已锁定" } else { "未锁定" },
                11.,
                if on { ACCENT } else { INK },
                on,
            ));
        let mic_card = white_card()
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(12.))
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(text("锁定麦克风", 11., INK, true))
                    .child(lock_btn),
            )
            .child(text("连接耳机时不切换默认输入设备", 9., MUTED, false));

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(12.))
            .child(head)
            .child(self.notice_line())
            .child(backend_card)
            .child(mic_card)
            .child(codec_card)
    }
}

thread_local! {
    /// One X connection for the popover's lifetime: connecting per call made
    /// moves fail intermittently and churned sockets on every poll tick.
    static X_CONN: std::cell::RefCell<
        Option<(x11rb::rust_connection::RustConnection, usize)>,
    > = const { std::cell::RefCell::new(None) };
}

fn with_x<T>(f: impl FnOnce(&x11rb::rust_connection::RustConnection, usize) -> T) -> Option<T> {
    X_CONN.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = x11rb::connect(None).ok();
        }
        slot.as_ref().map(|(conn, screen)| f(conn, *screen))
    })
}

fn window_xid(window: &mut Window) -> Option<u32> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        _ => None,
    }
}

/// Keep the popover mapped but off every screen. Unmapping a mapped Vulkan
/// surface races the swapchain and can wedge the render thread (xshmfence);
/// parking leaves the surface alive, so hide/show is only a move.
fn park(window: &mut Window) {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{configure_window, map_window, ConfigureWindowAux};

    const PARK_X: i32 = -10000;
    const PARK_Y: i32 = -10000;

    let Some(xid) = window_xid(window) else {
        return;
    };
    let _ = with_x(|conn, _| {
        let _ = configure_window(conn, xid, &ConfigureWindowAux::new().x(PARK_X).y(PARK_Y));
        // No-op when already mapped, and recovers if X dropped the window.
        let _ = map_window(conn, xid);
        let _ = conn.flush();
    });
}

/// Where the mouse pointer is right now (X11 root coordinates). The appindicator
/// extension passes the *icon-local* click position to `Activate`, not a screen
/// position, so the pointer is the only reliable anchor at click time.
fn pointer_pos() -> Option<(i32, i32)> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt;
    with_x(|conn, screen| {
        let root = conn.setup().roots[screen].root;
        conn.query_pointer(root)
            .ok()?
            .reply()
            .ok()
            .map(|p| (p.root_x as i32, p.root_y as i32))
    })
    .flatten()
}

/// Root screen size (X11), used to place auto-shown popovers.
fn screen_size() -> (i32, i32) {
    use x11rb::connection::Connection;
    with_x(|conn, screen| {
        let root = &conn.setup().roots[screen];
        (root.width_in_pixels as i32, root.height_in_pixels as i32)
    })
    .unwrap_or((1920, 1080))
}

fn reposition(window: &mut Window, icon_x: i32, icon_y: i32) -> Option<(i32, i32, i32, i32)> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        configure_window, map_window, ConfigureWindowAux, ConnectionExt as _, StackMode,
    };

    const W: i32 = 380;
    const H: i32 = 580;
    let xid = window_xid(window)?;
    with_x(|conn, screen| {
        let root = &conn.setup().roots[screen];
        let sw = root.width_in_pixels as i32;
        let sh = root.height_in_pixels as i32;

        // Stay inside the work area so the popover never slides under the panel.
        let (mut wa_bottom, mut wa_right) = (sh, sw);
        if let Ok(cookie) = conn.intern_atom(false, b"_NET_WORKAREA") {
            let atom = cookie.reply().map(|r| r.atom).unwrap_or(0);
            if let Ok(cookie) = conn.get_property(
                false,
                root.root,
                atom,
                x11rb::protocol::xproto::AtomEnum::CARDINAL,
                0,
                4,
            ) {
                let v: Vec<u32> = cookie
                    .reply()
                    .map(|r| r.value32().map(|it| it.collect()).unwrap_or_default())
                    .unwrap_or_default();
                if v.len() == 4 {
                    wa_bottom = (v[1] + v[3]) as i32;
                    wa_right = (v[0] + v[2]) as i32;
                }
            }
        }

        let mut left = icon_x - W / 2;
        let mut top = icon_y + 8;
        // Not enough room below: flip above the icon (matches the GTK popover).
        if top + H + 8 > wa_bottom {
            top = icon_y - H - 8;
        }
        left = left.clamp(8, (wa_right - W - 8).max(8));
        top = top.clamp(8, (wa_bottom - H - 8).max(8));

        let _ = configure_window(
            conn,
            xid,
            &ConfigureWindowAux::new()
                .x(left)
                .y(top)
                .stack_mode(StackMode::ABOVE),
        );
        // Already mapped by main.rs; this only recovers if X dropped it.
        let _ = map_window(conn, xid);
        let _ = conn.flush();
        (left, top, W, H)
    })
}

// ---- small text builders ----

// Hero art: the same bootstrap "earbuds" glyph used for the tray icon,
// rendered to PNG (GPUI's Svg renderer drew the raw SVG blank) and scaled up.
fn hero_image() -> impl IntoElement {
    img(Arc::new(gpui_kit::Image::from_bytes(
        gpui_kit::ImageFormat::Png,
        include_bytes!("../../assets/hero-earbuds.png").to_vec(),
    )))
    .w(px(54.))
    .h(px(54.))
}

fn text(content: impl Into<SharedString>, size: f32, color: u32, bold: bool) -> Div {
    div()
        .text_size(px(size))
        .text_color(rgb(color))
        .font_weight(if bold {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        })
        .child(content.into())
}

fn white_card() -> Div {
    div()
        .w_full()
        .bg(rgb(CARD))
        .rounded(px(12.))
        .border_1()
        .border_color(rgba(HAIRLINE))
        .shadow_sm()
}

fn battery_pct(v: Option<(i32, bool)>) -> (String, bool) {
    match v {
        Some((pct, charging)) if pct >= 0 => (format!("{pct}%"), charging),
        _ => ("未检测".to_string(), false),
    }
}

/// Status line, worded like the GTK tray so both front-ends say the same thing.
fn status_line(s: &State, offline: bool) -> String {
    if offline {
        return "服务离线".to_string();
    }
    if s.connected {
        let mode = MODES.get(s.noise_mode as usize).copied().unwrap_or("—");
        return format!("已连接 · {mode}");
    }
    if s.bluetooth_connected {
        return "蓝牙已连接 · 等待音频".to_string();
    }
    if s.has_cache {
        return "缓存状态".to_string();
    }
    "未连接电脑".to_string()
}

fn protocol_line(s: &State) -> String {
    let c = codec_label(s);
    if c.is_empty() {
        return "协议：读取中…".to_string();
    }
    // aptX Adaptive negotiates 48 kHz/24-bit when the link allows it.
    if s.codec.starts_with("aptx_adaptive") {
        format!("协议：{c} · 48 kHz/24-bit")
    } else {
        format!("协议：{c}")
    }
}

fn codec_label(s: &State) -> String {
    s.codecs
        .iter()
        .find(|(k, _)| *k == s.codec)
        .map(|(_, label)| label.clone())
        .unwrap_or_else(|| s.codec.clone())
}

fn battery_cell(caption: &str, v: Option<(i32, bool)>) -> Div {
    let (value, charging) = battery_pct(v);
    let value_color = if value == "未检测" { MUTED } else { INK };
    let mut value_row = div().flex().items_center().gap(px(2.));
    value_row = value_row.child(text(value, 16., value_color, true));
    if charging {
        value_row = value_row.child(text("⚡", 12., ACCENT, true));
    }
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(2.))
        .child(value_row)
        .child(text(caption, 9., MUTED, false))
}

/// One segmented control row. `selected` indexes the highlighted option;
/// clicking a cell calls `on_pick(index)`.
fn segmented(
    prefix: &str,
    options: &[&str],
    selected: usize,
    disabled: bool,
    on_pick: PickFn,
) -> Div {
    let mut row = div()
        .w_full()
        .flex()
        .gap(px(3.))
        .p(px(3.))
        .rounded(px(9.))
        .bg(rgba(SEG_TRACK));
    for (i, opt) in options.iter().enumerate() {
        let on = i == selected;
        let pick = on_pick.clone();
        let ink = if disabled {
            MUTED
        } else if on {
            ACCENT
        } else {
            INK
        };
        row = row.child(
            div()
                .flex_1()
                .h(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .when(on, |d| d.bg(rgb(CARD)).shadow_sm())
                .child(text(*opt, 11., ink, on && !disabled))
                .id(format!("{prefix}-{i}"))
                .when(!disabled, |d| {
                    d.cursor_pointer().on_click(move |_, _, cx| pick(i, cx))
                }),
        );
    }
    row
}

impl Render for Panel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = &self.state;
        let name = if s.name.is_empty() {
            "Xiaomi Buds 5".to_string()
        } else {
            s.name.clone()
        };
        let status = status_line(s, self.offline);
        let protocol = protocol_line(s);
        let mode_summary = MODES.get(s.noise_mode as usize).copied().unwrap_or("—");
        let svc_tx = self.svc_tx.clone();

        // Header: title + status on the left, a small refresh control on the right.
        let header = div()
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(text(name.clone(), 15., INK, true))
                    .child(text(status.clone(), 10., MUTED, false)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        div()
                            .id("refresh")
                            .w(px(24.))
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(rgba(0x0000000f))
                            .cursor_pointer()
                            .on_click(move |_, _, _| {
                                let _ = svc_tx.send(SvcCmd::Refresh);
                            })
                            .child(text("↻", 13., SUB, false)),
                    )
                    .child(
                        div()
                            .id("close")
                            .w(px(24.))
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(rgba(0x0000000f))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| {
                                // A tray popover closes; it does not kill the app.
                                this.hide(window, cx);
                            }))
                            .child(text("✕", 12., SUB, false)),
                    ),
            );

        // Hero: earbuds image on top, device name + status centered below.
        let hero = white_card()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.))
            .pt(px(8.))
            .pb(px(6.))
            .px(px(18.))
            .child(hero_image())
            .child(text(name.clone(), 16., INK, true))
            .child(text(status, 11., SUB, false))
            .child(text(protocol, 10., MUTED, false));

        // Battery: three cells (left / right / case).
        let battery = white_card()
            .flex()
            .items_center()
            .justify_between()
            .p(px(12.))
            .child(battery_cell("左耳", s.battery[0]))
            .child(battery_cell("右耳", s.battery[1]))
            .child(battery_cell("充电盒", s.battery[2]));

        // Volume: title + %, then a draggable slider (smooth thumb, coalesced set).
        let volume = white_card()
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(12.))
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(text("音量", 11., INK, true))
                    .child(text(format!("{}%", self.volume_pct), 11., SUB, false)),
            )
            .child(
                div().w_full().h(px(16.)).child(
                    Slider::new(&self.volume)
                        .bg(rgb(ACCENT))
                        .text_color(rgb(ACCENT)),
                ),
            );

        // Noise control: summary + two segmented rows (click to change).
        let mode_tx = self.svc_tx.clone();
        let mode_pick: PickFn = Rc::new(move |i, _| {
            let _ = mode_tx.send(SvcCmd::SetNoiseMode(i as u32));
        });
        let level_tx = self.svc_tx.clone();
        let cur_mode = s.noise_mode;
        let level_pick: PickFn = Rc::new(move |i, _| {
            let _ = level_tx.send(SvcCmd::SetNoiseLevel(cur_mode, i as u32));
        });
        let noise = white_card()
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(12.))
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(text("降噪控制", 11., INK, true))
                    .child(text(mode_summary, 10., MUTED, false)),
            )
            .child(segmented(
                "mode",
                &MODES,
                s.noise_mode as usize,
                false,
                mode_pick,
            ))
            // A noise level only exists inside an ANC mode: with ANC off the
            // device rejects the write, so the row is inert rather than an error.
            .child(segmented(
                "level",
                &LEVELS,
                s.noise_level as usize,
                s.noise_mode == 0,
                level_pick,
            ));

        // Action row: open the in-popover settings page / reconnect the device.
        let settings_btn = div()
            .id("open-settings")
            .flex_1()
            .h(px(30.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .bg(rgba(SEG_TRACK))
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                this.page = Page::Settings;
                cx.notify();
            }))
            .child(text("声音设置", 11., INK, false));
        let reconnect_tx = self.svc_tx.clone();
        let reconnect_btn = div()
            .id("reconnect")
            .flex_1()
            .h(px(30.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .bg(rgba(SEG_TRACK))
            .cursor_pointer()
            .on_click(move |_, _, _| {
                let _ = reconnect_tx.send(SvcCmd::Connect);
            })
            .child(text("重新连接", 11., INK, false));
        let actions = div()
            .w_full()
            .flex()
            .gap(px(4.))
            .child(settings_btn)
            .child(reconnect_btn);

        let main_view = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(12.))
            .child(header)
            .child(hero)
            .child(self.notice_line())
            .child(battery)
            .child(volume)
            .child(noise)
            .child(actions);

        let body = match self.page {
            Page::Main => main_view,
            Page::Settings => self.render_settings(cx),
        };

        // No shadow: GPUI paints shadows onto the transparent padding as an
        // opaque light halo, which showed up as a white square around the card.
        let card = div()
            .size_full()
            .bg(rgb(BG))
            .rounded(px(18.))
            .overflow_hidden()
            .child(body);

        // No fade-in: the popover is an override-redirect window and gets no
        // animation frames until it is composited, which left it invisible.
        div().size_full().p(px(14.)).child(card)
    }
}

#[cfg(test)]
mod tests {
    use super::{protocol_line, status_line};
    use crate::model::State;

    #[test]
    fn status_and_protocol_lines() {
        let mut s = State::default();
        assert_eq!(status_line(&s, true), "服务离线");
        assert_eq!(status_line(&s, false), "未连接电脑");
        s.has_cache = true;
        assert_eq!(status_line(&s, false), "缓存状态");
        s.bluetooth_connected = true;
        assert_eq!(status_line(&s, false), "蓝牙已连接 · 等待音频");
        s.connected = true;
        s.noise_mode = 1;
        assert_eq!(status_line(&s, false), "已连接 · 降噪");

        s.codecs = vec![("aptx_adaptive_hq".into(), "aptX Adaptive HQ".into())];
        s.codec = "aptx_adaptive_hq".into();
        assert_eq!(protocol_line(&s), "协议：aptX Adaptive HQ · 48 kHz/24-bit");
        s.codec = "sbc".into();
        assert_eq!(protocol_line(&s), "协议：sbc");
        s.codec.clear();
        assert_eq!(protocol_line(&s), "协议：读取中…");
    }
}
