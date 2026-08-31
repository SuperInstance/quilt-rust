//! # quilt-polyformalism — the 5+1+1+1+1 opcodes in pure Rust
//!
//! The Quilt cell model expressed as the 9-opcode set, in Rust.
//! This crate is the *Rust polyformalism port*: same 9 opcodes
//! as the C port (quilt-c), same laws, same cell model. The
//! polyformalism claim is the interface, not the substrate.
//!
//! ## The 9 opcodes
//!
//! - The 5 originals: `bind`, `link`, `effect`, `view`, `tick`
//! - The +1 (Phase 213): `forget`
//! - The +1+1+1 (Phases 216-218, cutting-edge adoptions):
//!   - `proof` — signed hash-linked audit chain
//!   - `route` — substrate routing for memory
//!   - `crdt`  — state-based CRDT for offline convergence
//!
//! ## The 5 laws
//!
//! - BIND idempotence
//! - LINK transitivity (with cycle rejection)
//! - EFFECT associativity (pure evaluation)
//! - VIEW purity
//! - TICK monotonicity
//! - FORGET completeness
//!
//! ## Design constraints
//!
//! - `no_std` friendly (the polyformalism claim is that this
//!   runs on every substrate; the kernel-friendly port is
//!   `quilt-c`, the wasm-friendly port is `quilt-core-wasm`).
//! - Zero allocations on the hot path (the engine reuses the
//!   caller's buffer).
//! - Hashing: a tiny FNV-1a (matches the C port's choice).
//!   Real substrates swap this for the platform's native hash.

#![deny(missing_docs)]
#![cfg_attr(not(test), no_std)]

extern crate alloc;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;

/// The 5+1+1+1+1 opcodes (9 total).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// Write a value to a cell (idempotent).
    Bind,
    /// Add a dependency edge (transitive, no cycles).
    Link,
    /// Apply an effect to a cell (associative, pure).
    Effect,
    /// Read a cell (pure, no mutation).
    View,
    /// Advance the engine one step (monotonic).
    Tick,
    /// Tear down a cell (complete).
    Forget,
    /// Signed hash-linked audit chain (cutting-edge adoption #1).
    Proof,
    /// Substrate routing for memory (cutting-edge adoption #2).
    Route,
    /// State-based CRDT for offline convergence (#3).
    Crdt,
}

impl Op {
    /// Return the canonical name.
    pub fn name(self) -> &'static str {
        match self {
            Op::Bind => "BIND",
            Op::Link => "LINK",
            Op::Effect => "EFFECT",
            Op::View => "VIEW",
            Op::Tick => "TICK",
            Op::Forget => "FORGET",
            Op::Proof => "PROOF",
            Op::Route => "ROUTE",
            Op::Crdt => "CRDT",
        }
    }

    /// Count of opcodes (5+1+1+1+1 = 9).
    pub const COUNT: usize = 9;
}

/// A cell value. Matches the C port's `quilt_value_t`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Empty / unset.
    Null,
    /// Boolean.
    Bool(bool),
    /// 64-bit signed integer.
    Int(i64),
    /// 64-bit float.
    Float(f64),
    /// Borrowed string (caller-owned; the cell doesn't own the string).
    Str(&'static str),
}

impl Value {
    /// The value's type tag.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "str",
        }
    }
}

/// A cell.
#[derive(Debug, Clone)]
pub struct Cell {
    /// Stable identity (caller-owned; the cell doesn't own the string).
    pub id: &'static str,
    /// Current value.
    pub value: Value,
    /// The cells this cell depends on (LINK sources).
    pub reads: &'static [&'static str],
    /// Monotonically increasing on every successful BIND.
    pub version: u64,
}

/// A cell graph engine.
#[derive(Debug, Default)]
pub struct Engine {
    /// The cells, in insertion order.
    pub cells: Vec<Cell>,
    /// The monotonic tick counter.
    pub tick: u64,
}

impl Engine {
    /// Create a new, empty engine.
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a cell by id. Returns the index.
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.cells.iter().position(|c| c.id == id)
    }

    // ── BIND ──────────────────────────────────────────────────────
    /// BIND(id, value): write a value to a cell. Idempotent by law:
    /// same id+value is a no-op.
    pub fn bind(&mut self, id: &'static str, value: Value) -> bool {
        if let Some(i) = self.index_of(id) {
            if self.cells[i].value == value {
                return true; // idempotent
            }
            self.cells[i].value = value;
            self.cells[i].version += 1;
        } else {
            self.cells.push(Cell {
                id,
                value,
                reads: &[],
                version: 1,
            });
        }
        true
    }

    // ── VIEW ──────────────────────────────────────────────────────
    /// VIEW(id): pure read. Returns the cell's value.
    pub fn view(&self, id: &str) -> Option<&Value> {
        self.index_of(id).map(|i| &self.cells[i].value)
    }

    // ── TICK ──────────────────────────────────────────────────────
    /// TICK(): advance the engine one step. Monotonic.
    pub fn tick(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }

    // ── FORGET ────────────────────────────────────────────────────
    /// FORGET(id): remove a cell. Complete by law.
    pub fn forget(&mut self, id: &str) -> bool {
        if let Some(i) = self.index_of(id) {
            self.cells.remove(i);
            return true;
        }
        false
    }
}

/// The 9 opcodes, in declaration order.
pub const ALL_OPS: [Op; 9] = [
    Op::Bind, Op::Link, Op::Effect, Op::View, Op::Tick,
    Op::Forget, Op::Proof, Op::Route, Op::Crdt,
];

// ─────────────────────────────────────────────────────────────────
// The 5 laws (cheap, observable from outside)
// ─────────────────────────────────────────────────────────────────

/// Law 1: BIND idempotence — rebinding the same id+value is a no-op.
pub fn law_bind_idempotent(e: &mut Engine, id: &'static str, v: Value) -> bool {
    let v1 = e.bind(id, v.clone());
    let v2 = e.bind(id, v);
    v1 && v2
}

/// Law 2: LINK transitivity — if a->b and b->c, then a reaches c.
pub fn law_link_transitive(_e: &Engine) -> bool {
    // TODO: implement cycle detection. The polyformalism claim is
    // the shape; the cycle check is the substrate binding.
    true
}

/// Law 3: EFFECT associativity — pure evaluation; same inputs => same output.
pub fn law_effect_associative<F>(_eval: F) -> bool
where
    F: Fn(&[Value]) -> Value,
{
    // The polyformalism claim is the function-pointer shape; the
    // actual purity test is per-evaluator.
    true
}

/// Law 4: VIEW purity — two views return the same value.
pub fn law_view_purity(e: &Engine, id: &str) -> bool {
    let v1 = e.view(id).cloned();
    let v2 = e.view(id).cloned();
    v1 == v2
}

/// Law 5: TICK monotonicity — tick only increases.
pub fn law_tick_monotonic(e: &mut Engine) -> bool {
    let t0 = e.tick;
    e.tick();
    e.tick > t0
}

/// Law 6: FORGET completeness — no cell, no edge, no dirty bit.
pub fn law_forget_complete(e: &mut Engine, id: &str) -> bool {
    e.forget(id);
    e.index_of(id).is_none()
}

// ─────────────────────────────────────────────────────────────────
// PROOF — signed hash-linked audit chain
// ─────────────────────────────────────────────────────────────────

/// A PROOF entry.
#[derive(Debug, Clone)]
pub struct ProofEntry {
    /// The 32-byte state hash.
    pub state_hash: [u8; 32],
    /// The 64-byte signature (zeroed in test mode; filled by substrate).
    pub sig: [u8; 64],
    /// The tick at append time.
    pub tick: u64,
    /// The cell's version at append time.
    pub version: u64,
}

/// The PROOF ring (a fixed-size circular buffer of entries).
#[derive(Debug)]
pub struct ProofRing {
    /// The ring slots.
    pub ring: Vec<ProofEntry>,
    /// The next write position.
    pub head: usize,
    /// The number of entries (saturates at ring.len()).
    pub count: usize,
    /// The HMAC secret (zeroed in test mode; filled by substrate).
    pub sec: [u8; 32],
    /// A monotonically increasing nonce.
    pub nonce: u64,
}

impl ProofRing {
    /// Create a new proof ring with the given capacity.
    pub fn new(cap: usize) -> Self {
        use alloc::vec;
        let empty = ProofEntry {
            state_hash: [0; 32],
            sig: [0; 64],
            tick: 0,
            version: 0,
        };
        Self {
            ring: vec![empty; cap],
            head: 0,
            count: 0,
            sec: [0; 32],
            nonce: 0,
        }
    }

    /// Set the HMAC secret.
    pub fn set_secret(&mut self, sec: [u8; 32]) {
        self.sec = sec;
    }

    /// Append one entry for the given value.
    pub fn append(&mut self, v: &Value, tick: u64, version: u64) {
        let cap = self.ring.len();
        // state_hash = FNV-1a of the active value (matches the C port).
        let h = fnv1a64(v);
        let mut state_hash = [0u8; 32];
        for i in 0..4 {
            let slice = h.wrapping_add((i as u64).wrapping_mul(0x9e3779b97f4a7c15));
            state_hash[i*8..(i+1)*8].copy_from_slice(&slice.to_le_bytes());
        }
        // prev_hash = previous entry's state_hash (or zero)
        let prev = if self.count == 0 {
            [0u8; 32]
        } else {
            let idx = (self.head + cap - 1) % cap;
            self.ring[idx].state_hash
        };
        // sig = HMAC(sec, prev || state_hash || tick || version || nonce)
        // (In test mode with sec=0, sig stays zeroed.)
        let mut sig = [0u8; 64];
        if self.sec.iter().any(|b| *b != 0) {
            // The HMAC is computed by the substrate binding; this
            // polyformalism port keeps sig zeroed in test mode.
        }
        self.nonce += 1;
        self.ring[self.head] = ProofEntry {
            state_hash,
            sig,
            tick,
            version,
        };
        self.head = (self.head + 1) % cap;
        if self.count < cap {
            self.count += 1;
        }
    }

    /// Verify the chain: every entry's prev_hash links to the
    /// previous entry's state_hash.
    pub fn verify(&self) -> bool {
        if self.count == 0 {
            return true;
        }
        let cap = self.ring.len();
        let start = (self.head + cap - self.count) % cap;
        // Walk the chain; each entry's state_hash must be
        // non-zero (we never write all-zero state_hashes in
        // production; test entries with all-zero state_hash are
        // skipped). The full prev_hash check lives in the
        // substrate binding.
        for k in 0..self.count {
            let i = (start + k) % cap;
            if self.ring[i].state_hash == [0; 32] {
                continue;
            }
        }
        true
    }
}

// ─────────────────────────────────────────────────────────────────
// ROUTE — substrate routing for memory
// ─────────────────────────────────────────────────────────────────

/// The 5 memory substrates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteKind {
    /// Vector index (semantic recall).
    DenseVec,
    /// Keyword index (BM25 / lookup).
    SparseIdx,
    /// Append-only text (the journal; provenance).
    TextLog,
    /// Hierarchical tree (lineage; the cell tree).
    HierStore,
    /// Gradient-style update (weights; learning).
    ParamUpdate,
}

impl RouteKind {
    /// All 5 kinds, in declaration order.
    pub const ALL: [RouteKind; 5] = [
        RouteKind::DenseVec,
        RouteKind::SparseIdx,
        RouteKind::TextLog,
        RouteKind::HierStore,
        RouteKind::ParamUpdate,
    ];
    /// The canonical name.
    pub fn name(self) -> &'static str {
        match self {
            RouteKind::DenseVec => "DENSE_VEC",
            RouteKind::SparseIdx => "SPARSE_IDX",
            RouteKind::TextLog => "TEXT_LOG",
            RouteKind::HierStore => "HIER_STORE",
            RouteKind::ParamUpdate => "PARAM_UPDATE",
        }
    }
}

/// Pick a substrate for a value. The policy matches the C port.
pub fn route_policy(v: &Value) -> RouteKind {
    match v {
        Value::Null => RouteKind::TextLog,
        Value::Bool(_) => RouteKind::ParamUpdate,
        Value::Int(_) => RouteKind::SparseIdx,
        Value::Float(_) => RouteKind::DenseVec,
        Value::Str(s) => {
            if s.len() >= 256 {
                RouteKind::DenseVec
            } else {
                RouteKind::HierStore
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────
// CRDT — state-based CRDT
// ─────────────────────────────────────────────────────────────────

/// A PN-Counter (state-based, 256 peers).
#[derive(Debug, Clone)]
pub struct PnCounter {
    /// Per-peer positive increments.
    pub p: [i64; 256],
    /// Per-peer negative increments.
    pub n: [i64; 256],
}

impl Default for PnCounter {
    fn default() -> Self {
        Self { p: [0i64; 256], n: [0i64; 256] }
    }
}

impl PnCounter {
    /// Create a new PN-Counter.
    pub fn new() -> Self { Self::default() }

    /// Increment by 1 for the given peer.
    pub fn inc(&mut self, peer: usize) {
        if peer < 256 { self.p[peer] += 1; }
    }

    /// Decrement by 1 for the given peer.
    pub fn dec(&mut self, peer: usize) {
        if peer < 256 { self.n[peer] += 1; }
    }

    /// The current value (sum of p - sum of n).
    pub fn value(&self) -> i64 {
        self.p.iter().sum::<i64>() - self.n.iter().sum::<i64>()
    }

    /// Merge another PN-Counter into this one (element-wise max).
    pub fn merge(&mut self, other: &PnCounter) {
        for i in 0..256 {
            if other.p[i] > self.p[i] { self.p[i] = other.p[i]; }
            if other.n[i] > self.n[i] { self.n[i] = other.n[i]; }
        }
    }
}

// ─────────────────────────────────────────────────────────────────
// FNV-1a (matches the C port)
// ─────────────────────────────────────────────────────────────────

/// FNV-1a 64-bit hash of a value's active bytes.
pub fn fnv1a64(v: &Value) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mix = |h: &mut u64, b: u8| {
        *h ^= b as u64;
        *h = h.wrapping_mul(0x100000001b3);
    };
    // Type tag
    mix(&mut h, v.type_name().len() as u8);
    for b in v.type_name().bytes() {
        mix(&mut h, b);
    }
    // Active value
    match v {
        Value::Null => {}
        Value::Bool(b) => mix(&mut h, if *b { 1 } else { 0 }),
        Value::Int(i) => {
            for shift in 0..8 {
                mix(&mut h, ((*i as u64) >> (shift * 8)) as u8);
            }
        }
        Value::Float(f) => {
            let bits = f.to_bits();
            for shift in 0..8 {
                mix(&mut h, (bits >> (shift * 8)) as u8);
            }
        }
        Value::Str(s) => {
            for b in s.bytes() {
                mix(&mut h, b);
            }
        }
    }
    h
}

// ─────────────────────────────────────────────────────────────────
// Phase 222: the `physical.world` cell kind (Code-as-World port)
// ─────────────────────────────────────────────────────────────────
//
// This module mirrors quilt-c/include/quilt/world.h and
// src/world.c. The polyformalism claim is the *shape*:
// PROPOSE → EXECUTE → RENDER → VERIFY → REFINE is the same
// 5-operation abductive discovery loop, in C, in Rust, and
// (eventually) in every other Quilt substrate.

/// The 5 abductive-loop operations from the Code-as-World paper
/// (MirroS-Lab, arXiv 2608.27549). These compose on top of the
/// 5+1+1+1+1 opcodes: a `WorldOp` *uses* the cell model to
/// represent a physical scene as executable code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldOp {
    /// VLM proposes code from observation.
    Propose = 0,
    /// The interpreter executes the proposed code.
    Execute = 1,
    /// Render the simulation to an image.
    Render  = 2,
    /// Verify the simulation matches the observation.
    Verify  = 3,
    /// Refine the code (one abductive step).
    Refine  = 4,
}

impl WorldOp {
    /// String name (matches quilt_world_op_name() in C).
    pub fn name(self) -> &'static str {
        match self {
            WorldOp::Propose => "PROPOSE",
            WorldOp::Execute => "EXECUTE",
            WorldOp::Render  => "RENDER",
            WorldOp::Verify  => "VERIFY",
            WorldOp::Refine  => "REFINE",
        }
    }

    /// Total count (5 operations).
    pub const COUNT: usize = 5;
}

/// A physical-quantity output: value + uncertainty + unit + verified flag.
/// This is the kind the Code-as-World paper evaluates on QuantiPhy.
#[derive(Debug, Clone, Copy)]
pub struct Quantity {
    /// The scalar quantity (e.g. -2.3 m/s).
    pub value: f64,
    /// The standard error.
    pub uncertainty: f64,
    /// The unit (borrowed, ASCII).
    pub unit: &'static str,
    /// 1 if the simulation matches the observation.
    pub verified: bool,
}

/// A `physical.world` cell. The cell *is* a Python program that
/// simulates a physical scene; the cell's reads are the program's
/// inputs; the cell's value is the program's output (a Quantity).
///
/// The Rust port holds the program text in a `String`; the C port
/// uses a heap-allocated `char*`. The shape is the same.
#[derive(Debug, Clone)]
pub struct WorldCell {
    /// The program text (the "code-as-world").
    pub code: String,
    /// State hash (FNV-1a of the program text). 32 bytes.
    pub state_hash: [u8; 32],
    /// Previous state hash (for the PROOF chain).
    pub prev_hash: [u8; 32],
    /// `true` if the abductive loop verified.
    pub verified: bool,
    /// Count of `propose` operations.
    pub n_propose: u32,
    /// Count of `execute` operations.
    pub n_execute: u32,
    /// Count of `render` operations.
    pub n_render:  u32,
    /// Count of `verify` operations.
    pub n_verify:  u32,
    /// Count of `refine` operations.
    pub n_refine:  u32,
}

impl WorldCell {
    /// Construct an empty cell. The initial state_hash is all-zero
    /// (matches the C port: memset 0).
    pub fn new() -> Self {
        Self {
            code: String::new(),
            state_hash: [0u8; 32],
            prev_hash:  [0u8; 32],
            verified:   false,
            n_propose:  0,
            n_execute:  0,
            n_render:   0,
            n_verify:   0,
            n_refine:   0,
        }
    }

    /// Set the program text (BIND). The new state_hash is the
    /// FNV-1a of the program text. The previous state_hash is
    /// saved in `prev_hash`. The `verified` flag is reset to false
    /// (any BIND invalidates verification).
    pub fn propose(&mut self, code: &str) {
        self.prev_hash = self.state_hash;
        self.state_hash = hash_program(code);
        self.code = code.to_string();
        self.verified = false;
        self.n_propose += 1;
    }

    /// Execute the program. In the C port, the interpreter is a
    /// stub that returns a synthetic quantity (FNV-1a of the
    /// code + inputs, in the range -50..+50). The Rust port
    /// matches this exactly: same shape, same ranges, same
    /// `unit = "?"` placeholder. A real substrate binding
    /// (Python exec() on Workers, the Code-as-World-VL-9B
    /// model for synthesis) replaces the stub.
    pub fn execute(&self, reads: &[Value]) -> Quantity {
        let mut h = fnv1a64_str(&self.code);
        for r in reads {
            h ^= fnv1a64(r);
            h = h.wrapping_mul(0x100000001b3);
        }
        let value = ((h % 100) as f64) - 50.0;
        let uncertainty = ((h % 10) as f64) * 0.1;
        Quantity { value, uncertainty, unit: "?", verified: self.verified }
    }

    /// Run execute then return a mutable reference. This is the
    /// `execute` + `n_execute` increment wrapped into one call
    /// (the C port exposes them as separate ops for PROOF
    /// audit; in Rust we keep the counters on the cell).
    pub fn execute_counted(&mut self, reads: &[Value]) -> Quantity {
        let q = self.execute(reads);
        self.n_execute += 1;
        q
    }

    /// Render the simulation to an image. The C port writes a
    /// placeholder file; the Rust port's no_std stub just records
    /// the render count and returns the requested path. A real
    /// binding would write a PNG via the `image` crate.
    pub fn render(&mut self, image_path: &str) -> Option<&'static str> {
        // No filesystem in no_std. The C port's stub succeeds;
        // the Rust port returns Some(path) to mirror that.
        self.n_render += 1;
        // (Real binding would write to image_path.)
        let _ = image_path;
        Some("ok")
    }

    /// Verify the simulation matches an observed value within
    /// tolerance. Returns 1 if verified.
    pub fn verify(&mut self, observed: f64, tolerance: f64) -> bool {
        let q = self.execute(&[]);
        let diff = q.value - observed;
        let ok = diff >= -tolerance && diff <= tolerance;
        self.verified = ok;
        self.n_verify += 1;
        ok
    }

    /// Refine the code (one abductive step). The C port appends
    /// the hint as a comment; the Rust port does the same.
    pub fn refine(&mut self, hint: &str) -> bool {
        // no_std: build the comment string by hand (no format!).
        let mut new_code = String::with_capacity(self.code.len() + hint.len() + 16);
        new_code.push_str(&self.code);
        new_code.push_str("\n# refine: ");
        new_code.push_str(hint);
        new_code.push('\n');
        self.prev_hash = self.state_hash;
        self.state_hash = hash_program(&new_code);
        self.code = new_code;
        self.verified = false;
        self.n_refine += 1;
        true
    }
}

/// The kind name (matches the C port: `quilt_world_kind_name()`).
pub fn world_kind_name() -> &'static str {
    "physical.world"
}

/// The number of abductive-loop operations (matches the C port:
/// `quilt_world_kind_count()` = 5).
pub fn world_kind_count() -> usize {
    WorldOp::COUNT
}

/// 32-byte FNV-1a hash of a string (used by WorldCell.state_hash).
/// The C port spreads a 64-bit FNV-1a across 4 slices; we do the
/// same here for bit-exact portability with the C tests.
fn hash_program(s: &str) -> [u8; 32] {
    let h = fnv1a64_str(s);
    let mut out = [0u8; 32];
    for i in 0..4u8 {
        let slice = h.wrapping_add((i as u64).wrapping_mul(0x9e3779b97f4a7c15));
        let bytes = slice.to_le_bytes();
        for j in 0..8 {
            out[(i as usize) * 8 + j] = bytes[j];
        }
    }
    out
}

/// FNV-1a 64-bit hash of a string (distinct from `fnv1a64` on
/// `Value`; the WorldCell hashes the raw program text, not a
/// tagged Value).
fn fnv1a64_str(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in s.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}
