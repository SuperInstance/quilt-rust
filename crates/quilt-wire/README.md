# quilt-wire — QuiltWire v0 link-core + desktop arrival peer

The metal lane's wire format and receiver (EGOCENTRIC-LADDER Rung 5a /
LINK-LAYER-FEASIBILITY §4.2). Transport-blind 16-byte frames; arrivals
stamped receiver-side into `walks/2`-compatible JSONL; firmware twin under
`firmware/esp32-cell/`.

## The frame (16 bytes, pinned)

```text
byte  0    : magic 0x51 ('Q')
byte  1    : version 0x01
byte  2    : kind  0x00 TICK | 0x01 DELTA | 0x02 ALARM | 0x03 LINKMETA | 0x04 ACK
byte  3    : cell id (u8 — demo universe has few cells; portals map ids)
bytes 4-5  : seq (u16 LE, wraps — gap detection = reliability observation)
bytes 6-9  : tick (u32 LE — sender-local; no cross-clock claims)
bytes 10-13: value (f32 LE, raw IEEE-754 bits, any pattern legal)
bytes 14-15: CRC16-CCITT-FALSE over bytes 0..=13
              poly 0x1021, init 0xFFFF, no reflection, xorout 0
              check("123456789") = 0x29B1
```

All multi-byte integers **little-endian** (ESP32/xtensa and hosts are LE).
Golden vector (shared by Rust and C tests):
`510101070201e80300000000ac41d6d3` = DELTA, cell 7, seq 0x0102, tick 1000,
value 21.5.

Optional TLVs after byte 16 when the transport MTU allows (ESP-Now has
234 bytes of headroom): `type u8 | len u8 | value`. TLV `0x01` =
reason + considered-mask, present **only** when the sender had ≥2 live
links — the one declared half of subtext. Unknown TLV types are skipped.
Serial v0 sends bare frames.

Deliberately absent: µs timestamps, sender-quality self-reports, routing
headers, encryption. *Subtext is observed, not declared.*

## Crates map

| module | what | env |
|---|---|---|
| `frame` | layout, CRC16, encode/decode, streaming `FrameDecoder` with resync-on-garbage | no_std, no alloc |
| `seq` | per-sender continuity: contiguous / gap / duplicate / restart (torn walk) | no_std, no alloc |
| `tlv` | TLV tail walking + reason decoding | no_std, no alloc |
| `link` | link-quality EWMA — alpha from half-life in frames (`1 − 2^(−1/h)`), RSSI or delivery-ratio samples | no_std core; half-life→alpha is std |
| `walks` | `walks/2` step construction (sha256-chained, canonical JSON) + verifier mirroring the dissertation exporter | std |
| `peer` | `ArrivalPeer`: bytes in → stamped walks/2 lines out; EWMA link quality (radio: RSSI; wired: delivery ratio) | std |
| bin `quilt-wire-peer` | `--input PATH|- --output PATH|- [--road R] [--medium M]` | std |

Build the core for embedded with `cargo build -p quilt-wire --no-default-features`
(no alloc anywhere in the core paths).

## walks/2 alignment

Lines are byte-compatible with `research/walks-bridge/exporter.py`
(EXPORTER.md §3+§7): six-field digest core (`walk_id, ts, cell_id, opcode,
payload_digest, prev_digest` — sha256 over compact sorted-key canonical JSON),
arrival-path fields (`road`, `link_quality`, `arrival_meta`) outside the
core. The Python exporter's `--verify` passes on this crate's output
(cross-checked; also asserted in Rust by `walks::verify`).

Mapping decisions, documented:

- **opcode**: frame kind `TICK` → `tick` step; every other kind → `effect`
  (inbound arrival receipt).
- **ts** = arrival epoch ms (receiver clock; the frame's sender tick rides in
  `meta.tick` — no cross-clock claims in digested fields).
- **road**: a wired USB-CDC link stamps `local` (`serial` is a walks/3
  question); the enum is closed per §7 — `road` is stamped by the receiver,
  never inferred from payload shape.
- **link_quality**: radio roads carry RSSI EWMA (driver-observed, dBm);
  wired links carry a delivery-ratio EWMA (0..1) penalized by seq gaps —
  Rung 1's "app: latency+loss bucket". The EWMA's alpha derives from a
  half-life in frames: `alpha = 1 − 2^(−1/h)` (see `link`; `PeerConfig`
  takes the alpha). `null` only before any observation.
- **payload** is float-free (`value_bits` u32) so digests are byte-stable
  across languages; the human value renders in undigested `meta`.
- **restart** (backwards seq) tears the walk: new `walk_id` = `cell-N#life`,
  fresh GENESIS — never spliced across the tear.

## Tests (all run green in-sandbox, no hardware)

```text
cargo test -p quilt-wire
  17 unit  (frame golden/CRC, seq wrap/gap/restart, tlv strictness,
            link EWMA half-life/convergence, walks chain + verify,
            peer stamping/gap/restart)
   7 roundtrip props (20k random frames, every single-bit flip rejected,
            every truncation rejected, garbage resync, chunk-size
            invariance, random-bytes never fabricate frames)
   2 pty loopback  — (1) THE METAL PROOF: real kernel pty pair,
            firmware-twin sender (250 frames + injected line noise) →
            desktop peer; asserts 100% decode, seq 0..249 contiguous,
            walks/2 chain verifies, road stamped on every line,
            link_quality present.
            (2) LOSSY RADIO SIM: same pty path behind a simulated
            esp-now transport (5% drop + 2 reordered pairs), RSSI fed
            per chunk like a driver; asserts every loss observed as a
            counted gap, reorder tears the walk (chain still verifies,
            3 walks), road="esp-now" + RSSI EWMA link_quality on every
            line, EWMA converges to the fed mean within ripple bounds.
```

`firmware/esp32-cell/test_quiltwire.c` proves the C codec byte-identical
under gcc (golden vector, 20k roundtrips, bit-flip rejection).

## Honest ledger

- Proven here: codec (two languages, byte-parity), stream resync, walks/2
  chain cross-verified by the Python exporter, pty serial path end-to-end,
  seq-gap recovery and EWMA convergence under simulated radio loss/reorder.
- NOT proven here: anything on silicon. No ESP32 was attached; the sketch is
  written to be reviewed and is marked untested-on-silicon in its header.
  RSSI stamping is exercised by the simulated-lossy pty test, but no real
  radio driver feeds it yet.
