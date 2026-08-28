# firmware/esp32-cell — the minimal exocortex cell (Rung 5a)

One ESP32, one temperature-ish sensor stub, one link: USB-CDC serial at
115200 carrying **QuiltWire v0** 16-byte frames. The laptop/desktop side is
`crates/quilt-wire` (`quilt-wire-peer` binary / `ArrivalPeer`), which stamps
arrivals into `walks/2` JSONL.

## Files

| file | what |
|---|---|
| `quiltwire.h` | QuiltWire v0 codec, portable C99, **byte-identical to the Rust link-core** (`crates/quilt-wire/src/frame.rs`). No Arduino deps. |
| `esp32_cell.ino` | The Arduino sketch: 1 Hz value cell, DELTA/heartbeat/ALARM discipline, own seq, honest retry/backoff. |
| `test_quiltwire.c` | Host-side codec test (golden vector, CRC check, 20k roundtrips, bit-flip rejection) — runs under plain gcc, no hardware. |

## Status — the honesty ledger

- **Host-tested (this repo, no hardware):** `quiltwire.h` via `test_quiltwire.c`
  under gcc; the same bytes verified from Rust in `crates/quilt-wire`.
  The full serial path is proven by the pty loopback integration test
  (`cargo test -p quilt-wire --test pty_loopback`), where a Rust twin of
  this firmware's sender discipline drives a real kernel pty into the
  desktop peer.
- **UNTESTED ON SILICON:** the sketch itself. No board attached. Arduino-ESP32
  glue (Serial/CDC behavior, `min()`, timing) is written for review by eye,
  and gets verified the moment hardware arrives. No claims are made beyond
  that. When it runs on a board, update this file — don't leave stale
  "untested" markers around either.

## Building (when silicon arrives)

Arduino IDE: install the `esp32 by Espressif Systems` board package, select
the board, open `esp32_cell.ino` (keep `quiltwire.h` in the same folder),
Upload, Serial Monitor @ 115200.

PlatformIO (`platformio.ini` next to the sketch):

```ini
[env:cell]
platform = espressif32
framework = arduino
```

Desktop side: `cargo run -p quilt-wire --bin quilt-wire-peer -- --input <serial-port> --output cell.jsonl`
(`--road local --medium usb-cdc` are the defaults; a wired CDC link stamps
`local` — `serial` is a documented candidate for a `walks/3` enum change,
decided then, not now).

## Sender discipline (mirror this if you port it)

- tick at 1 Hz; `DELTA` when |v − last_sent| > 0.05; else `TICK` heartbeat
  after 30 s of silence; `ALARM` every tick while ≥ 85.0 (duplicates fine).
- seq advances **per frame attempted**, sent or dropped — a gap is the
  reliability observation, never a faked send.
- retry/backoff: 3 attempts, 8 ms → 32 ms → 128 ms (cap 512 ms).
- deliberately absent: µs timestamps, sender quality self-reports, routing
  headers, encryption, TLVs-on-serial (subtext is *observed, not declared*).
