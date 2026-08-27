# JOURNAL — the quilt black-box recorder

*Format spec and recovery contract for the crash-safe journal (`quilt journal`
/ `quilt replay` / `quilt journal-verify`). Undersell, overdeliver: this doc
states exactly what the format guarantees and exactly what it does not.*

---

## What it is

An append-only on-disk record of every mutation a sheet ever received
(`set`, `push`, ledger entries, checkpoints), written as it happens, designed
to survive power loss mid-write. At sea the journal is the only witness a
mint has — this is the file that makes the ledger's evidence recoverable.

## Format (version 1, pinned)

```
file   := header frame*
header := MAGIC "QUILTJNL" (8 bytes) | FORMAT_VERSION u16 = 1 | reserved (6 bytes)
frame  := body_len u32 | body (body_len bytes) | crc32 u32 | frame_hash u32
body   := seq u64 | prev_frame_hash [u8;32] | entry_type u8 | payload_len u32 | payload
```

- **Entry types:** `1 = SHEET_META` (the sheet's YAML at start), `2 =
  LEDGER_ENTRY` (a sealed ledger mutation), `3 = CHECKPOINT`.
- **crc32** covers the body — catches torn/garbled frames.
- **frame_hash** chains each frame to its predecessor (genesis frame is
  `sha256("quilt-journal/frames/1")`); reordering or truncation mid-file
  cannot pass silently.
- **Max body 8 MiB.** Larger payloads are refused at write time.
- **fsync before ack** (configurable off for tests via the recorder options).

## Recovery contract

On replay, every truncation of a valid journal at any byte offset produces
exactly one of three honest outcomes — **never a silent wrong state**:

1. **Clean replay** up to the last complete frame (the torn frame's write
   "never happened" — it is truncated and reported).
2. **NOT A JOURNAL** — the header/magic is absent or the first frame is
   unreadable.
3. **CORRUPT** — a frame passes length but fails CRC/body structure (bit
   rot, not a power-loss outcome); reported with frame index and reason.

Divergences between replayed and live state are **reported, never silently
merged**. The property-style test (`tests/journal_power_yank.rs`) truncates a
five-frame journal at **every byte offset** and asserts the outcome matrix.

## CLI

```
quilt journal <sheet.yaml> --out <journal.bin>   # live: every mutation sealed as it lands
quilt replay <journal.bin>                       # rebuild into a fresh engine + verify
quilt journal-verify <journal.bin>               # structural: CRC + chain, no replay
```

## What metal (ESP32 / SD / littlefs) would need

The frame format is already fixed-size-header friendly; a metal port needs
(1) a block-aligned write layer over SD/littlefs (the 4-byte body_len lets
a scanner resync cheaply), (2) fsync → `sd_sync()`/`littlefs` commit, and
(3) the same truncation-matrix test run against real flash. The genesis
and CRC code is plain `no_std`-able arithmetic — port `sha256` from
`ledger.rs`, already present on-metal in quilt-vm-c.

## Evidence

`cargo test -p quilt-core --test journal_power_yank` — 3/3 green
(clean journal replay; full truncation matrix; corrupt-frame reporting).
Full workspace tests green; clippy `-D warnings` clean.
