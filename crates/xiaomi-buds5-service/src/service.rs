//! D-Bus control service (zbus blocking) that owns one long-lived Xiaomi Buds 5
//! control session. A port of `src/xiaomi_buds5_panel/service.py`.
//!
//! Architecture: an RFCOMM worker thread owns the [`BudsSession`] and does all
//! blocking I/O; the D-Bus interface only reads/writes the shared [`Inner`]
//! snapshot and enqueues commands. Every state change is pushed to clients as a
//! `StateChanged` signal, so the panel updates in real time without polling the
//! device.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedValue, Value};

use crate::rcsp::{BudsError, BudsSession, Cell};

pub const BUS_NAME: &str = "io.github.kang.XiaomiBuds5";
pub const OBJECT_PATH: &str = "/io/github/kang/XiaomiBuds5";
pub const INTERFACE: &str = "io.github.kang.XiaomiBuds5";
const ADAPTIVE_SERVICE: &str = "xiaomi-buds5-adaptive.service";

/// BlueZ names a paired device's card `bluez_card.<MAC with ':' replaced by '_'>`.
fn bluez_card_name(address: &str) -> String {
    format!("bluez_card.{}", address.replace(':', "_"))
}

// --- paths / codec vocab -------------------------------------------------

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}
fn state_dir() -> PathBuf {
    home_dir().join(".local/share/xiaomi-buds5-panel/state")
}
fn cache_file() -> PathBuf {
    state_dir().join("last-state.json")
}
fn adaptive_state() -> PathBuf {
    state_dir().join("adaptive.pid")
}
fn adaptive_runtime() -> PathBuf {
    home_dir().join(".local/share/xiaomi-buds5-panel/adaptive-runtime")
}

fn codec_alias(name: &str) -> String {
    match name {
        "769" => "aptx_adaptive",
        "770" => "aptx_adaptive_ll",
        "771" => "aptx_adaptive_hq",
        "6" => "aptx",
        "4" => "aac",
        "1" => "sbc",
        "2" => "sbc_xq",
        other => other,
    }
    .to_string()
}

fn codec_name(alias: &str, description: Option<&str>) -> String {
    match alias {
        "aptx_adaptive_hq" => "aptX Adaptive HQ（接近无损）".into(),
        "aac" => "AAC".into(),
        "aptx_adaptive" => "aptX Adaptive（动态）".into(),
        "aptx_adaptive_ll" => "aptX Adaptive LL（低延迟）".into(),
        "aptx" => "aptX Classic".into(),
        "sbc" => "SBC".into(),
        "sbc_xq" => "SBC-XQ".into(),
        other => description
            .map(|s| s.to_string())
            .unwrap_or_else(|| other.to_string()),
    }
}

/// When the optional aptX Adaptive stack is not installed, the stock BlueZ card
/// reports no `aptx_adaptive*` codecs at all. Surface them anyway, clearly
/// marked, so users know the option exists and how to enable it (docs/aptx-adaptive.md).
fn codec_has_adaptive(codecs: &[(String, String)]) -> bool {
    codecs.iter().any(|(a, _)| a.starts_with("aptx_adaptive"))
}
fn aptx_hint_codecs() -> Vec<(String, String)> {
    vec![
        (
            "aptx_adaptive_hq".into(),
            "aptX Adaptive HQ（需安装可选栈）".into(),
        ),
        (
            "aptx_adaptive".into(),
            "aptX Adaptive（动态）（需安装可选栈）".into(),
        ),
        (
            "aptx_adaptive_ll".into(),
            "aptX Adaptive LL（低延迟）（需安装可选栈）".into(),
        ),
    ]
}

// --- shared state --------------------------------------------------------

#[derive(Clone)]
struct Inner {
    address: String,
    name: String,
    battery: Vec<Cell>,
    noise_mode: u32,
    noise_level: u32,
    last_error: String,
    last_seen: f64,
    connected: bool,
    bluetooth_connected: bool,
    audio_backend: String,
    codec: String,
    codecs: Vec<(String, String)>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CachedCell {
    percent: Option<u8>,
    charging: bool,
}
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Cache {
    #[serde(default)]
    battery: Vec<CachedCell>,
    #[serde(default)]
    noise_mode: u32,
    #[serde(default)]
    noise_level: u32,
    #[serde(default)]
    last_seen: f64,
}

fn load_cache() -> Cache {
    std::fs::read_to_string(cache_file())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_cache(inner: &Inner) {
    let cache = Cache {
        battery: inner
            .battery
            .iter()
            .map(|c| CachedCell {
                percent: c.percent,
                charging: c.charging,
            })
            .collect(),
        noise_mode: inner.noise_mode,
        noise_level: inner.noise_level,
        last_seen: inner.last_seen,
    };
    if let Ok(s) = serde_json::to_string(&cache) {
        let _ = std::fs::create_dir_all(state_dir());
        let _ = std::fs::write(cache_file(), s);
    }
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// --- external helpers ----------------------------------------------------

fn run_pactl(args: &[&str]) -> Option<Output> {
    let mut cmd = Command::new("pactl");
    cmd.args(args);
    if adaptive_state().exists() {
        cmd.env(
            "PULSE_SERVER",
            format!("unix:{}/pulse/native", adaptive_runtime().display()),
        );
    }
    cmd.output().ok()
}

fn refresh_audio_state(inner: &Arc<Mutex<Inner>>) {
    let server = run_pactl(&["info"])
        .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase())
        .unwrap_or_default();
    let backend = if server.contains("pipewire") {
        "adaptive"
    } else {
        "pulse"
    };

    let address = inner.lock().unwrap().address.clone();
    let card = format!("/card/{}/bluez", bluez_card_name(&address));
    let mut codecs: Vec<(String, String)> = Vec::new();
    if let Some(out) = run_pactl(&["send-message", &card, "list-codecs"]) {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(line) {
                for entry in arr {
                    let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    if name.is_empty() {
                        continue;
                    }
                    let alias = codec_alias(name);
                    let desc = entry.get("description").and_then(|v| v.as_str());
                    codecs.push((alias.clone(), codec_name(&alias, desc)));
                }
            }
        }
    }

    if !codec_has_adaptive(&codecs) {
        codecs.extend(aptx_hint_codecs());
    }

    let raw = run_pactl(&["send-message", &card, "get-codec"])
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let current =
        serde_json::from_str::<String>(&raw).unwrap_or_else(|_| raw.trim_matches('"').to_string());

    let mut inner = inner.lock().unwrap();
    inner.audio_backend = backend.to_string();
    inner.codecs = codecs;
    inner.codec = codec_alias(&current);
}

fn bluez_connected(address: &str) -> bool {
    Command::new("bluetoothctl")
        .args(["info", address])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("Connected: yes"))
        .unwrap_or(false)
}

fn systemctl(verb: &str, timeout: Duration) -> Result<(), String> {
    // `timeout` is advisory; systemctl returns once the unit settles.
    let _ = timeout;
    let status = Command::new("systemctl")
        .args(["--user", verb, ADAPTIVE_SERVICE])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("systemctl {verb}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("systemctl {verb} {ADAPTIVE_SERVICE} 失败"))
    }
}

fn bluez_sink_name() -> Option<String> {
    let out = run_pactl(&["list", "short", "sinks"])?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find(|l| l.contains("bluez_output."))
        .and_then(|l| l.split_whitespace().nth(1).map(str::to_string))
}

fn sink_input_ids() -> Vec<String> {
    run_pactl(&["list", "short", "sink-inputs"])
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| l.split_whitespace().next().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn bluez_active_profile(address: &str) -> Option<String> {
    let out = run_pactl(&["list", "cards"])?;
    let s = String::from_utf8_lossy(&out.stdout);
    let card_name = bluez_card_name(address);
    let mut in_card = false;
    for line in s.lines() {
        if line.contains(&card_name) {
            in_card = true;
            continue;
        }
        if in_card {
            let t = line.trim_start();
            if let Some(rest) = t.strip_prefix("Active Profile:") {
                return Some(rest.trim().to_string());
            }
        }
    }
    None
}

/// (volume token like "60%", muted) for the current default sink.
fn default_sink_volume_mute() -> (String, bool) {
    let vol = run_pactl(&["get-sink-volume", "@DEFAULT_SINK@"])
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .find(|t| t.ends_with('%'))
                .unwrap_or("100%")
                .to_string()
        })
        .unwrap_or_else(|| "100%".to_string());
    let mute = run_pactl(&["get-sink-mute", "@DEFAULT_SINK@"])
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .to_lowercase()
                .contains("yes")
        })
        .unwrap_or(false);
    (vol, mute)
}

/// Point the desktop back at the buds after its sink node was recreated: re-set
/// the default sink, keep the volume/mute the user had, and pull any stream that
/// was orphaned when the old node disappeared onto the new one.
fn restore_routing(sink: &str, vol: &str, muted: bool) {
    let _ = run_pactl(&["set-default-sink", sink]);
    let _ = run_pactl(&["set-sink-mute", sink, if muted { "1" } else { "0" }]);
    let _ = run_pactl(&["set-sink-volume", sink, vol]);
    for id in sink_input_ids() {
        let _ = run_pactl(&["move-sink-input", &id, sink]);
    }
}

fn explicit_profile_for(codec: &str) -> Option<&'static str> {
    match codec {
        "aptx_adaptive_hq" => Some("a2dp-sink-aptx_adaptive_hq"),
        "aptx_adaptive" => Some("a2dp-sink-aptx_adaptive"),
        "aptx_adaptive_ll" => Some("a2dp-sink"),
        "aptx" => Some("a2dp-sink-aptx"),
        "aac" => Some("a2dp-sink-aac"),
        "sbc" => Some("a2dp-sink-sbc"),
        "sbc_xq" => Some("a2dp-sink-sbc_xq"),
        _ => None,
    }
}

/// BlueZ lands the card on an `a2dp-auto-*` profile, which renegotiates the codec
/// mid-stream and drops the sink (`running -> error` in pipewire.log) — the panel
/// still shows a codec but there is no sound. Lock it to an explicit profile once
/// so the codec stays put and the sink keeps streaming.
fn ensure_explicit_profile(inner: &Arc<Mutex<Inner>>) -> bool {
    let address = inner.lock().unwrap().address.clone();
    let profile = match bluez_active_profile(&address) {
        Some(p) => p,
        None => return false,
    };
    if !profile.starts_with("a2dp-auto") {
        return false;
    }
    let codec = inner.lock().unwrap().codec.clone();
    let target = explicit_profile_for(&codec).unwrap_or("a2dp-sink-sbc");
    let (vol, mute) = default_sink_volume_mute();
    let card_name = bluez_card_name(&address);
    let _ = run_pactl(&["set-card-profile", card_name.as_str(), target]);
    for _ in 0..16 {
        std::thread::sleep(Duration::from_millis(500));
        if bluez_sink_name().is_some() {
            break;
        }
    }
    if let Some(sink) = bluez_sink_name() {
        restore_routing(&sink, &vol, mute);
    }
    refresh_audio_state(inner);
    eprintln!("[xiaomi-buds5-panel] 锁定显式编码 profile={target}（原自动档 {profile}）");
    true
}

fn switch_audio_backend(
    backend: &str,
    codec: &str,
    inner: &Arc<Mutex<Inner>>,
    address: &str,
) -> Result<(), String> {
    let _ = address; // routing is re-asserted below instead of a disconnect/connect cycle
    if backend != "pulse" && backend != "adaptive" {
        return Err("audio backend must be pulse or adaptive".into());
    }
    let current_backend = inner.lock().unwrap().audio_backend.clone();
    // Snapshot so we can roll back and restore routing if the sink never returns.
    let card_name = bluez_card_name(address);
    let prev_profile = bluez_active_profile(address);
    let (prev_vol, prev_mute) = default_sink_volume_mute();

    if backend == "adaptive" {
        if current_backend != "adaptive" {
            systemctl("start", Duration::from_secs(70))?;
        }
        for _ in 0..20 {
            refresh_audio_state(inner);
            let (b, has) = {
                let i = inner.lock().unwrap();
                (i.audio_backend.clone(), !i.codecs.is_empty())
            };
            if b == "adaptive" && has {
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    } else {
        if current_backend != "pulse" {
            systemctl("stop", Duration::from_secs(20))?;
        }
        for _ in 0..20 {
            refresh_audio_state(inner);
            let (b, has) = {
                let i = inner.lock().unwrap();
                (i.audio_backend.clone(), !i.codecs.is_empty())
            };
            if b == "pulse" && has {
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    if !codec.is_empty() {
        let card = format!("/card/{card_name}/bluez");
        let fixed_profile = match codec {
            "aptx_adaptive_hq" => Some("a2dp-sink-aptx_adaptive_hq"),
            "aptx_adaptive" => Some("a2dp-sink-aptx_adaptive"),
            "aptx_adaptive_ll" => Some("a2dp-sink"),
            _ => None,
        };
        if let Some(profile) = fixed_profile {
            let out = run_pactl(&["set-card-profile", card_name.as_str(), profile]);
            if let Some(o) = out {
                if !o.status.success() {
                    return Err(pactl_err(&o));
                }
            }
        } else if backend == "pulse" {
            let cur = inner.lock().unwrap().codec.clone();
            if cur != codec {
                let out = run_pactl(&["send-message", &card, "switch-codec", codec]);
                if let Some(o) = out {
                    if !o.status.success() {
                        return Err(pactl_err(&o));
                    }
                }
            }
        } else {
            let profile = match codec {
                "aptx" => "a2dp-sink-aptx",
                "aac" => "a2dp-sink-aac",
                "sbc" => "a2dp-sink-sbc",
                "sbc_xq" => "a2dp-sink-sbc_xq",
                other => other,
            };
            let out = run_pactl(&["set-card-profile", card_name.as_str(), profile]);
            if let Some(o) = out {
                if !o.status.success() {
                    return Err(pactl_err(&o));
                }
            }
        }

        // The card reports the new codec before the sink node is rebuilt, and
        // while the sink is gone any playing stream (e.g. Chrome) is orphaned.
        // Re-attach streams the moment the new sink exists rather than waiting
        // for codec renegotiation — otherwise the user hears a long dropout.
        let mut ok = false;
        let mut routed = false;
        for attempt in 0..40 {
            std::thread::sleep(Duration::from_millis(500));
            refresh_audio_state(inner);
            let sink = bluez_sink_name();
            match (&sink, routed) {
                (Some(s), false) => {
                    restore_routing(s.as_str(), &prev_vol, prev_mute);
                    routed = true;
                }
                // Keep streams (including ones that attach late) on the buds.
                (Some(s), true) => {
                    for id in sink_input_ids() {
                        let _ = run_pactl(&["move-sink-input", id.as_str(), s.as_str()]);
                    }
                }
                _ => {}
            }
            let codec_ok = inner.lock().unwrap().codec == codec;
            if routed && codec_ok && sink.is_some() {
                ok = true;
                break;
            }
            // Adaptive profiles occasionally race the renegotiation; re-assert once.
            if attempt == 20 {
                if let Some(p) = fixed_profile {
                    let _ = run_pactl(&["set-card-profile", card_name.as_str(), p]);
                }
            }
        }
        if !ok {
            // Keep the user's sound: roll back to the profile that was working.
            if let Some(prev) = prev_profile.as_deref() {
                let _ = run_pactl(&["set-card-profile", card_name.as_str(), prev]);
            }
            for _ in 0..20 {
                std::thread::sleep(Duration::from_millis(500));
                refresh_audio_state(inner);
                if bluez_sink_name().is_some() {
                    break;
                }
            }
            if let Some(sink) = bluez_sink_name() {
                restore_routing(&sink, &prev_vol, prev_mute);
            }
            refresh_audio_state(inner);
            return Err(format!(
                "切换到 {codec} 失败，已回滚到{}",
                prev_profile.as_deref().unwrap_or("上一个可用编码")
            ));
        }
    }
    // Re-assert routing: the sink node was recreated with a new id, so point
    // the desktop back at the buds and pull any orphaned stream onto it.
    if let Some(sink) = bluez_sink_name() {
        restore_routing(&sink, &prev_vol, prev_mute);
    }
    refresh_audio_state(inner);
    Ok(())
}

fn pactl_err(o: &Output) -> String {
    let err = String::from_utf8_lossy(&o.stderr);
    let out = String::from_utf8_lossy(&o.stdout);
    let msg = if !err.trim().is_empty() { err } else { out };
    let msg = msg.trim();
    if msg.is_empty() {
        "编码切换失败".into()
    } else {
        msg.to_string()
    }
}

// --- D-Bus interface -----------------------------------------------------

enum Cmd {
    Refresh,
    Connect,
    SetNoiseMode(u32),
    SetNoiseLevel(u32, u32),
    SetAudioBackend(String, String),
}

pub struct Service {
    inner: Arc<Mutex<Inner>>,
    tx: Sender<Cmd>,
}

fn ov_str(s: &str) -> OwnedValue {
    Value::from(s).try_to_owned().unwrap()
}

fn battery_map(inner: &Inner) -> HashMap<String, OwnedValue> {
    let names = ["left", "right", "case"];
    let mut out = HashMap::new();
    for (i, cell) in inner.battery.iter().take(3).enumerate() {
        let percent = cell.percent.map(|p| p as i32).unwrap_or(-1);
        let v: OwnedValue = Value::from((percent, cell.charging))
            .try_to_owned()
            .unwrap();
        out.insert(names[i].to_string(), v);
    }
    out
}

fn state_map(inner: &Inner) -> HashMap<String, OwnedValue> {
    let mut m: HashMap<String, OwnedValue> = HashMap::new();
    m.insert("connected".into(), OwnedValue::from(inner.connected));
    m.insert("name".into(), ov_str(&inner.name));
    m.insert("address".into(), ov_str(&inner.address));
    m.insert(
        "battery".into(),
        Value::from(battery_map(inner)).try_to_owned().unwrap(),
    );
    m.insert("noiseMode".into(), OwnedValue::from(inner.noise_mode));
    m.insert("noiseLevel".into(), OwnedValue::from(inner.noise_level));
    m.insert("lastError".into(), ov_str(&inner.last_error));
    m.insert("lastSeen".into(), OwnedValue::from(inner.last_seen));
    m.insert(
        "hasCachedState".into(),
        OwnedValue::from(!inner.battery.is_empty()),
    );
    m.insert(
        "bluetoothConnected".into(),
        OwnedValue::from(inner.bluetooth_connected),
    );
    m.insert("audioBackend".into(), ov_str(&inner.audio_backend));
    m.insert("codec".into(), ov_str(&inner.codec));
    m.insert(
        "codecs".into(),
        Value::from(inner.codecs.clone()).try_to_owned().unwrap(),
    );
    m
}

fn emit(conn: &Connection, inner: &Inner) {
    let body = state_map(inner);
    if let Err(e) = conn.emit_signal(
        None::<&str>,
        OBJECT_PATH,
        INTERFACE,
        "StateChanged",
        &(body,),
    ) {
        eprintln!("emit StateChanged failed: {e}");
    }
}

#[zbus::interface(name = "io.github.kang.XiaomiBuds5")]
impl Service {
    fn refresh(&self) {
        let _ = self.tx.send(Cmd::Refresh);
    }

    fn connect(&self) {
        let _ = self.tx.send(Cmd::Connect);
    }

    fn set_noise_mode(&self, mode: u32) -> zbus::fdo::Result<()> {
        if mode > 2 {
            return Err(zbus::fdo::Error::Failed(
                "noise mode must be 0, 1, or 2".into(),
            ));
        }
        let connected = self.inner.lock().unwrap().connected;
        if !connected {
            return Err(zbus::fdo::Error::Failed("耳机未连接".into()));
        }
        let _ = self.tx.send(Cmd::SetNoiseMode(mode));
        Ok(())
    }

    fn set_noise_level(&self, mode: u32, level: u32) -> zbus::fdo::Result<()> {
        let connected = self.inner.lock().unwrap().connected;
        if !connected {
            return Err(zbus::fdo::Error::Failed("耳机未连接".into()));
        }
        let _ = self.tx.send(Cmd::SetNoiseLevel(mode, level));
        Ok(())
    }

    fn set_audio_backend(&self, backend: String, codec: String) -> zbus::fdo::Result<()> {
        let _ = self.tx.send(Cmd::SetAudioBackend(backend, codec));
        Ok(())
    }

    fn list_codecs(&self) -> Vec<(String, String)> {
        refresh_audio_state(&self.inner);
        self.inner.lock().unwrap().codecs.clone()
    }

    #[zbus(property)]
    fn connected(&self) -> bool {
        self.inner.lock().unwrap().connected
    }
    #[zbus(property)]
    fn name(&self) -> String {
        self.inner.lock().unwrap().name.clone()
    }
    #[zbus(property)]
    fn address(&self) -> String {
        self.inner.lock().unwrap().address.clone()
    }
    #[zbus(property)]
    fn battery(&self) -> HashMap<String, OwnedValue> {
        battery_map(&self.inner.lock().unwrap())
    }
    #[zbus(property)]
    fn noise_mode(&self) -> u32 {
        self.inner.lock().unwrap().noise_mode
    }
    #[zbus(property)]
    fn noise_level(&self) -> u32 {
        self.inner.lock().unwrap().noise_level
    }
    #[zbus(property)]
    fn last_error(&self) -> String {
        self.inner.lock().unwrap().last_error.clone()
    }
    #[zbus(property)]
    fn last_seen(&self) -> f64 {
        self.inner.lock().unwrap().last_seen
    }
    #[zbus(property)]
    fn has_cached_state(&self) -> bool {
        !self.inner.lock().unwrap().battery.is_empty()
    }
    #[zbus(property)]
    fn bluetooth_connected(&self) -> bool {
        self.inner.lock().unwrap().bluetooth_connected
    }
    #[zbus(property)]
    fn audio_backend(&self) -> String {
        self.inner.lock().unwrap().audio_backend.clone()
    }
    #[zbus(property)]
    fn codec(&self) -> String {
        self.inner.lock().unwrap().codec.clone()
    }
    #[zbus(property)]
    fn codecs(&self) -> Vec<(String, String)> {
        self.inner.lock().unwrap().codecs.clone()
    }
}

// --- worker --------------------------------------------------------------

struct Worker {
    inner: Arc<Mutex<Inner>>,
    conn: Connection,
    rx: Receiver<Cmd>,
    address: String,
    verbose: bool,
    stop: Arc<AtomicBool>,
    start: std::time::Instant,
    retry: f64,
    rfcomm_failures: u32,
    last_attempt: f64,
    manual_connect: bool,
    force_connect: bool,
    last_bluez_attempt: f64,
    refresh_requested: bool,
    last_maintenance: f64,
    session: Option<BudsSession>,
}

impl Worker {
    fn elapsed(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    fn set_connected_false(&self, error: Option<&str>) {
        let mut inner = self.inner.lock().unwrap();
        inner.connected = false;
        if let Some(e) = error {
            inner.last_error = e.to_string();
        }
    }

    fn emit(&self) {
        let inner = self.inner.lock().unwrap().clone();
        emit(&self.conn, &inner);
    }

    fn save(&self) {
        let inner = self.inner.lock().unwrap().clone();
        save_cache(&inner);
    }

    /// Returns true if the connection should stop.
    fn wait_cmd(&mut self, dur: Duration) -> bool {
        if dur.is_zero() {
            return false;
        }
        match self.rx.recv_timeout(dur) {
            Ok(cmd) => {
                self.handle_cmd(cmd);
                false
            }
            Err(mpsc::RecvTimeoutError::Timeout) => false,
            Err(mpsc::RecvTimeoutError::Disconnected) => true,
        }
    }

    fn drain_cmds(&mut self) -> bool {
        loop {
            match self.rx.try_recv() {
                Ok(cmd) => self.handle_cmd(cmd),
                Err(mpsc::TryRecvError::Empty) => return false,
                Err(mpsc::TryRecvError::Disconnected) => return true,
            }
        }
    }

    fn handle_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Refresh => {
                self.refresh_requested = true;
                // A user-initiated refresh should not sit out the backoff timer:
                // try to open the control channel right away.
                if self.session.is_none() {
                    self.force_connect = true;
                }
            }
            Cmd::Connect => {
                self.manual_connect = true;
                self.force_connect = true;
            }
            Cmd::SetNoiseMode(mode) => {
                let result = match self.session.as_mut() {
                    Some(s) => s.set_noise(mode as u8),
                    None => Err(BudsError::new("耳机未连接")),
                };
                match result {
                    Ok(()) => {
                        let mut inner = self.inner.lock().unwrap();
                        inner.noise_mode = mode;
                        if mode == 0 {
                            inner.noise_level = 0;
                        } else if inner.noise_level == 0 {
                            inner.noise_level = 1;
                        }
                        inner.last_error.clear();
                        drop(inner);
                        self.save();
                        self.emit();
                    }
                    Err(e) => {
                        self.inner.lock().unwrap().last_error = e.msg.clone();
                        self.emit();
                    }
                }
            }
            Cmd::SetNoiseLevel(mode, level) => {
                let result = match self.session.as_mut() {
                    Some(s) => s.set_noise_level(mode as u8, level as u8),
                    None => Err(BudsError::new("耳机未连接")),
                };
                match result {
                    Ok(()) => {
                        let mut inner = self.inner.lock().unwrap();
                        inner.noise_mode = mode;
                        inner.noise_level = level;
                        drop(inner);
                        self.save();
                        self.emit();
                    }
                    Err(e) => {
                        self.inner.lock().unwrap().last_error = e.msg.clone();
                        self.emit();
                    }
                }
            }
            Cmd::SetAudioBackend(backend, codec) => {
                // Long, blocking subprocess work; runs off the D-Bus thread.
                match switch_audio_backend(&backend, &codec, &self.inner, &self.address) {
                    Ok(()) => {
                        self.inner.lock().unwrap().last_error.clear();
                    }
                    Err(e) => {
                        self.inner.lock().unwrap().last_error = e;
                    }
                }
                self.save();
                self.emit();
            }
        }
    }

    fn maintenance(&mut self) {
        let now = self.elapsed();
        if now - self.last_maintenance < 3.0 {
            return;
        }
        self.last_maintenance = now;
        // Started with no address (no --mac/env, earbuds not paired yet): keep
        // re-running auto-detection so the service recovers once they appear.
        if self.address.is_empty() {
            if let Some(found) = find_buds() {
                self.address = found.clone();
                self.inner.lock().unwrap().address = found;
                eprintln!("[xiaomi-buds5-panel] 自动发现耳机 {0}", self.address);
            }
        }
        refresh_audio_state(&self.inner);
        let bt = bluez_connected(&self.address);
        let changed = {
            let mut inner = self.inner.lock().unwrap();
            let changed = inner.bluetooth_connected != bt;
            inner.bluetooth_connected = bt;
            changed
        };
        if changed {
            // On a fresh (re)connect, BlueZ may have picked an a2dp-auto-* profile
            // that renegotiates and drops the sink. Lock an explicit profile once.
            if bt {
                ensure_explicit_profile(&self.inner);
            }
            self.emit();
        }
    }

    fn open(&mut self) -> Result<(), BudsError> {
        let mut session = BudsSession::new(&self.address, self.verbose);
        session.connect()?;
        session.authenticate()?;
        let info = session.target_info();
        let noise = session.noise_state().ok().flatten();
        let battery = match info.as_ref() {
            Ok(i) if !i.battery.is_empty() => i.battery.clone(),
            _ => self.inner.lock().unwrap().battery.clone(),
        };
        let name = info
            .as_ref()
            .ok()
            .map(|i| {
                i.name
                    .trim_matches(|c: char| c == '\0' || c == ' ')
                    .to_string()
            })
            .filter(|s| !s.is_empty());
        {
            let mut inner = self.inner.lock().unwrap();
            inner.connected = true;
            inner.bluetooth_connected = true;
            inner.battery = battery;
            if let Some(n) = name {
                inner.name = n;
            }
            if let Some((m, l)) = noise {
                inner.noise_mode = if m <= 2 { m as u32 } else { 0 };
                inner.noise_level = l as u32;
            }
            inner.last_error.clear();
            inner.last_seen = now_secs();
        }
        self.save();
        eprintln!(
            "[xiaomi-buds5-panel] connected on RFCOMM {:?}",
            session.channel
        );
        self.emit();
        self.session = Some(session);
        Ok(())
    }

    fn disconnect(&mut self, error: Option<String>) {
        if let Some(mut s) = self.session.take() {
            s.close();
        }
        self.set_connected_false(error.as_deref());
        let bt = bluez_connected(&self.address);
        self.inner.lock().unwrap().bluetooth_connected = bt;
        if let Some(e) = &error {
            eprintln!("[xiaomi-buds5-panel] disconnected: {e}");
        }
        self.save();
        self.emit();
    }

    fn run(&mut self) {
        refresh_audio_state(&self.inner);
        self.inner.lock().unwrap().bluetooth_connected = bluez_connected(&self.address);
        if self.inner.lock().unwrap().bluetooth_connected {
            ensure_explicit_profile(&self.inner);
        }
        self.emit();

        while !self.stop.load(Ordering::Relaxed) {
            if self.drain_cmds() {
                return;
            }

            if self.session.is_none() {
                let now = self.elapsed();
                if !self.force_connect && now - self.last_attempt < self.retry {
                    let wait = (self.retry - (now - self.last_attempt)).max(0.1);
                    if self.wait_cmd(Duration::from_secs_f64(wait)) {
                        return;
                    }
                    continue;
                }
                self.last_attempt = now;
                self.force_connect = false;

                if !bluez_connected(&self.address) {
                    self.inner.lock().unwrap().bluetooth_connected = false;
                    if self.manual_connect {
                        self.ensure_bluetooth();
                    }
                    if !self.inner.lock().unwrap().bluetooth_connected {
                        if self.wait_cmd(Duration::from_secs_f64(self.retry)) {
                            return;
                        }
                        continue;
                    }
                }

                match self.open() {
                    Ok(()) => {
                        self.rfcomm_failures = 0;
                    }
                    Err(exc) => {
                        self.rfcomm_failures += 1;
                        self.set_connected_false(Some(&exc.msg));
                        let backoff = 120.0_f64.min(
                            8.0 * 2f64.powi(self.rfcomm_failures.saturating_sub(1).min(4) as i32),
                        );
                        eprintln!(
                            "[xiaomi-buds5-panel] connect failed ({}): {}; retry in {:.0}s",
                            self.rfcomm_failures, exc.msg, backoff
                        );
                        self.emit();
                        if self.wait_cmd(Duration::from_secs_f64(backoff)) {
                            return;
                        }
                    }
                }
                continue;
            }

            // Connected: actively re-read on Refresh, otherwise wait for the
            // device's unsolicited battery notification.
            let poll = {
                let sess = self.session.as_mut().unwrap();
                if self.refresh_requested {
                    sess.battery().map(Some)
                } else {
                    sess.poll_battery(Duration::from_millis(500))
                }
            };
            match poll {
                Ok(Some(battery)) => {
                    let mut inner = self.inner.lock().unwrap();
                    if !battery.is_empty() {
                        inner.battery = battery;
                    }
                    inner.last_error.clear();
                    inner.last_seen = now_secs();
                    drop(inner);
                    self.save();
                    self.emit();
                }
                Ok(None) => {}
                Err(e) => {
                    self.disconnect(Some(e.msg));
                    self.retry = 3.0;
                }
            }
            self.refresh_requested = false;
            self.maintenance();
        }
    }

    fn ensure_bluetooth(&mut self) {
        let now = self.elapsed();
        if now - self.last_bluez_attempt < 20.0 {
            return;
        }
        self.last_bluez_attempt = now;
        let _ = Command::new("bluetoothctl")
            .args(["connect", &self.address])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let bt = bluez_connected(&self.address);
        self.inner.lock().unwrap().bluetooth_connected = bt;
        if bt {
            self.manual_connect = false;
        }
    }
}

/// Build the initial state (cache preloaded), start the worker and own the bus
/// name. Blocks the calling thread until SIGINT/SIGTERM.
pub fn run(mac: Option<String>, verbose: bool) -> Result<(), Box<dyn std::error::Error>> {
    // No --mac/env and nothing paired yet: leave empty and let the worker keep
    // re-running auto-detection instead of pinning to one machine's address.
    let address = mac.or_else(find_buds).unwrap_or_default();
    let cache = load_cache();
    let inner = Arc::new(Mutex::new(Inner {
        address: address.clone(),
        name: "Xiaomi Buds 5".into(),
        battery: cache
            .battery
            .iter()
            .map(|c| Cell {
                percent: c.percent,
                charging: c.charging,
            })
            .collect(),
        noise_mode: cache.noise_mode,
        noise_level: cache.noise_level,
        last_error: String::new(),
        last_seen: cache.last_seen,
        connected: false,
        bluetooth_connected: false,
        audio_backend: "pulse".into(),
        codec: String::new(),
        codecs: Vec::new(),
    }));

    let (tx, rx) = mpsc::channel();
    let service = Service {
        inner: inner.clone(),
        tx,
    };

    let conn = zbus::blocking::connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, service)?
        .build()?;
    eprintln!("[xiaomi-buds5-panel] owning {BUS_NAME}; target {address}");

    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let worker_inner = inner.clone();
    let worker_conn = conn.clone();
    let worker_address = address.clone();
    let handle = std::thread::spawn(move || {
        let mut worker = Worker {
            inner: worker_inner,
            conn: worker_conn,
            rx,
            address: worker_address,
            verbose,
            stop: worker_stop,
            start: std::time::Instant::now(),
            retry: 8.0,
            rfcomm_failures: 0,
            last_attempt: 0.0,
            manual_connect: false,
            force_connect: false,
            last_bluez_attempt: -1000.0,
            refresh_requested: false,
            last_maintenance: -1000.0,
            session: None,
        };
        worker.run();
    });

    wait_for_signal();
    stop.store(true, Ordering::Relaxed);
    // Wake the worker if it is parked on the channel by dropping nothing —
    // setting stop is enough because waits are bounded.
    let _ = handle.join();
    Ok(())
}

fn find_buds() -> Option<String> {
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
        let addr = parts[1].to_string();
        match &best {
            Some((s, _)) if *s <= score => {}
            _ => best = Some((score, addr)),
        }
    }
    best.map(|(_, a)| a)
}

static TERM: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    TERM.store(true, Ordering::Relaxed);
}

fn wait_for_signal() {
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
    }
    while !TERM.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(test)]
mod tests {
    use super::{aptx_hint_codecs, codec_has_adaptive, explicit_profile_for};

    #[test]
    fn maps_codecs_to_explicit_profiles() {
        // The auto-profile guard relies on this map to pick a stable profile.
        assert_eq!(explicit_profile_for("aptx_adaptive_ll"), Some("a2dp-sink"));
        assert_eq!(
            explicit_profile_for("aptx_adaptive_hq"),
            Some("a2dp-sink-aptx_adaptive_hq")
        );
        assert_eq!(explicit_profile_for("sbc"), Some("a2dp-sink-sbc"));
        assert_eq!(explicit_profile_for("unknown"), None);
    }

    #[test]
    fn adaptive_hint_only_when_stack_missing() {
        // Stock BlueZ (no optional stack) lists no aptx_adaptive* codecs -> we add
        // three clearly-marked hints so the option stays discoverable.
        let stock = vec![
            ("aac".to_string(), "AAC".to_string()),
            ("sbc".to_string(), "SBC".to_string()),
        ];
        assert!(!codec_has_adaptive(&stock));
        assert_eq!(aptx_hint_codecs().len(), 3);

        // With the stack installed the real codecs are present -> no hints.
        let with_stack = vec![("aptx_adaptive".to_string(), "x".to_string())];
        assert!(codec_has_adaptive(&with_stack));
    }
}
