//! quilt-wire-peer — desktop arrival peer.
//!
//! Reads a serial-style byte stream (default: stdin), decodes QuiltWire v0
//! frames, stamps arrivals, writes `walks/2`-compatible JSONL (default:
//! stdout). Honest counters go to stderr at EOF.
//!
//! ```text
//! quilt-wire-peer [--road ROAD] [--medium MEDIUM] [--input PATH|-] [--output PATH|-]
//! ```

use std::io::{Read, Write};

use quilt_wire::peer::{ArrivalPeer, PeerConfig};

fn main() {
    let mut cfg = PeerConfig::default();
    let mut input: String = "-".into();
    let mut output: String = "-".into();

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--road" => cfg.road = args.next().unwrap_or_else(|| die("--road needs a value")),
            "--medium" => cfg.medium = args.next().unwrap_or_else(|| die("--medium needs a value")),
            "--cell-prefix" => {
                cfg.cell_prefix = args
                    .next()
                    .unwrap_or_else(|| die("--cell-prefix needs a value"))
            }
            "--input" => input = args.next().unwrap_or_else(|| die("--input needs a value")),
            "--output" => output = args.next().unwrap_or_else(|| die("--output needs a value")),
            "--help" | "-h" => {
                println!("usage: quilt-wire-peer [--road ROAD] [--medium MEDIUM] [--input PATH|-] [--output PATH|-]");
                return;
            }
            other => die(&format!("unknown arg {other:?}")),
        }
    }

    if !quilt_wire::walks::ROADS.contains(&cfg.road.as_str()) {
        die(&format!(
            "road must be one of {:?}",
            quilt_wire::walks::ROADS
        ));
    }

    let mut peer = ArrivalPeer::new(cfg);
    let epoch_ms = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    };

    let mut reader: Box<dyn Read> = match input.as_str() {
        "-" => Box::new(std::io::stdin()),
        path => Box::new(
            std::fs::File::open(path).unwrap_or_else(|e| die(&format!("open {path}: {e}"))),
        ),
    };
    let mut writer: Box<dyn Write> = match output.as_str() {
        "-" => Box::new(std::io::stdout()),
        path => Box::new(
            std::fs::File::create(path).unwrap_or_else(|e| die(&format!("create {path}: {e}"))),
        ),
    };

    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break, // EOF — the peer stops honestly, no padding
            Ok(n) => {
                let ts = epoch_ms();
                peer.feed(&buf[..n], ts, None, |line| {
                    let _ = writeln!(writer, "{line}");
                });
            }
            Err(e) => die(&format!("read: {e}")),
        }
    }
    let _ = writer.flush();
    let s = peer.stats();
    eprintln!(
        "quilt-wire-peer: frames={} lines={} gaps={} duplicates={} restarts={}",
        s.frames, s.lines, s.gaps, s.duplicates, s.restarts
    );
}

fn die(msg: &str) -> ! {
    eprintln!("quilt-wire-peer: {msg}");
    std::process::exit(2);
}
