//! Thin D-Bus client for the Xiaomi Buds 5 control service.
//!
//! Talks to the running service over the existing `io.github.kang.XiaomiBuds5`
//! interface; it never touches RFCOMM or the audio stack itself.

use std::collections::HashMap;
use std::process::ExitCode;

use zbus::blocking::Proxy;

const BUS: &str = "io.github.kang.XiaomiBuds5";
const PATH: &str = "/io/github/kang/XiaomiBuds5";
const IFACE: &str = "io.github.kang.XiaomiBuds5";

type Res = Result<(), Box<dyn std::error::Error>>;

fn proxy() -> Result<Proxy<'static>, Box<dyn std::error::Error>> {
    Ok(Proxy::new_owned(
        zbus::blocking::Connection::session()?,
        BUS,
        PATH,
        IFACE,
    )?)
}

fn mode_name(m: u32) -> &'static str {
    match m {
        1 => "降噪",
        2 => "通透",
        _ => "关闭",
    }
}

fn level_name(l: u32) -> &'static str {
    match l {
        0 => "舒适",
        1 => "均衡",
        2 => "深度",
        _ => "-",
    }
}

fn parse_mode(s: &str) -> Option<u32> {
    match s.to_ascii_lowercase().as_str() {
        "anc" | "noise" | "降噪" | "1" => Some(1),
        "transparency" | "transparent" | "通透" | "2" => Some(2),
        "off" | "normal" | "关闭" | "0" => Some(0),
        _ => None,
    }
}

fn status() -> Res {
    let p = proxy()?;
    let connected: bool = p.get_property("Connected")?;
    let name: String = p.get_property("Name")?;
    let address: String = p.get_property("Address")?;
    let bt: bool = p.get_property("BluetoothConnected")?;
    let battery: HashMap<String, (i32, bool)> = p.get_property("Battery")?;
    let mode: u32 = p.get_property("NoiseMode")?;
    let level: u32 = p.get_property("NoiseLevel")?;
    let backend: String = p.get_property("AudioBackend")?;
    let codec: String = p.get_property("Codec")?;
    let last_error: String = p.get_property("LastError")?;

    let cell = |k: &str| match battery.get(k) {
        Some((pct, chg)) if *pct >= 0 => format!("{pct}%{}", if *chg { " 充电中" } else { "" }),
        _ => "未知".to_string(),
    };
    println!("设备     : {name} ({address})");
    println!("蓝牙连接 : {}", if bt { "已连接" } else { "未连接" });
    println!("控制通道 : {}", if connected { "已连接" } else { "未连接" });
    println!(
        "电量     : 左 {} / 右 {} / 盒 {}",
        cell("left"),
        cell("right"),
        cell("case")
    );
    println!(
        "降噪     : {} (强度 {})",
        mode_name(mode),
        level_name(level)
    );
    println!("音频后端 : {backend}    编码: {codec}");
    if !last_error.is_empty() {
        println!("最近错误 : {last_error}");
    }
    Ok(())
}

fn set_noise(mode: u32) -> Res {
    proxy()?.call::<_, _, ()>("SetNoiseMode", &(mode,))?;
    println!("降噪已切换为: {}", mode_name(mode));
    Ok(())
}

fn selftest() -> Res {
    let p = proxy()?;
    let bt: bool = p.get_property("BluetoothConnected")?;
    let connected: bool = p.get_property("Connected")?;
    let last_error: String = p.get_property("LastError")?;
    println!("服务     : 可访问 ({BUS})");
    println!("蓝牙连接 : {}", if bt { "已连接" } else { "未连接" });
    println!("控制通道 : {}", if connected { "已连接" } else { "未连接" });
    println!(
        "最近错误 : {}",
        if last_error.is_empty() {
            "无"
        } else {
            &last_error
        }
    );
    // A missing control channel is normal while a phone owns the buds; only a
    // dropped bluetooth link or a recorded error is a real failure.
    if !bt {
        return Err("耳机蓝牙未连接".into());
    }
    if !last_error.is_empty() {
        return Err(format!("服务最近错误: {last_error}").into());
    }
    println!("selftest : OK");
    Ok(())
}

fn usage() {
    println!("用法: xiaomi-buds5 <命令>");
    println!("  status                    显示电量/降噪/编码等状态（默认）");
    println!("  noise anc|transparency|off    设置降噪模式");
    println!("  selftest                  自检服务与连接");
}

fn run(args: &[String]) -> Res {
    match args {
        [] => status(),
        [c] if c.as_str() == "status" => status(),
        [c, m] if c.as_str() == "noise" => match parse_mode(m) {
            Some(v) => set_noise(v),
            None => Err(format!("未知降噪模式: {m}（anc/transparency/off）").into()),
        },
        [c] if c.as_str() == "selftest" => selftest(),
        [c] if c.as_str() == "-h" || c.as_str() == "--help" || c.as_str() == "help" => {
            usage();
            Ok(())
        }
        _ => {
            usage();
            Err("参数错误".into())
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("错误: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_noise_modes() {
        assert_eq!(parse_mode("anc"), Some(1));
        assert_eq!(parse_mode("transparency"), Some(2));
        assert_eq!(parse_mode("off"), Some(0));
        assert_eq!(parse_mode("降噪"), Some(1));
        assert_eq!(parse_mode("2"), Some(2));
        assert_eq!(parse_mode("bogus"), None);
    }
}
