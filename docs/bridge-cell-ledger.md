# Bridge — the elephant cell-ledger ↔ this repo's ledger vocabulary

*2026-08-22 · companion to [cell-ledger.md](cell-ledger.md),
[field-edge-ledger-bridge.md](field-edge-ledger-bridge.md),
[codespace-cortex.md](codespace-cortex.md).*

This repo specifies the ledger; the **elephant** project
(`projects/elephant`, commit `8440e4d`) now *produces* for it, and the
**crab-traps** relay buffers the result in D1. None of that was visible
from here: zero cross-repo references. This page closes that gap by
mapping their artifact names onto ours, so a reader of `ledger.rs` can
find the running producer in one hop.

## What the elephant cell-ledger is

The elephant's `CellLedgerProducer` (`elephant/cell_ledger.py`) is a
thin seal-only implementation of [cell-ledger.md](cell-ledger.md) §4:
`RoomDaemon.enable_ledger(producer)` attaches it, and every field read
(`warmth`, κ, dial bank) is sealed via `record(...)` into an entry for a
`room.field.<name>` cell, then echoed back into the reading as
`{seq, hash, prev_hash}`. Because each seal is
`sha256(canonical_json(entry minus hash))` including `prev_hash`, the
chain behaves like an Ammann bar: every entry is locally decodable on its
own, yet any single seal pins the **global phase** — edit any entry and
every later seal shifts, so one spot-check certifies the whole prefix.
The crab-traps D1 relay (`worker/src/edge-ledger.ts`, table
`ledger_edges`) is the same contract at fleet grain: the relay is the
sealing authority, producers echo the `chain_head` it hands back.

## Vocabulary map

| elephant / crab-traps term | quilt-rust concept | what differs |
|---|---|---|
| `CellLedgerProducer.record(entry)` — seals only | `CellLedger::record_with(input, expected, …)` | producer is the write-ahead stub: it seals and sequences but posts no double-entry sides and scores no `imbalance` (field reads carry `expected: null`) |
| `seal(entry, prev_hash)`, `genesis_commit(cell_id, ts)` | §4 chain rule, genesis commit, `chain_hash()` | same canonical form (compact, sorted keys); cross-language float hazard is the known one — Python must match ryū shortest-round-trip rendering to stay on-chain |
| `enable_ledger(producer)` on `RoomDaemon` | engine wiring (§9 integration path — not yet landed) | elephant injects at daemon level; here the joint is `QuiltEngine`'s `get/set/call/push` paths + `CallerContext` |
| cell id `room.field.<name>`, `kind: "field"` | `CellId` + `LedgerOrigin` (`push` for remote reads) | elephant namespaces rooms into cell ids by string convention; here ids come from the sheet, provenance carries origin |
| `reading["ledger"] = {seq, hash, prev_hash}` echo | `chain_hash()` head citation | identical idea: the authority hands back the head, the producer echoes it — no canonicalization guesswork on the limb |
| D1 `ledger_edges` row / `EdgeInput {v, cell, ts, before, after, delta, imbalance, provenance, chain}` | `LedgerEntry` (§3) | D1 stores the edge-only projection (no postings, PK `(cell, ts)`); full double-entry lives in `ledger.rs` |
| `POST /edge` → `chain_head`; `GET /queue?since=` | `replay(until_ts)`, Merkle-style head citation | relay buffers while the cortex sleeps ("limb never blocks, brain never listens"); drain is the wake-and-poll contract |
| elephant field-edge `d_mu`, `Δμ̂` | `delta.magnitude`, `imbalance` | already proven identical in [field-edge-ledger-bridge.md](field-edge-ledger-bridge.md) (golden vectors, 1e-12) |

## Adoption points

1. **Engine wiring (§9):** when `ledgers: RwLock<HashMap<CellId,
   CellLedger>>` lands beside `cells`, a `room.field.*` cell is a
   `push`-origin cell whose entries arrive pre-sealed — engine-side
   verification is `verify_chain()` over the replayed prefix, and
   `record_with(input, expected)` is the right call shape for a remote
   producer feed.
2. **Cross-language differential test:** `crates/field-edge-bridge`
   already golden-tests `imbalance ≡ d_mu`; extend the same harness to
   assert the elephant's Python `seal()` reproduces `canonical_json`
   hashes from `ledger.rs` bit-for-bit — the §4 polyformal property,
   now with a live second language.
3. **Cortex drain:** the codespace-cortex wake→think→commit→sleep loop
   can consume `GET /queue?since=` edges as `origin: "push"` entries;
   `replay(until_ts)` gives the no-leakage time cut, and the
   `chain_hash()` head at wake is the citation for corpus provenance.

*Docs-only bridge — no code changed on either side. The seam was already
real; this page just names it in both vocabularies.*
