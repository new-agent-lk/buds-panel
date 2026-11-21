//! Entry point for the Rust Xiaomi Buds 5 control daemon.

mod auth;
mod rcsp;
mod service;

use std::process::exit;

fn usage() -> ! {
    eprintln!(
        "usage: xiaomi-buds5-service [--mac MAC] [--verbose]\n\
         \n\
         Owns the io.github.kang.XiaomiBuds5 D-Bus name and keeps a live\n\
         RFCOMM control session to the earbuds. Prints nothing on stdout;\n\
         set --verbose (or XIAOMI_BUDS5_VERBOSE=1) for per-frame logging.\n\
         MAC falls back to $XIAOMI_BUDS5_MAC, then bluetoothctl auto-detect."
    );
    exit(2);
}

fn main() {
    let mut mac: Option<String> = std::env::var("XIAOMI_BUDS5_MAC")
        .ok()
        .filter(|s| !s.is_empty());
    let mut verbose = matches!(
        std::env::var("XIAOMI_BUDS5_VERBOSE").as_deref(),
        Ok("1") | Ok("true")
    );

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => usage(),
            "-v" | "--verbose" => verbose = true,
            "--mac" => match args.next() {
                Some(v) => mac = Some(v),
                None => usage(),
            },
            other if other.starts_with("--mac=") => mac = Some(other["--mac=".len()..].to_string()),
            other => {
                eprintln!("unknown argument: {other}");
                usage();
            }
        }
    }

    if let Err(e) = service::run(mac, verbose) {
        eprintln!("fatal: {e}");
        exit(1);
    }
}
