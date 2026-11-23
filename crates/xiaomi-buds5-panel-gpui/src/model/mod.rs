//! Domain state mirrored from the service, plus local (pactl/bluetoothctl) probes.
//! Copied from the GTK tray while both front-ends coexist; the tray is frozen and
//! this copy is removed together with it in the P6 cleanup (see docs/gpui-migration.md §9).
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

pub const MODES: [&str; 3] = ["关闭", "降噪", "通透"];
pub const LEVELS: [&str; 3] = ["舒适", "均衡", "深度"];

/// Mirrors the service D-Bus properties. Some fields are only needed from P2 on,
/// so they are parsed but not yet rendered in the P1 skeleton.
#[allow(dead_code)]
#[derive(Default, Clone, Debug)]
pub struct State {
    pub connected: bool,
    pub has_cache: bool,
    pub bluetooth_connected: bool,
    pub name: String,
    pub battery: [Option<(i32, bool)>; 3],
    pub noise_mode: u32,
    pub noise_level: u32,
    pub codec: String,
    pub audio_backend: String,
    pub last_error: String,
    pub codecs: Vec<(String, String)>,
}

pub fn parse_state(dict: &HashMap<String, glib::Variant>) -> State {
    let get_bool = |k: &str| dict.get(k).and_then(|v| v.get::<bool>()).unwrap_or(false);
    let get_u32 = |k: &str| dict.get(k).and_then(|v| v.get::<u32>()).unwrap_or(0);
    let get_str = |k: &str| {
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
        connected: get_bool("Connected"),
        has_cache: get_bool("HasCachedState"),
        bluetooth_connected: get_bool("BluetoothConnected"),
        name: get_str("Name"),
        battery,
        noise_mode: get_u32("NoiseMode"),
        noise_level: get_u32("NoiseLevel"),
        codec: get_str("Codec"),
        audio_backend: get_str("AudioBackend"),
        last_error: get_str("LastError"),
        codecs,
    }
}

// ---- local system probes (pactl / bluetoothctl), used for the volume line ----

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
    format!("{}/state", adaptive_runtime())
}

fn has_adaptive() -> bool {
    Path::new(&adaptive_state()).exists()
}

fn pactl(args: &[&str]) -> String {
    let mut cmd = Command::new("pactl");
    if has_adaptive() {
        cmd.env(
            "PULSE_SERVER",
            format!("unix:{}/pulse.sock", adaptive_runtime()),
        );
    }
    match cmd.args(args).output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => String::new(),
    }
}

fn buds_address() -> Option<String> {
    let out = Command::new("bluetoothctl").arg("devices").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if line.contains("Xiaomi Buds") || line.contains("Xiaomi") {
            if let Some(addr) = line.split_whitespace().nth(1) {
                return Some(addr.to_string());
            }
        }
    }
    None
}

fn current_sink() -> String {
    if let Some(addr) = buds_address() {
        let addr_us = addr.replace(':', "_");
        let out = pactl(&["list", "short", "sinks"]);
        for line in out.lines() {
            if line.contains(&addr) || line.contains(&addr_us) {
                if let Some(name) = line.split_whitespace().nth(1) {
                    return name.to_string();
                }
            }
        }
    }
    "@DEFAULT_SINK@".to_string()
}

fn sink_volume_pct(sink: &str) -> i32 {
    let out = pactl(&["get-sink-volume", sink]);
    // e.g. "Volume: front-left: 32768 /  50% / -18.06 dB,   front-right: ... 50%"
    for tok in out.split('%').rev().skip(1) {
        if let Some(num) = tok.split_whitespace().last() {
            if let Ok(v) = num.parse::<i32>() {
                return v;
            }
        }
    }
    0
}

pub fn read_volume_pct() -> i32 {
    sink_volume_pct(&current_sink())
}

/// Set the earbuds sink volume (local PulseAudio/PipeWire, not the device gain).
pub fn set_volume_pct(pct: i32) {
    let sink = current_sink();
    let _ = pactl(&["set-sink-volume", &sink, &format!("{}%", pct.clamp(0, 130))]);
}

/// Input sources, excluding the per-sink monitor pseudo-sources.
fn source_names() -> Vec<String> {
    pactl(&["list", "short", "sources"])
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1).map(str::to_string))
        .filter(|n| !n.ends_with(".monitor"))
        .collect()
}

fn is_buds_source(name: &str) -> bool {
    match buds_address() {
        Some(addr) => {
            let us = addr.replace(':', "_");
            name.contains(&addr) || name.contains(&us)
        }
        None => false,
    }
}

pub fn default_source() -> String {
    pactl(&["get-default-source"]).trim().to_string()
}

fn set_default_source(name: &str) {
    let _ = pactl(&["set-default-source", name]);
}

/// The built-in microphone: the first source that is not the earbuds.
pub fn fallback_source() -> Option<String> {
    source_names().into_iter().find(|n| !is_buds_source(n))
}

/// Keep `preferred` as the default input while the earbuds are connected:
/// PipeWire/WirePlumber moves the default to the headset on connect, which
/// silently switches the microphone mid-call (AirBuddy's "audio input lock").
pub fn enforce_default_source(preferred: &str) {
    let cur = default_source();
    if cur != preferred && is_buds_source(&cur) {
        set_default_source(preferred);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glib::ToVariant;

    #[test]
    fn parses_service_properties() {
        let mut d: HashMap<String, glib::Variant> = HashMap::new();
        d.insert("Connected".into(), true.to_variant());
        d.insert("HasCachedState".into(), true.to_variant());
        d.insert("Name".into(), "Xiaomi Buds 5".to_variant());
        d.insert("NoiseMode".into(), 1u32.to_variant());
        d.insert("NoiseLevel".into(), 2u32.to_variant());
        let mut batt: HashMap<String, glib::Variant> = HashMap::new();
        batt.insert("left".into(), (92, false).to_variant());
        batt.insert("right".into(), (88, true).to_variant());
        batt.insert("case".into(), (-1, false).to_variant());
        d.insert("Battery".into(), batt.to_variant());

        let s = parse_state(&d);
        assert!(s.connected && s.has_cache);
        assert_eq!(s.name, "Xiaomi Buds 5");
        assert_eq!(s.noise_mode, 1);
        assert_eq!(s.noise_level, 2);
        assert_eq!(s.battery[0], Some((92, false)));
        assert_eq!(s.battery[1], Some((88, true)));
        assert_eq!(s.battery[2], Some((-1, false)));
    }
}
