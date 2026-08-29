# firmware/esp32-cell — the minimal exocortex cell (Rung 5a + 5b)

One ESP32, one temperature-ish sensor stub, one link — **compile-time
selected** among USB-CDC serial (default), ESP-Now, or BLE — carrying
**QuiltWire v0** 16-byte frames, the same frame on every road. The
laptop/desktop side is `crates/quilt-wire` (`quilt-wire-peer` binary /
`ArrivalPeer`), which stamps arrivals into `walks/2` JSONL.

## Files

| file | what |
|---|---|
| `quiltwire.h` | QuiltWire v0 codec, portable C99, **byte-identical to the Rust link-core** (`crates/quilt-wire/src/frame.rs`). No Arduino deps. |
| `qw_transport.h` | Transport abstraction (Rung 5b): `QW_TRANSPORT_USB_CDC` (default) / `_ESPNOW` / `_BLE`, one compile-time select. Same frame on the wire; per-frame RSSI captured on radio roads (ESP-Now recv callback / BLE GAP RSSI read) as the LINKMETA source. |
| `esp32_cell.ino` | The Arduino sketch: 1 Hz value cell, DELTA/heartbeat/ALARM discipline, LINKMETA when the radio reports fresh RSSI, own seq, honest retry/backoff. |
| `test_quiltwire.c` | Host-side codec test (golden vector, CRC check, 20k roundtrips, bit-flip rejection) — runs under plain gcc, no hardware. |

## Status — the honesty ledger

- **Host-tested (this repo, no hardware):** `quiltwire.h` via `test_quiltwire.c`
  under gcc; the same bytes verified from Rust in `crates/quilt-wire`.
  The full serial path is proven by the pty loopback integration test
  (`cargo test -p quilt-wire --test pty_loopback`), where a Rust twin of
  this firmware's sender discipline drives a real kernel pty into the
  desktop peer — including a simulated-lossy radio variant (5% drop +
  reorder, RSSI fed per chunk) proving seq-gap recovery and EWMA
  convergence.
- **UNTESTED ON SILICON:** the sketch and all of `qw_transport.h`. No board
  attached. The USB-CDC path's *bytes* are host-proven; the Arduino/ESP-Now/
  Bluedroid-BLE glue (Serial CDC behavior, `esp_now` callbacks, BLE
  GAP/GATT plumbing, `min()`, timing) is written for review by eye, and
  gets verified the moment hardware arrives. No claims are made beyond
  that. When it runs on a board, update this file — don't leave stale
  "untested" markers around either.

## Building (when silicon arrives)

Arduino IDE: install the `esp32 by Espressif Systems` board package, select
the board, open `esp32_cell.ino` (keep `quiltwire.h` and `qw_transport.h`
in the same folder), Upload, Serial Monitor @ 115200.

PlatformIO (`platformio.ini` next to the sketch); default is USB-CDC,
pick a road with `build_flags`:

```ini
[env:cell]
platform = espressif32
framework = arduino
build_flags = -D QW_TRANSPORT=QW_TRANSPORT_ESPNOW  ; or =QW_TRANSPORT_BLE
```

ESP-Now defaults to a broadcast peer MAC — pin the portal's MAC with
`-D QW_ESPNOW_PEER_MAC={0xAA,0xBB,...}` once the topology is real. BLE
advertises a Nordic-UART-style service (`6E400001-…`, TX notify `…0003`,
RX write `…0002`).

Desktop side: `cargo run -p quilt-wire --bin quilt-wire-peer -- --input <serial-port> --output cell.jsonl`
(`--road local --medium usb-cdc` are the defaults; a wired CDC link stamps
`local` — `serial` is a documented candidate for a `walks/3` enum change,
decided then, not now. Radio portals stamp `--road esp-now` / `--road ble`.)

## Sender discipline (mirror this if you port it)

- tick at 1 Hz; `DELTA` when |v − last_sent| > 0.05; else `TICK` heartbeat
  after 30 s of silence; `ALARM` every tick while ≥ 85.0 (duplicates fine).
- `LINKMETA` when the radio reports a fresh per-frame RSSI (dBm as the f32
  value) — radio roads only; USB-CDC declares nothing about link quality.
- seq advances **per frame attempted**, sent or dropped — a gap is the
  reliability observation, never a faked send.
- retry/backoff: 3 attempts, 8 ms → 32 ms → 128 ms (cap 512 ms).
- deliberately absent: µs timestamps, routing headers, encryption,
  TLVs-on-serial (subtext is *observed, not declared* — RSSI is the one
  sender-side observation, and the desktop stamps its own independently).
