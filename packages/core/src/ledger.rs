//! # ledger.rs
//!
//! The `CellLedger` — a per-cell, append-only, hash-chained, double-entry
//! record of every input→output transaction a cell ever performs.
//!
//! ## Role in the system
//!
//! This is the cell's **first-person memory**. The engine (`engine.rs`)
//! answers "what is the value now?"; the ledger answers "what did this
//! cell *experience*, in order, and how surprising was it?" Each entry
//! pairs the input that arrived with the output that left (double
//! entry), records the before→after edge of the cell's state (the
//! unit of perception — the same shape as the field edge in the
//! elephant and the polyformal kernel's `edge(fb, fa)`), and records
//! the **imbalance** (surprise / prediction-error) as a first-class
//! value.
//!
//! The ledger is a pure data structure: no tokio, no clocks, no I/O.
//! Callers pass timestamps; the ledger chains, hashes, reconciles and
//! replays. That makes it embeddable anywhere the engine runs,
//! serializable into training corpora, and portable across language
//! ports bit-for-bit (see `docs/cell-ledger.md`).
//!
//! ## Depends on
//!
//! - `serde`, `serde_json` — entries and whole ledgers serialize; the
//!   hash chain runs over a canonical JSON form.
//! - `crate::types::CellId` — the cell this ledger belongs to.
//! - `crate::error::{Error, Result}` — typed errors for `settle_*`.
//! - A private, dependency-free SHA-256 (`sha256` module below) so the
//!   chain is tamper-evident and exactly specified across ports.
//!
//! ## Used by
//!
//! - Anything that wants a replayable record of a cell: the engine
//!   (future integration), adapters, training-corpus builders, and
//!   audit tooling.
//!
//! ## Key decisions
//!
//! - **Append-only, hash-chained.** Every entry's hash commits to the
//!   hash of its predecessor plus its own canonical body. Editing any
//!   entry breaks every hash after it. The empty chain's hash commits
//!   to the cell id and genesis state, so identity is in the chain.
//! - **Double entry.** A transaction is complete only when both sides
//!   are posted: an input posting (debit — what the world gave the
//!   cell) and an output posting (credit — what the cell gave back).
//!   `open_input` records a debit without its credit; `reconcile()`
//!   reports such open inputs as an imbalance in the books.
//! - **The imbalance is first-class.** Each entry records the surprise
//!   `distance(expected, actual)`. The default prediction is the
//!   persistence prior (the cell's state before the transaction), in
//!   which case surprise and the edge magnitude coincide — the
//!   elephant's claim that perception *is* the delta. An explicit
//!   `expected` (a model's forecast) is hashed into the entry, so
//!   predictions cannot be rewritten after the fact.
//! - **`expected` is part of the hash.** This is what makes a ledger
//!   usable as honest training data: forecasts are committed before
//!   outcomes are known, on pain of a broken chain.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::types::CellId;

/// A ledger hash: 64 lowercase hex characters (SHA-256).
pub type Hash = String;

/// The all-zeros sentinel never appears in a valid chain; the chain
/// root is instead the *genesis commit* (see [`CellLedger::chain_hash`]).
const GENESIS_KIND: &str = "quilt-cell-ledger/1";

// ---------------------------------------------------------------------------
// SHA-256 — dependency-free, exactly specified
// ---------------------------------------------------------------------------

/// A minimal, dependency-free SHA-256 (FIPS 180-4).
///
/// Implemented inline rather than pulling a hashing crate so that
/// (a) the ledger adds zero dependencies and (b) the hash is pinned
/// by this file — any port of the ledger (TypeScript, Python, ...)
/// reproduces the same chain hashes bit-for-bit by implementing the
/// same standard, the same move the polyformal kernel made with its
/// edge function.
pub mod sha256 {
    /// Round constants (first 32 bits of the fractional parts of the
    /// cube roots of the first 64 primes).
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    /// SHA-256 of `data`, as 32 raw bytes.
    pub fn sha256(data: &[u8]) -> [u8; 32] {
        let mut h: [u32; 8] = [
            0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
            0x5be0cd19,
        ];

        // Padding: 0x80, zeros to 56 mod 64, then the 64-bit big-endian
        // bit length.
        let bit_len = (data.len() as u64).wrapping_mul(8);
        let mut padded = data.to_vec();
        padded.push(0x80);
        while padded.len() % 64 != 56 {
            padded.push(0);
        }
        padded.extend_from_slice(&bit_len.to_be_bytes());

        for block in padded.chunks_exact(64) {
            let mut w = [0u32; 64];
            for i in 0..16 {
                let b = &block[4 * i..4 * i + 4];
                w[i] = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }

            let (mut a, mut b, mut c, mut d) = (h[0], h[1], h[2], h[3]);
            let (mut e, mut f, mut g, mut hh) = (h[4], h[5], h[6], h[7]);

            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ ((!e) & g);
                let t1 = hh
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(maj);
                hh = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }

            h[0] = h[0].wrapping_add(a);
            h[1] = h[1].wrapping_add(b);
            h[2] = h[2].wrapping_add(c);
            h[3] = h[3].wrapping_add(d);
            h[4] = h[4].wrapping_add(e);
            h[5] = h[5].wrapping_add(f);
            h[6] = h[6].wrapping_add(g);
            h[7] = h[7].wrapping_add(hh);
        }

        let mut out = [0u8; 32];
        for (i, word) in h.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    /// SHA-256 of `data`, as 64 lowercase hex characters.
    pub fn hex(data: &[u8]) -> String {
        sha256(data).iter().map(|b| format!("{b:02x}")).collect()
    }
}

// ---------------------------------------------------------------------------
// Canonical JSON — the hash preimage form
// ---------------------------------------------------------------------------

/// Canonical JSON: compact, with object keys sorted by UTF-8 byte
/// order, independent of map insertion order. Numbers render via
/// serde_json semantics: integers as integers, floats as Rust's
/// shortest-round-trip form. This exact form is pinned in
/// `docs/cell-ledger.md` so ports reproduce the chain bit-for-bit.
pub fn canonical_json(v: &Value) -> String {
    let mut out = String::new();
    write_canonical(v, &mut out);
    out
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&canonical_number(n)),
        Value::String(s) => {
            // Serializing a bare string cannot fail.
            out.push_str(&serde_json::to_string(s).expect("string -> json is infallible"))
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key.as_str()).expect("infallible"));
                out.push(':');
                write_canonical(map.get(*key).expect("key came from the map"), out);
            }
            out.push('}');
        }
    }
}

fn canonical_number(n: &serde_json::Number) -> String {
    // serde_json / ryū semantics, as pinned in docs/cell-ledger.md §4:
    // integers render as integers, floats as the shortest-round-trip
    // decimal that keeps the float marker (85.0 -> "85.0", not "85").
    // Preserving the float/int distinction is what lets JS/Python ports
    // stay on-chain bit-for-bit.
    n.to_string()
}

// ---------------------------------------------------------------------------
// Distance — the edge / surprise metric
// ---------------------------------------------------------------------------

/// A total metric between two JSON values. This is the ledger's
/// generic `d_mu`: the magnitude of an edge, and the magnitude of a
/// surprise.
///
/// - numbers: `|a - b|`
/// - equal values (any type): `0`
/// - arrays: mean of element-wise distances; missing elements count
///   as full distance (`1.0`) so length changes are visible
/// - objects: mean over the key union; missing keys count `1.0`
/// - anything else (type mismatch, string vs number, ...): `1.0`
pub fn value_distance(a: &Value, b: &Value) -> f64 {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap_or(0.0), y.as_f64().unwrap_or(0.0));
            (x - y).abs()
        }
        (Value::Array(xs), Value::Array(ys)) => {
            let n = xs.len().max(ys.len());
            if n == 0 {
                0.0
            } else {
                let sum: f64 = (0..n)
                    .map(|i| match (xs.get(i), ys.get(i)) {
                        (Some(x), Some(y)) => value_distance(x, y),
                        _ => 1.0,
                    })
                    .sum();
                sum / n as f64
            }
        }
        (Value::Object(xm), Value::Object(ym)) => {
            let keys: BTreeSet<&String> = xm.keys().chain(ym.keys()).collect();
            if keys.is_empty() {
                0.0
            } else {
                let sum: f64 = keys
                    .iter()
                    .map(|k| match (xm.get(*k), ym.get(*k)) {
                        (Some(x), Some(y)) => value_distance(x, y),
                        _ => 1.0,
                    })
                    .sum();
                sum / keys.len() as f64
            }
        }
        (a, b) if a == b => 0.0,
        _ => 1.0,
    }
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

/// Which side of the double entry a posting sits on.
///
/// Accounting vocabulary: the input posting is the *debit* (what the
/// world gave the cell), the output posting is the *credit* (what the
/// cell gave back). A transaction balances when both are posted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntrySide {
    /// The debit side — what flowed into the cell.
    Input,
    /// The credit side — what flowed out of the cell.
    Output,
}

/// What caused the transaction. Maps one-to-one onto the engine's
/// universal verbs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerOrigin {
    /// A `get` evaluation.
    Get,
    /// A `set` write.
    Set,
    /// A `call` invocation (with input).
    Call,
    /// A `push` into a sensor/io cell.
    Push,
    /// Anything else: engine-internal, adapters, replay tooling.
    System,
}

/// Where a transaction came from — the first-person "who touched me".
///
/// Mirrors the essential fields of `types::CallerContext` (caller +
/// trace + which verb), kept small so entries stay cheap and the
/// canonical form stays portable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    /// Which engine verb triggered this transaction.
    pub origin: LedgerOrigin,
    /// The cell (or client) that initiated it, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller: Option<CellId>,
    /// The ancestor chain, outermost first (as in `CallerContext`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trace: Vec<CellId>,
}

impl Default for Provenance {
    fn default() -> Self {
        Self {
            origin: LedgerOrigin::System,
            caller: None,
            trace: Vec::new(),
        }
    }
}

/// One side of a double entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Posting {
    /// Which side of the entry this posting is.
    pub side: EntrySide,
    /// The value that flowed.
    pub value: Value,
    /// When this side was posted (millis since epoch).
    pub ts: u64,
}

/// The before→after edge of a transaction — the cell-grain instance
/// of the field edge (`field_before → field_after`). The change, not
/// the state: the unit of perception.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delta {
    /// The cell's state before the transaction.
    pub before: Value,
    /// The cell's state after the transaction.
    pub after: Value,
    /// Did the state change at all (`before != after`).
    pub changed: bool,
    /// `value_distance(before, after)` — the magnitude of the edge,
    /// the ledger's generic `d_mu`.
    pub magnitude: f64,
}

impl Delta {
    fn compute(before: &Value, after: &Value) -> Self {
        Self {
            before: before.clone(),
            after: after.clone(),
            changed: before != after,
            magnitude: value_distance(before, after),
        }
    }
}

/// One complete double entry: an input posting, its matching output
/// posting, the state edge it caused, the prediction it was scored
/// against, and its place in the hash chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    /// Sequence number, assigned at append, contiguous from 1 in
    /// chain order.
    pub seq: u64,
    /// Primary timestamp (the input posting's time; the transaction
    /// begins when the world arrives).
    pub ts: u64,
    /// The debit side — what the world gave the cell.
    pub input: Posting,
    /// The credit side — what the cell gave back.
    pub output: Posting,
    /// Who/what caused this transaction.
    pub provenance: Provenance,
    /// The before→after edge of the cell's state.
    pub delta: Delta,
    /// The prediction this entry was scored against. `None` only when
    /// no prior existed (first entry of a genesis-less ledger).
    /// Hashed into `hash` — forecasts cannot be rewritten later.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<Value>,
    /// The surprise: `value_distance(expected, output)`. Under the
    /// default persistence prior (`expected == before`) this equals
    /// `delta.magnitude`. `None` iff `expected` is `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imbalance: Option<f64>,
    /// Hash of the previous entry (or the genesis commit for the
    /// first entry).
    pub prev_hash: Hash,
    /// `sha256(canonical_json(this entry minus the hash field))`.
    pub hash: Hash,
}

impl LedgerEntry {
    /// Recompute this entry's seal from its body. Any edit to any
    /// hashed field changes the result — the tamper-evidence check.
    fn seal(&self) -> Hash {
        let mut body = serde_json::to_value(self)
            .expect("LedgerEntry is plain data; serialization is infallible");
        if let Value::Object(ref mut map) = body {
            map.remove("hash");
        }
        sha256::hex(canonical_json(&body).as_bytes())
    }
}

/// An input that has been posted (debit) but not yet answered
/// (credit). The ledger's "owed" column: `reconcile()` counts these
/// as open inputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingInput {
    /// The open ticket. Pass to `settle_output` to close.
    pub ticket: u64,
    /// When the input arrived (millis since epoch).
    pub ts: u64,
    /// The value that flowed in.
    pub input: Value,
    /// Where it came from.
    pub provenance: Provenance,
}

// ---------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------

/// The result of walking the hash chain and recomputing every seal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainAudit {
    /// Number of entries whose seals were recomputed.
    pub verified: usize,
    /// True when every prev-link and every seal checks out.
    pub intact: bool,
    /// The sequence number of the first entry that failed, if any.
    pub first_break: Option<u64>,
}

/// The result of [`CellLedger::reconcile`] — the books: do the two
/// sides of every transaction meet, is the chain honest, is the
/// history continuous, and how much surprise has accumulated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reconciliation {
    /// The cell this report is about.
    pub cell_id: CellId,
    /// Completed (double-posted) entries in the chain.
    pub entries: usize,
    /// Inputs posted but not yet answered. Nonzero means the books
    /// do not balance: the cell owes the world a response.
    pub open_inputs: usize,
    /// Entries carrying both an input and an output posting.
    pub matched_pairs: usize,
    /// Did the hash chain verify end to end?
    pub chain_intact: bool,
    /// Sequence number of the first chain break, if any.
    pub first_break: Option<u64>,
    /// Is the history continuous — each entry's `before` equal to
    /// its predecessor's `after` (and to the genesis for the first)?
    pub continuity_intact: bool,
    /// Sum of every entry's imbalance. The cell's total surprise.
    pub total_surprise: f64,
    /// Mean imbalance over entries that carry one (`None` if none do).
    pub mean_surprise: Option<f64>,
    /// True iff no open inputs, all pairs matched, chain intact, and
    /// history continuous. The ledger balances.
    pub balanced: bool,
}

/// The result of [`CellLedger::replay`] — a point-in-time view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Replay {
    /// The cell this replay is about.
    pub cell_id: CellId,
    /// The cutoff that was requested (inclusive).
    pub until_ts: u64,
    /// How many entries fell at or before the cutoff.
    pub replayed: usize,
    /// The reconstructed state at `until_ts`: the `after` of the last
    /// entry at or before the cutoff, else the genesis, else `null`.
    pub state: Value,
    /// Cumulative surprise of the replayed prefix. A cell's surprise
    /// *so far* — the first-person view up to that moment.
    pub surprise: f64,
    /// The replayed entries, in chain order. These are the
    /// `(input, expected, output, surprise)` tuples a corpus wants;
    /// re-running them through a cell evaluator replays computation,
    /// not just state.
    pub entries: Vec<LedgerEntry>,
}

// ---------------------------------------------------------------------------
// The ledger
// ---------------------------------------------------------------------------

/// A per-cell, append-only, hash-chained, double-entry ledger.
///
/// Construct one per cell (`new` for a fresh cell, `with_genesis` to
/// seed a known initial state), then `record` transactions — or
/// `open_input` / `settle_output` when the answer arrives later than
/// the question (async cells). `reconcile` audits the books,
/// `replay` reconstructs any past state, `chain_hash` is the
/// tamper-evident commitment to everything the cell ever did.
///
/// Pure data: no clocks (callers pass timestamps), no I/O, no async.
/// Serializes wholesale via serde — that is how ledgers aggregate
/// into corpora.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellLedger {
    cell_id: CellId,
    /// The genesis state, if one was declared. Committed by the
    /// chain root, so it cannot be swapped later.
    genesis: Option<Value>,
    /// When the genesis state was declared (millis since epoch).
    genesis_ts: Option<u64>,
    /// The cell's current state (the `after` of the last entry, or
    /// the genesis).
    state: Value,
    next_seq: u64,
    next_ticket: u64,
    entries: Vec<LedgerEntry>,
    pending: Vec<PendingInput>,
}

impl CellLedger {
    /// A fresh ledger for `cell_id`, state `null`, no genesis.
    pub fn new(cell_id: impl Into<CellId>) -> Self {
        Self {
            cell_id: cell_id.into(),
            genesis: None,
            genesis_ts: None,
            state: Value::Null,
            next_seq: 1,
            next_ticket: 1,
            entries: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// A ledger whose cell began life at a known state. The genesis
    /// (value and timestamp) is committed by the chain root hash and
    /// scores the very first transaction against the persistence
    /// prior.
    pub fn with_genesis(cell_id: impl Into<CellId>, genesis: Value, ts: u64) -> Self {
        Self {
            cell_id: cell_id.into(),
            genesis: Some(genesis.clone()),
            genesis_ts: Some(ts),
            state: genesis,
            next_seq: 1,
            next_ticket: 1,
            entries: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// The cell this ledger belongs to.
    pub fn cell_id(&self) -> &str {
        &self.cell_id
    }

    /// The current state (the `after` of the last entry, else the
    /// genesis, else `null`).
    pub fn state(&self) -> &Value {
        &self.state
    }

    /// Number of completed entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no completed entries exist (pending inputs may).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// All completed entries, in chain order.
    pub fn entries(&self) -> &[LedgerEntry] {
        &self.entries
    }

    /// The head (most recent) entry, if any.
    pub fn head(&self) -> Option<&LedgerEntry> {
        self.entries.last()
    }

    /// Inputs posted but not yet settled.
    pub fn pending(&self) -> &[PendingInput] {
        &self.pending
    }

    // -- Recording ---------------------------------------------------------

    /// Record a complete double entry: `input` in, `output` out, at
    /// `ts` (millis since epoch), system provenance, persistence
    /// prediction. Returns the sealed entry.
    ///
    /// Convenience for the common sync path; use `record_with` to
    /// attach provenance or an explicit prediction, and
    /// `open_input`/`settle_output` when output arrives later.
    pub fn record(
        &mut self,
        input: impl Into<Value>,
        output: impl Into<Value>,
        ts: u64,
    ) -> LedgerEntry {
        self.record_with(input, output, ts, Provenance::default(), None)
    }

    /// `record` with full control over provenance and prediction.
    ///
    /// `expected` is the forecast the output is scored against. When
    /// `None`, the persistence prior is used: the cell's state before
    /// the transaction (so surprise == edge magnitude). When a prior
    /// exists it is recorded — and hashed — either way.
    pub fn record_with(
        &mut self,
        input: impl Into<Value>,
        output: impl Into<Value>,
        ts: u64,
        provenance: Provenance,
        expected: Option<Value>,
    ) -> LedgerEntry {
        let (input, output) = (input.into(), output.into());
        self.append_entry(input, ts, output, ts, provenance, expected)
    }

    /// Post an input (debit) without its answer. Returns the ticket
    /// to pass to `settle_output`. The open input does not move the
    /// cell's state and is not in the chain; `reconcile` counts it.
    pub fn open_input(&mut self, input: impl Into<Value>, ts: u64) -> u64 {
        self.open_input_with(input, ts, Provenance::default())
    }

    /// `open_input` with provenance.
    pub fn open_input_with(
        &mut self,
        input: impl Into<Value>,
        ts: u64,
        provenance: Provenance,
    ) -> u64 {
        let ticket = self.next_ticket;
        self.next_ticket += 1;
        self.pending.push(PendingInput {
            ticket,
            ts,
            input: input.into(),
            provenance,
        });
        ticket
    }

    /// Settle an open input with its output (credit), closing the
    /// double entry and appending it to the chain.
    pub fn settle_output(
        &mut self,
        ticket: u64,
        output: impl Into<Value>,
        ts: u64,
    ) -> Result<LedgerEntry> {
        self.settle_output_with(ticket, output, ts, None)
    }

    /// `settle_output` with an explicit prediction.
    pub fn settle_output_with(
        &mut self,
        ticket: u64,
        output: impl Into<Value>,
        ts: u64,
        expected: Option<Value>,
    ) -> Result<LedgerEntry> {
        let position = self
            .pending
            .iter()
            .position(|p| p.ticket == ticket)
            .ok_or_else(|| {
                Error::other(format!(
                    "ledger '{}': no open input with ticket {}",
                    self.cell_id, ticket
                ))
            })?;
        let pending = self.pending.remove(position);
        Ok(self.append_entry(
            pending.input,
            pending.ts,
            output.into(),
            ts,
            pending.provenance,
            expected,
        ))
    }

    /// The one place entries are born. Computes the edge, scores the
    /// prediction, seals the hash, advances the state.
    fn append_entry(
        &mut self,
        input: Value,
        input_ts: u64,
        output: Value,
        output_ts: u64,
        provenance: Provenance,
        expected: Option<Value>,
    ) -> LedgerEntry {
        let before = self.state.clone();
        let after = output.clone();
        let delta = Delta::compute(&before, &after);

        // A prior exists if the cell had a genesis or has already
        // completed an entry. Without one, no surprise is claimed.
        let has_prior = self.genesis.is_some() || !self.entries.is_empty();
        let (expected, imbalance) = match expected {
            Some(e) => {
                let surprise = value_distance(&e, &after);
                (Some(e), Some(surprise))
            }
            None if has_prior => (Some(before.clone()), Some(delta.magnitude)),
            None => (None, None),
        };

        let seq = self.next_seq;
        self.next_seq += 1;
        let prev_hash = self.chain_hash();

        let mut entry = LedgerEntry {
            seq,
            ts: input_ts,
            input: Posting {
                side: EntrySide::Input,
                value: input,
                ts: input_ts,
            },
            output: Posting {
                side: EntrySide::Output,
                value: output,
                ts: output_ts,
            },
            provenance,
            delta,
            expected,
            imbalance,
            prev_hash,
            hash: String::new(),
        };
        entry.hash = entry.seal();

        self.state = after;
        self.entries.push(entry.clone());
        entry
    }

    // -- Hashing -----------------------------------------------------------

    /// The head of the chain: the last entry's hash, or — for an
    /// empty ledger — the genesis commit `sha256(canonical({"kind":
    /// "quilt-cell-ledger/1", "cell_id": ..., "genesis": ...}))`.
    ///
    /// This single value commits to the cell's identity, its initial
    /// state, and every transaction it ever recorded.
    pub fn chain_hash(&self) -> Hash {
        match self.entries.last() {
            Some(head) => head.hash.clone(),
            None => self.genesis_commit(),
        }
    }

    fn genesis_commit(&self) -> Hash {
        let body = serde_json::json!({
            "kind": GENESIS_KIND,
            "cell_id": self.cell_id,
            "genesis": self.genesis.clone().unwrap_or(Value::Null),
            "genesis_ts": self.genesis_ts,
        });
        sha256::hex(canonical_json(&body).as_bytes())
    }

    /// Recompute every seal and every prev-link. `intact == false`
    /// means some entry (at `first_break`) was added, removed, or
    /// edited after the fact.
    pub fn verify_chain(&self) -> ChainAudit {
        let mut expected_prev = self.genesis_commit();
        for entry in &self.entries {
            if entry.prev_hash != expected_prev || entry.hash != entry.seal() {
                return ChainAudit {
                    verified: (entry.seq - 1) as usize,
                    intact: false,
                    first_break: Some(entry.seq),
                };
            }
            expected_prev = entry.hash.clone();
        }
        ChainAudit {
            verified: self.entries.len(),
            intact: true,
            first_break: None,
        }
    }

    // -- Auditing ----------------------------------------------------------

    /// Reconcile the books. Checks, in order:
    ///
    /// 1. **Matching** — every completed entry carries both an input
    ///    and an output posting (`matched_pairs == entries`).
    /// 2. **Open inputs** — debits posted without credits. Nonzero
    ///    means the cell owes the world answers.
    /// 3. **Chain** — every seal and prev-link verifies.
    /// 4. **Continuity** — each entry's `before` equals its
    ///    predecessor's `after` (and the first equals the genesis).
    ///
    /// Then totals the surprise. `balanced` requires all four.
    pub fn reconcile(&self) -> Reconciliation {
        let audit = self.verify_chain();

        let mut continuity_intact = true;
        let mut prior: Option<&Value> = self.genesis.as_ref();
        for entry in &self.entries {
            let expected_before = prior.unwrap_or(&Value::Null);
            if &entry.delta.before != expected_before {
                continuity_intact = false;
                break;
            }
            prior = Some(&entry.delta.after);
        }

        let matched_pairs = self
            .entries
            .iter()
            .filter(|e| e.input.side == EntrySide::Input && e.output.side == EntrySide::Output)
            .count();

        let scored: Vec<f64> = self.entries.iter().filter_map(|e| e.imbalance).collect();
        let total_surprise: f64 = scored.iter().sum();
        let mean_surprise = if scored.is_empty() {
            None
        } else {
            Some(total_surprise / scored.len() as f64)
        };

        Reconciliation {
            cell_id: self.cell_id.clone(),
            entries: self.entries.len(),
            open_inputs: self.pending.len(),
            matched_pairs,
            chain_intact: audit.intact,
            first_break: audit.first_break,
            continuity_intact,
            total_surprise,
            mean_surprise,
            balanced: self.pending.is_empty()
                && matched_pairs == self.entries.len()
                && audit.intact
                && continuity_intact,
        }
    }

    // -- Replay ------------------------------------------------------------

    /// Replay the cell's history up to and including `until_ts`.
    ///
    /// Returns the entries at or before the cutoff, the state
    /// reconstructed from them (the `after` of the last, else the
    /// genesis, else `null`), and the cumulative surprise of the
    /// prefix — the cell's first-person view at that moment.
    ///
    /// This is state replay. Because every entry also carries its
    /// input, a caller can go further: feed the entry inputs to a
    /// cell evaluator (live or modified) and compare against the
    /// recorded outputs — computation replay. The ledger is the
    /// ground truth both are scored against.
    pub fn replay(&self, until_ts: u64) -> Replay {
        let entries: Vec<LedgerEntry> = self
            .entries
            .iter()
            .filter(|e| e.ts <= until_ts)
            .cloned()
            .collect();
        let state = entries
            .last()
            .map(|e| e.delta.after.clone())
            .unwrap_or_else(|| self.genesis.clone().unwrap_or(Value::Null));
        let surprise = entries.iter().filter_map(|e| e.imbalance).sum();
        Replay {
            cell_id: self.cell_id.clone(),
            until_ts,
            replayed: entries.len(),
            state,
            surprise,
            entries,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- SHA-256 correctness ------------------------------------------------

    #[test]
    fn sha256_matches_known_vectors() {
        // FIPS 180-4 / NIST vectors.
        assert_eq!(
            sha256::hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256::hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256::hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    // -- Recording ----------------------------------------------------------

    #[test]
    fn record_pairs_input_and_output_and_computes_the_edge() {
        let mut ledger = CellLedger::with_genesis("bilge.level", json!(40.0), 1_000);
        let entry = ledger.record(json!(85.0), json!(85.0), 2_000);

        assert_eq!(entry.seq, 1);
        assert_eq!(entry.input.side, EntrySide::Input);
        assert_eq!(entry.output.side, EntrySide::Output);
        assert_eq!(entry.input.value, json!(85.0));
        assert_eq!(entry.output.value, json!(85.0));
        // The edge: 40 -> 85.
        assert_eq!(entry.delta.before, json!(40.0));
        assert_eq!(entry.delta.after, json!(85.0));
        assert!(entry.delta.changed);
        assert!((entry.delta.magnitude - 45.0).abs() < 1e-12);
        // Persistence prior: expected == before, surprise == edge.
        assert_eq!(entry.expected, Some(json!(40.0)));
        assert!((entry.imbalance.unwrap() - 45.0).abs() < 1e-12);
        // State advanced.
        assert_eq!(ledger.state(), &json!(85.0));
    }

    #[test]
    fn provenance_is_captured() {
        let mut ledger = CellLedger::new("pump.relay");
        let entry = ledger.record_with(
            json!(true),
            json!(true),
            7,
            Provenance {
                origin: LedgerOrigin::Set,
                caller: Some("alarm.listener".into()),
                trace: vec!["bilge.level".into(), "pump.should_run".into()],
            },
            None,
        );
        assert_eq!(entry.provenance.origin, LedgerOrigin::Set);
        assert_eq!(entry.provenance.caller.as_deref(), Some("alarm.listener"));
        assert_eq!(entry.provenance.trace.len(), 2);
    }

    #[test]
    fn first_entry_without_genesis_claims_no_surprise() {
        let mut ledger = CellLedger::new("fresh.cell");
        let entry = ledger.record(json!("in"), json!("out"), 10);
        // No prior existed, so no prediction and no surprise claim.
        assert_eq!(entry.expected, None);
        assert_eq!(entry.imbalance, None);
        // The edge still exists: null -> "out".
        assert!(entry.delta.changed);
    }

    #[test]
    fn explicit_prediction_separates_surprise_from_edge() {
        let mut ledger = CellLedger::with_genesis("forecast.cell", json!(40.0), 0);
        let entry = ledger.record_with(
            json!({"q": "hi"}),
            json!(60.0),
            100,
            Provenance::default(),
            Some(json!(50.0)),
        );
        // Edge is 40 -> 60 (magnitude 20), surprise is 50 vs 60 (= 10).
        assert!((entry.delta.magnitude - 20.0).abs() < 1e-12);
        assert!((entry.imbalance.unwrap() - 10.0).abs() < 1e-12);
    }

    // -- The chain ----------------------------------------------------------

    #[test]
    fn identical_histories_produce_identical_chain_hashes() {
        let build = || {
            let mut l = CellLedger::with_genesis("compass.heading", json!(180.0), 0);
            l.record(json!(185.0), json!(185.0), 1_000);
            l.record(json!(190.0), json!(190.0), 2_000);
            l
        };
        // Bit-for-bit determinism — the polyformal property, at cell grain.
        assert_eq!(build().chain_hash(), build().chain_hash());
        // Different cell id -> different chain root -> different head.
        let mut other = CellLedger::with_genesis("other.cell", json!(180.0), 0);
        other.record(json!(185.0), json!(185.0), 1_000);
        other.record(json!(190.0), json!(190.0), 2_000);
        assert_ne!(build().chain_hash(), other.chain_hash());
    }

    #[test]
    fn tampering_with_an_entry_breaks_the_chain() {
        let mut ledger = CellLedger::with_genesis("sensor.a", json!(1.0), 0);
        ledger.record(json!(2.0), json!(2.0), 1_000);
        ledger.record(json!(3.0), json!(3.0), 2_000);
        ledger.record(json!(4.0), json!(4.0), 3_000);

        let clean = ledger.verify_chain();
        assert!(clean.intact);
        assert_eq!(clean.verified, 3);

        // Rewrite history: entry 2 claims it output 99. The seal on
        // entry 2 no longer verifies, and so the chain is broken from
        // there on — even though entry 3 itself was never touched.
        ledger.entries[1].output.value = json!(99.0);
        let audit = ledger.verify_chain();
        assert!(!audit.intact);
        assert_eq!(audit.first_break, Some(2));
    }

    #[test]
    fn swapping_genesis_breaks_the_chain() {
        let mut ledger = CellLedger::with_genesis("sensor.b", json!(10.0), 0);
        ledger.record(json!(11.0), json!(11.0), 1_000);
        assert!(ledger.verify_chain().intact);

        // The genesis is committed by the chain root: relabeling the
        // starting state invalidates the first prev-link.
        ledger.genesis = Some(json!(999.0));
        let audit = ledger.verify_chain();
        assert!(!audit.intact);
        assert_eq!(audit.first_break, Some(1));
    }

    // -- Reconciliation -----------------------------------------------------

    #[test]
    fn reconcile_balances_a_clean_ledger_and_totals_surprise() {
        let mut ledger = CellLedger::with_genesis("cell.x", json!(0.0), 0);
        ledger.record(json!(10.0), json!(10.0), 1_000); // surprise 10
        ledger.record(json!(10.0), json!(10.0), 2_000); // surprise 0
        ledger.record(json!(13.0), json!(13.0), 3_000); // surprise 3

        let rec = ledger.reconcile();
        assert!(rec.balanced);
        assert!(rec.chain_intact);
        assert!(rec.continuity_intact);
        assert_eq!(rec.entries, 3);
        assert_eq!(rec.matched_pairs, 3);
        assert_eq!(rec.open_inputs, 0);
        assert!((rec.total_surprise - 13.0).abs() < 1e-12);
        assert!((rec.mean_surprise.unwrap() - 13.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn open_inputs_do_not_balance() {
        let mut ledger = CellLedger::new("slow.cell");
        let ticket = ledger.open_input(json!({"request": 1}), 5_000);

        let rec = ledger.reconcile();
        assert!(!rec.balanced);
        assert_eq!(rec.open_inputs, 1);

        // State is not moved by an unanswered input.
        assert_eq!(ledger.state(), &Value::Null);

        let entry = ledger
            .settle_output(ticket, json!({"answer": 42}), 5_050)
            .unwrap();
        assert_eq!(entry.input.value, json!({"request": 1}));
        assert_eq!(entry.output.value, json!({"answer": 42}));
        assert_eq!(entry.input.ts, 5_000);
        assert_eq!(entry.output.ts, 5_050);

        let rec = ledger.reconcile();
        assert!(rec.balanced);
        assert_eq!(rec.open_inputs, 0);
        assert_eq!(ledger.state(), &json!({"answer": 42}));
    }

    #[test]
    fn settling_an_unknown_or_used_ticket_errors() {
        let mut ledger = CellLedger::new("cell.y");
        let ticket = ledger.open_input(json!(1), 100);
        assert!(ledger.settle_output(999, json!(0), 101).is_err());
        assert!(ledger.settle_output(ticket, json!(0), 101).is_ok());
        // Already consumed.
        assert!(ledger.settle_output(ticket, json!(0), 102).is_err());
    }

    #[test]
    fn discontinuous_history_is_detected() {
        let mut ledger = CellLedger::with_genesis("cell.z", json!(0.0), 0);
        ledger.record(json!(5.0), json!(5.0), 1_000);
        ledger.record(json!(6.0), json!(6.0), 2_000);

        // Forge a past: splice entry 1's after without resealing. Both
        // the chain seal and the continuity check fire.
        ledger.entries[0].delta.after = json!(4.0);
        let rec = ledger.reconcile();
        assert!(!rec.chain_intact);
        assert!(!rec.continuity_intact);
        assert!(!rec.balanced);
    }

    // -- Replay -------------------------------------------------------------

    #[test]
    fn replay_reconstructs_past_states_and_surprise() {
        let mut ledger = CellLedger::with_genesis("cell.t", json!(40.0), 0);
        ledger.record(json!(50.0), json!(50.0), 1_000); // surprise 10
        ledger.record(json!(80.0), json!(80.0), 2_000); // surprise 30
        ledger.record(json!(90.0), json!(90.0), 3_000); // surprise 10

        // Before any entry: the genesis.
        let r0 = ledger.replay(999);
        assert_eq!(r0.state, json!(40.0));
        assert_eq!(r0.replayed, 0);
        assert!((r0.surprise - 0.0).abs() < 1e-12);

        // At t=2000 (inclusive): state 80, surprise 40 so far.
        let r2 = ledger.replay(2_000);
        assert_eq!(r2.state, json!(80.0));
        assert_eq!(r2.replayed, 2);
        assert!((r2.surprise - 40.0).abs() < 1e-12);

        // The end of history.
        let r_all = ledger.replay(u64::MAX);
        assert_eq!(r_all.state, json!(90.0));
        assert_eq!(r_all.replayed, 3);
        assert!((r_all.surprise - 50.0).abs() < 1e-12);
    }

    // -- Serialization / corpora ---------------------------------------------

    #[test]
    fn ledger_round_trips_through_json() {
        let mut ledger = CellLedger::with_genesis("cell.s", json!(7.0), 0);
        ledger.record(json!(8.0), json!(8.0), 1_000);
        let t = ledger.open_input(json!(9), 2_000);
        ledger.settle_output(t, json!(9.0), 2_100).unwrap();

        let value = serde_json::to_value(&ledger).unwrap();
        let restored: CellLedger = serde_json::from_value(value).unwrap();

        assert_eq!(restored.cell_id(), "cell.s");
        assert_eq!(restored.len(), 2);
        assert_eq!(restored.chain_hash(), ledger.chain_hash());
        assert!(restored.verify_chain().intact);
        assert!(restored.reconcile().balanced);
        assert_eq!(restored.state(), &json!(9.0));
    }

    #[test]
    fn canonical_json_sorts_keys_and_pins_numbers() {
        assert_eq!(
            canonical_json(&json!({"b": 1, "a": [2.5, true, null, "x"]})),
            r#"{"a":[2.5,true,null,"x"],"b":1}"#
        );
        // Same value, different insertion order -> same canonical form.
        let v1 = serde_json::from_str::<Value>(r#"{"x":1,"y":2}"#).unwrap();
        let v2 = serde_json::from_str::<Value>(r#"{"y":2,"x":1}"#).unwrap();
        assert_eq!(canonical_json(&v1), canonical_json(&v2));
    }

    #[test]
    fn value_distance_handles_numbers_structures_and_type_shifts() {
        assert_eq!(value_distance(&json!(3.0), &json!(5.0)), 2.0);
        assert_eq!(value_distance(&json!(true), &json!(true)), 0.0);
        // Type shift: full surprise.
        assert_eq!(value_distance(&json!(false), &json!(0)), 1.0);
        // Arrays: mean of element-wise distances.
        assert!((value_distance(&json!([1.0, 2.0]), &json!([3.0, 2.0])) - 1.0).abs() < 1e-12);
        // Length mismatch: missing element costs full distance (mean of 1,0,1).
        assert!((value_distance(&json!([1.0]), &json!([1.0, 5.0])) - 0.5).abs() < 1e-12);
        // Objects: union of keys, missing key costs 1.0.
        assert!((value_distance(&json!({"a": 1}), &json!({"a": 1, "b": 2})) - 0.5).abs() < 1e-12);
        assert_eq!(value_distance(&json!({}), &json!({})), 0.0);
    }
}
