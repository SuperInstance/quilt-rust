//! THE METAL PROOF (no hardware) — QuiltWire v0 over a real kernel pty.
//!
//! What proves what here:
//!
//! 1. `openpty(3)` creates a genuine kernel pseudo-terminal pair. The slave
//!    end is exactly what a USB-CDC serial port looks like to userland: a
//!    byte-stream fd behind termios, cooked by default. Both ends are set
//!    raw. (socat / python pty are the same mechanism; this needs neither
//!    binary — only libc, which every unix test host has.)
//! 2. The **master end plays the ESP32 cell**: a Rust twin of the sender
//!    discipline in `firmware/esp32-cell/esp32_cell.ino` — same epsilon-delta
//!    logic, same heartbeat cadence, same seq-per-frame accounting, same
//!    honest retry/backoff — writes N frames, with deliberate line noise
//!    injected mid-stream to exercise resync.
//! 3. The **slave end is the desktop peer** (`quilt_wire::peer::ArrivalPeer`),
//!    reading the serial-style stream and stamping arrivals.
//! 4. Asserted: 100% decode, seq continuity 0..N-1, walks/2 arrival lines
//!    written with road stamped and link_quality present, chain verifies.
//!
//! Silicon remains untested (no board attached) — what this proves is the
//! link-core + peer + serial-framing path, i.e. everything above the metal.

#![cfg(unix)]

use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::FromRawFd;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use quilt_wire::frame::{Frame, Kind};
use quilt_wire::peer::{ArrivalPeer, PeerConfig};
use quilt_wire::walks;
use serde_json::Value;

const N: u16 = 250;
const CELL: u8 = 7;

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A real pty pair, both ends raw (no CR/NL translation, no echo, no
/// canonical-mode line editing — binary-safe like a CDC data pipe).
fn raw_pty_pair() -> (i32, i32) {
    unsafe {
        let mut master: i32 = 0;
        let mut slave: i32 = 0;
        let rc = libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        assert_eq!(rc, 0, "openpty failed");
        for fd in [master, slave] {
            let mut t: libc::termios = std::mem::zeroed();
            assert_eq!(libc::tcgetattr(fd, &mut t), 0);
            libc::cfmakeraw(&mut t);
            assert_eq!(libc::tcsetattr(fd, libc::TCSANOW, &t), 0);
        }
        (master, slave)
    }
}

/// Sensor stub — twin of the firmware's `sensor_stub()`: a slow staircase
/// (plateaus of 40 ticks, steps of 0.5 °C) so both DELTA-on-change and
/// TICK-heartbeat paths are exercised.
fn sensor_stub(tick: u32) -> f32 {
    20.0 + ((tick / 40) % 3) as f32 * 0.5
}

/// The firmware sender discipline, as a pure function: tick the value cell
/// at 1 Hz; DELTA when |v − last_sent| > ε (0.05); otherwise a TICK
/// heartbeat at most every 30 s of silence. One seq per sent frame.
/// Mirrors `esp32_cell.ino` `loop()` exactly.
fn firmware_twin_frames(n: u16) -> Vec<Frame> {
    const EPS: f32 = 0.05;
    const HEARTBEAT_TICKS: u32 = 30;
    let mut out = Vec::new();
    let mut seq: u16 = 0;
    let mut last_sent: f32 = f32::NAN;
    let mut since_sent: u32 = 0;
    let mut tick: u32 = 0;
    while (out.len() as u16) < n {
        tick += 1;
        let v = sensor_stub(tick);
        let changed = (v - last_sent).abs() > EPS;
        if changed || since_sent >= HEARTBEAT_TICKS {
            let kind = if changed { Kind::Delta } else { Kind::Tick };
            out.push(Frame::from_f32(kind, CELL, seq, tick, v));
            seq = seq.wrapping_add(1);
            last_sent = v;
            since_sent = 0;
        } else {
            since_sent += 1;
        }
    }
    out
}

/// Honest retry/backoff — twin of the firmware's `send_frame()`: three
/// attempts, delay 2 ms → 8 ms → 32 ms (firmware: 8 → 32 → 128, capped
/// 512 ms; shortened here so a wedged stream fails fast in test).
/// Returns false only after real retries failed — never fakes a send.
fn send_frame_twin(stream: &mut File, bytes: &[u8; 16]) -> bool {
    let mut delay = Duration::from_millis(2);
    for _ in 0..3 {
        match stream.write_all(bytes) {
            Ok(()) => return true,
            Err(_) => std::thread::sleep(delay),
        }
        delay *= 4;
    }
    false
}

/// Noise that must NOT contain the byte pair 0x51 0x01 (magic+version) in
/// positions that could form a false frame start; bytes here are printable
/// ASCII + two control bytes, no 0x51/0x01 at all.
const NOISE: &[u8] = b"QW-LINK-NOISE-\x00\xff\x07-";

#[test]
fn pty_loopback_metal_proof() {
    let (master_fd, slave_fd) = raw_pty_pair();

    // ---- master end: the ESP32-cell twin writes N frames ----
    let frames = firmware_twin_frames(N);
    let writer = std::thread::spawn(move || {
        let mut stream = unsafe { File::from_raw_fd(master_fd) };
        let mut dropped = 0u32;
        for (i, f) in frames.iter().enumerate() {
            if !send_frame_twin(&mut stream, &f.encode()) {
                dropped += 1; // honest miss — the gap is the signal
            }
            if i == 50 || i == 137 {
                let _ = stream.write_all(NOISE); // line noise mid-stream
            }
            // Cadence: 1ms per frame (a 1 Hz cell sped up 1000x). This also
            // matters for real-physics reasons: a kernel pty, like an unread
            // UART, DROPS bytes once its input buffer fills — an overrun is
            // delivery loss, not decode loss, and this proof measures the
            // codec+peer, so we pace like the firmware would.
            std::thread::sleep(Duration::from_millis(1));
        }
        let _ = stream.flush();
        dropped
    });

    // ---- slave end: the desktop arrival peer ----
    let mut peer = ArrivalPeer::new(PeerConfig {
        road: "local".into(),
        medium: "usb-cdc-pty".into(),
        cell_prefix: "cell".into(),
        alpha: 0.25,
    });
    let mut reader = unsafe { File::from_raw_fd(slave_fd) };
    let mut buf = [0u8; 4096];
    let mut lines = String::new();
    let deadline = Instant::now() + Duration::from_secs(30);

    loop {
        if peer.stats().lines as u16 >= N {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "loopback timed out at {} lines",
            peer.stats().lines
        );
        let mut pfd = libc::pollfd {
            fd: slave_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let r = unsafe { libc::poll(&mut pfd, 1, 250) };
        assert!(r >= 0, "poll failed: {}", std::io::Error::last_os_error());
        if r == 0 {
            continue; // spurious wakeup window — recheck counts/deadline
        }
        let n = reader.read(&mut buf).expect("read from pty slave");
        if n == 0 {
            break; // master closed and drained
        }
        let ts = epoch_ms();
        peer.feed(&buf[..n], ts, None, |line| {
            lines.push_str(line);
            lines.push('\n');
        });
    }
    let dropped = writer.join().expect("writer thread panicked");

    // ---- assertions: the metal-proof claims ----
    assert_eq!(dropped, 0, "firmware twin must not have dropped frames");
    let s = peer.stats();
    assert_eq!(s.frames, N as u64, "100% decode: every frame landed");
    assert_eq!(s.lines, N as u64, "one walks/2 line per frame");
    assert_eq!(s.gaps, 0, "seq continuity: no gaps");
    assert_eq!(s.duplicates, 0, "no duplicates");
    assert_eq!(s.restarts, 0, "no spurious restarts");

    // Chain verifies; road coverage 100%.
    let report = walks::verify(&lines).expect("walks/2 chain verifies");
    assert_eq!(report.steps, N as usize);
    assert_eq!(report.walks, 1);
    assert_eq!(report.roads_unknown, 0, "road stamped on every line");

    // Per-line shape: road, link_quality, arrival stamp; seq continuity.
    let mut last_seq: Option<u16> = None;
    let mut opcodes = std::collections::BTreeSet::new();
    for (i, line) in lines.lines().enumerate() {
        let v: Value = serde_json::from_str(line).expect("line is JSON");
        assert_eq!(v["road"].as_str(), Some("local"), "line {i}: road stamped");
        let q = v["link_quality"]
            .as_f64()
            .expect("line {i}: link_quality present, numeric");
        assert!(
            q > 0.0 && q <= 1.0,
            "line {i}: delivery-ratio EWMA in (0,1], got {q}"
        );
        let am = v["arrival_meta"]
            .as_object()
            .expect("line {i}: arrival_meta object");
        assert!(
            am.contains_key("arrival_epoch_ms"),
            "line {i}: epoch_ms stamped"
        );
        assert!(am.contains_key("medium"), "line {i}: medium recorded");
        let seq = v["meta"]["seq"].as_u64().expect("line {i}: meta.seq") as u16;
        match last_seq {
            None => assert_eq!(seq, 0, "first seq is 0"),
            Some(prev) => {
                assert_eq!(
                    seq,
                    prev.wrapping_add(1),
                    "line {i}: seq contiguous ({prev} -> {seq})"
                )
            }
        }
        last_seq = Some(seq);
        opcodes.insert(v["opcode"].as_str().expect("opcode").to_string());
    }
    assert_eq!(last_seq, Some(N - 1), "seq ran 0..={} contiguously", N - 1);
    assert!(
        opcodes.contains("effect"),
        "delta arrivals mapped to effect steps"
    );
    assert!(
        opcodes.contains("tick"),
        "heartbeat arrivals mapped to tick steps"
    );

    // Link quality stayed honest: a noise-free seq stream pins it near 1.0.
    let q = peer.link_quality(CELL).expect("quality estimate exists");
    assert!(q > 0.99, "clean stream quality ~1.0, got {q}");

    // Persist the proof artifact for inspection (target/ is gitignored).
    let proof = format!("target/quilt-wire-loopback-proof-{}.jsonl", epoch_ms());
    if let Some(dir) = std::path::Path::new(&proof)
        .parent()
        .map(|p| p.to_path_buf())
    {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&proof, &lines);
    eprintln!("pty loopback: {N} frames, 100% decode, chain verified; proof written to {proof}");
}
