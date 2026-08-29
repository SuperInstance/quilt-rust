//! # cell_ledger.rs
//!
//! Append-only cell history over the sealed ledger, plus the
//! `record_with` chronicle helper other cell bridges call.
//!
//! ## Role in the system
//!
//! `quilt_core::CellLedger` is the storage pattern this crate builds on:
//! per-cell, append-only, hash-chained, double-entry, with
//! `record_with(input, output, ts, provenance, expected)` as the sealing
//! call for remote producer feeds (`docs/bridge-cell-ledger.md`,
//! adoption point 1). This module gives that ledger the surface the
//! field-edge side and the quilt grid viewer speak:
//!
//! - [`CellLedger`] — a thin wrapper: `append(event)` seals one
//!   transition, `iter()` / `iter_range(range)` walk the history in
//!   chain order over the seq space (contiguous from 1).
//! - [`CellEvent`] — one cell transition as the producing side sees it:
//!   who (`agent_id`), why (`op_name`), when (`ts`), and the state the
//!   transition installs (`value`). The `before` side is not a parameter:
//!   an append-only ledger derives it from its own state — that is what
//!   makes the history un-gameable.
//! - [`record_with`] — the chronicle/provenance helper: run an operation
//!   and have the recording happen *around* it, sealing `(agent_id, ts,
//!   before/after field state)` into one push-origin entry.
//!
//! ## The viewer mapping (quilt grid viewer, cell history)
//!
//! Each sealed entry is one history row: `actor` = `provenance.caller`,
//! `cause` = the `op` recorded in the input posting, `old_value` =
//! `delta.before`, `new_value` = `delta.after`, `ts` = `entry.ts` — plus
//! the seal (`entry.hash`) a bare transaction log cannot offer.
//!
//! ## Key decisions
//!
//! - **No second chain.** Everything funnels into
//!   `quilt_core::CellLedger::record_with`; `verify_chain` /
//!   `reconcile` on the wrapper are the core's own audits.
//! - **Push origin.** Bridge entries are remote feeds by definition
//!   (`origin: Push`, `caller: agent_id`), matching the elephant
//!   producer / crab-traps relay contract in `docs/bridge-cell-ledger.md`.
//! - **Persistence prior.** Appends score against `expected = before`
//!   (surprise == edge magnitude) — the elephant's field reads carry
//!   `expected: null`; the JEPA forecast-sealing loop stays on the core
//!   API (`inner_mut()`), which already accepts explicit predictions.

use std::ops::Range;

use quilt_core::{
    CellId, CellLedger as SealedLedger, ChainAudit, LedgerEntry, LedgerOrigin, Provenance,
    Reconciliation, Replay,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// CellEvent — one transition, as the producing side sees it
// ---------------------------------------------------------------------------

/// One cell transition to seal into the ledger: who, why, when, and the
/// state it installs. `before` is derived from the ledger's current
/// state at append time — never a parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellEvent {
    /// The agent (bridge, daemon, relay, client) that performed the
    /// operation — recorded as `provenance.caller`.
    pub agent_id: CellId,
    /// What the agent did (`"field.read"`, `"dial.set"`, ...) —
    /// recorded in the sealed input posting.
    pub op_name: String,
    /// When the transition happened (millis since epoch). The ledger
    /// keeps no clocks; callers pass timestamps.
    pub ts: u64,
    /// The field state this transition installs (the `after` side of
    /// the edge).
    pub value: Value,
}

// ---------------------------------------------------------------------------
// CellLedger — append-only history over the sealed core ledger
// ---------------------------------------------------------------------------

/// An append-only ledger of cell transitions, backed by
/// `quilt_core::CellLedger`'s hash-chained entries.
///
/// Construct one per cell (`new` / `with_genesis`), `append` events,
/// walk the history with `iter` / `iter_range`. Sealing, chaining and
/// auditing belong to the core ledger — the wrapper adds the
/// event-shaped append and the seq-range read the viewer wants.
///
/// # Example
///
/// ```
/// use field_edge_bridge::{CellEvent, CellLedger};
/// use serde_json::json;
///
/// let mut history = CellLedger::with_genesis("bilge.level", json!(40.0), 0);
/// history.append(CellEvent {
///     agent_id: "sensor.gauge".into(),
///     op_name: "field.read".into(),
///     ts: 1_000,
///     value: json!(85.0),
/// });
/// history.append(CellEvent {
///     agent_id: "sensor.gauge".into(),
///     op_name: "field.read".into(),
///     ts: 2_000,
///     value: json!(90.0),
/// });
///
/// // History in chain order; ranges address the seq space (1-based).
/// let rows: Vec<u64> = history.iter().map(|e| e.seq).collect();
/// assert_eq!(rows, vec![1, 2]);
/// assert_eq!(history.iter_range(2..3).next().unwrap().delta.after, json!(90.0));
/// assert!(history.verify_chain().intact);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CellLedger {
    inner: SealedLedger,
}

impl CellLedger {
    /// A fresh history for `cell_id`, state `null`, no genesis.
    pub fn new(cell_id: impl Into<CellId>) -> Self {
        Self {
            inner: SealedLedger::new(cell_id),
        }
    }

    /// A history whose cell began life at a known state; the genesis is
    /// committed by the chain root and scores the first transition
    /// against the persistence prior.
    pub fn with_genesis(cell_id: impl Into<CellId>, genesis: Value, ts: u64) -> Self {
        Self {
            inner: SealedLedger::with_genesis(cell_id, genesis, ts),
        }
    }

    // -- Appending ---------------------------------------------------------

    /// Seal one cell transition into the ledger and advance the state.
    ///
    /// The event's `value` becomes the `after` side of the edge; the
    /// `before` side is the ledger's current state. Returns the sealed
    /// entry (seq, both postings, the edge, the surprise, the hashes).
    pub fn append(&mut self, event: CellEvent) -> LedgerEntry {
        let CellEvent {
            agent_id,
            op_name,
            ts,
            value,
        } = event;
        self.inner.record_with(
            json!({ "op": op_name }),
            value,
            ts,
            Provenance {
                origin: LedgerOrigin::Push,
                caller: Some(agent_id),
                trace: Vec::new(),
            },
            None,
        )
    }

    // -- History reads ------------------------------------------------------

    /// All sealed transitions, in chain order (oldest first).
    pub fn iter(&self) -> impl Iterator<Item = &LedgerEntry> + '_ {
        self.inner.entries().iter()
    }

    /// Sealed transitions whose seq falls in `range` (1-based, end
    /// exclusive), in chain order. Seqs are contiguous from 1, so a
    /// range is a positional window: `iter_range(n - k..n + 1)` is the
    /// viewer's "last k entries" query. Out-of-bounds ends clamp.
    pub fn iter_range(&self, range: Range<u64>) -> impl Iterator<Item = &LedgerEntry> + '_ {
        let len = self.inner.len();
        let start = (range.start.saturating_sub(1) as usize).min(len);
        let count = (range.end.saturating_sub(range.start) as usize).min(len - start);
        self.inner.entries()[start..start + count].iter()
    }

    // -- Delegated to the sealed core ledger --------------------------------

    /// The cell this history belongs to.
    pub fn cell_id(&self) -> &str {
        self.inner.cell_id()
    }

    /// The current state (the `after` of the last transition, else the
    /// genesis, else `null`).
    pub fn state(&self) -> &Value {
        self.inner.state()
    }

    /// Number of sealed transitions.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// True when no sealed transitions exist.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// All sealed transitions as a slice, in chain order.
    pub fn entries(&self) -> &[LedgerEntry] {
        self.inner.entries()
    }

    /// The head (most recent) transition, if any.
    pub fn head(&self) -> Option<&LedgerEntry> {
        self.inner.head()
    }

    /// The tamper-evident commitment to everything this cell ever did.
    pub fn chain_hash(&self) -> quilt_core::Hash {
        self.inner.chain_hash()
    }

    /// Recompute every seal and prev-link.
    pub fn verify_chain(&self) -> ChainAudit {
        self.inner.verify_chain()
    }

    /// Reconcile the books (matching, open inputs, chain, continuity,
    /// surprise totals).
    pub fn reconcile(&self) -> Reconciliation {
        self.inner.reconcile()
    }

    /// Replay the history up to and including `until_ts`.
    pub fn replay(&self, until_ts: u64) -> Replay {
        self.inner.replay(until_ts)
    }

    /// Borrow the backing sealed ledger (e.g. for `restore_entry` when
    /// draining a pre-sealed remote chain).
    pub fn inner(&self) -> &SealedLedger {
        &self.inner
    }

    /// Mutably borrow the backing sealed ledger (e.g. for explicit
    /// forecast sealing via `record_with(..., expected)`).
    pub fn inner_mut(&mut self) -> &mut SealedLedger {
        &mut self.inner
    }

    /// Unwrap into the backing sealed ledger.
    pub fn into_inner(self) -> SealedLedger {
        self.inner
    }
}

impl From<SealedLedger> for CellLedger {
    fn from(inner: SealedLedger) -> Self {
        Self { inner }
    }
}

impl From<CellLedger> for SealedLedger {
    fn from(bridge: CellLedger) -> Self {
        bridge.into_inner()
    }
}

// ---------------------------------------------------------------------------
// record_with — the chronicle helper
// ---------------------------------------------------------------------------

/// Run `op` with the recording wrapped around it: the before-state is
/// captured from the ledger, the operation runs once, and its outcome
/// is sealed as one push-origin entry carrying `(agent_id, ts,
/// before/after field state)`.
///
/// This is the helper other cell bridges call (the "rebuild with
/// `record_with`" recipe of `docs/cohesion-and-fascia.md` §6): the
/// agent and the op name land in the sealed provenance and input
/// posting, the edge and its surprise are scored by the core ledger
/// under the persistence prior, and the returned entry is
/// tamper-evident the moment `op` returns. Returns the operation's own
/// result alongside the sealed entry.
///
/// # Example
///
/// ```
/// use field_edge_bridge::{record_with, CellLedger};
/// use serde_json::{json, Value};
///
/// let mut ledger =
///     CellLedger::with_genesis("room.field.warmth", json!([0.25, -0.125, 0.5]), 1_000);
///
/// let (reading, entry) = record_with(
///     &mut ledger,
///     "elephant.daemon",
///     "field.read",
///     1_060,
///     |before: &Value| {
///         // the operation sees the before-state; returns (result, after-state)
///         let after = json!([0.375, -0.0625, 0.625]);
///         (after.clone(), after)
///     },
/// );
///
/// assert_eq!(entry.delta.before, json!([0.25, -0.125, 0.5]));
/// assert_eq!(entry.delta.after, reading);
/// assert_eq!(entry.provenance.caller.as_deref(), Some("elephant.daemon"));
/// assert_eq!(ledger.state(), &reading);
/// ```
pub fn record_with<F, R>(
    ledger: &mut CellLedger,
    agent_id: impl Into<CellId>,
    op_name: &str,
    ts: u64,
    op: F,
) -> (R, LedgerEntry)
where
    F: FnOnce(&Value) -> (R, Value),
{
    let before = ledger.state().clone();
    let (result, after) = op(&before);
    let entry = ledger.append(CellEvent {
        agent_id: agent_id.into(),
        op_name: op_name.to_owned(),
        ts,
        value: after,
    });
    (result, entry)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- append -------------------------------------------------------------

    #[test]
    fn append_seals_a_push_origin_edge_and_advances_state() {
        let mut history = CellLedger::with_genesis("bilge.level", json!(40.0), 0);
        let entry = history.append(CellEvent {
            agent_id: "sensor.gauge".into(),
            op_name: "field.read".into(),
            ts: 1_000,
            value: json!(85.0),
        });

        assert_eq!(entry.seq, 1);
        assert_eq!(entry.ts, 1_000);
        // The cause is sealed into the input posting; the actor into provenance.
        assert_eq!(entry.input.value, json!({ "op": "field.read" }));
        assert_eq!(entry.provenance.origin, LedgerOrigin::Push);
        assert_eq!(entry.provenance.caller.as_deref(), Some("sensor.gauge"));
        // The edge: before derived from state, after from the event.
        assert_eq!(entry.delta.before, json!(40.0));
        assert_eq!(entry.delta.after, json!(85.0));
        // Persistence prior: expected == before, surprise == edge.
        assert_eq!(entry.expected, Some(json!(40.0)));
        assert!((entry.imbalance.unwrap() - 45.0).abs() < 1e-12);
        assert_eq!(history.state(), &json!(85.0));

        assert!(history.verify_chain().intact);
        assert!(history.reconcile().balanced);
    }

    #[test]
    fn record_with_wraps_the_operation_with_before_after_state() {
        let mut ledger =
            CellLedger::with_genesis("room.field.warmth", json!([0.25, -0.125, 0.5]), 1_000);

        let (ticks, entry) = record_with(
            &mut ledger,
            "crab-traps.relay",
            "field.read",
            1_060,
            |before: &Value| {
                let n = before.as_array().unwrap().len();
                let after = json!([0.375, -0.0625, 0.625]);
                (n, after)
            },
        );

        assert_eq!(ticks, 3);
        assert_eq!(entry.delta.before, json!([0.25, -0.125, 0.5]));
        assert_eq!(entry.delta.after, json!([0.375, -0.0625, 0.625]));
        assert_eq!(entry.provenance.caller.as_deref(), Some("crab-traps.relay"));
        assert_eq!(entry.input.value, json!({ "op": "field.read" }));
        assert_eq!(ledger.state(), &json!([0.375, -0.0625, 0.625]));
        // A second wrapped op sees the advanced state as its before.
        let (_, e2) = record_with(
            &mut ledger,
            "crab-traps.relay",
            "dial.set",
            2_000,
            |before| ((), before.clone()),
        );
        assert_eq!(e2.delta.before, json!([0.375, -0.0625, 0.625]));
        assert!(!e2.delta.changed);
        assert!(ledger.verify_chain().intact);
    }

    #[test]
    fn ledger_imbalance_and_wire_imbalance_are_two_projections_of_one_edge() {
        // The crate's thesis, on one sealed entry: the core's sealed
        // imbalance scores value_distance (mean-metric on arrays), the
        // wire op_d quantity is the L2 norm ||Δ||₂ — same edge, two lenses.
        let mut ledger = CellLedger::with_genesis("room.field", json!([0.25, -0.125, 0.5]), 1_000);
        let entry = ledger.append(CellEvent {
            agent_id: "elephant.daemon".into(),
            op_name: "field.read".into(),
            ts: 3_000,
            value: json!([0.375, -0.0625, 0.625]),
        });

        let before = [0.25, -0.125, 0.5];
        let after = [0.375, -0.0625, 0.625];
        // Ledger lens: mean of element-wise |Δ| = 0.3125 / 3.
        assert!((entry.imbalance.unwrap() - 0.3125 / 3.0).abs() < 1e-12);
        // Wire lens: ||Δ||₂ = 0.1875 (golden, bit-for-bit).
        assert!((crate::field_edge::imbalance(&before, &after).unwrap() - 0.1875).abs() < 1e-12);
    }

    // -- history reads ------------------------------------------------------

    #[test]
    fn iter_yields_chain_order_and_iter_range_windows_the_seq_space() {
        let mut history = CellLedger::with_genesis("cell.h", json!(0.0), 0);
        for i in 1..=5 {
            history.append(CellEvent {
                agent_id: "agent".into(),
                op_name: "tick".into(),
                ts: i * 100,
                value: json!(i as f64),
            });
        }

        let all: Vec<u64> = history.iter().map(|e| e.seq).collect();
        assert_eq!(all, vec![1, 2, 3, 4, 5]);

        // Half-open seq window.
        let mid: Vec<u64> = history.iter_range(2..4).map(|e| e.seq).collect();
        assert_eq!(mid, vec![2, 3]);
        // The viewer's "last k" query: seqs n-k+1 ..= n.
        let last2: Vec<u64> = history.iter_range(4..6).map(|e| e.seq).collect();
        assert_eq!(last2, vec![4, 5]);
        // Clamping: past the end, or empty.
        assert_eq!(history.iter_range(4..99).count(), 2);
        assert_eq!(history.iter_range(2..2).count(), 0);
        assert_eq!(history.iter_range(0..1).count(), 1); // seq 0 does not exist
                                                         // Ranges still carry the sealed rows, not projections.
        let row = history.iter_range(3..4).next().unwrap();
        assert_eq!(row.delta.after, json!(3.0));
        assert_eq!(row.hash.len(), 64);
    }

    #[test]
    fn history_rows_carry_the_viewer_transaction_fields() {
        // The quilt grid viewer's history row (old/new value, cause,
        // actor, ts) is a projection of one sealed entry.
        let mut history = CellLedger::new("fresh.cell");
        let e = history.append(CellEvent {
            agent_id: "bridge.a".into(),
            op_name: "field.read".into(),
            ts: 42,
            value: json!({"warmth": 0.5}),
        });

        let row = history.iter().next().unwrap();
        assert_eq!(row.delta.before, Value::Null);
        assert_eq!(row.delta.after, json!({"warmth": 0.5}));
        assert_eq!(row.ts, 42);
        assert_eq!(row.provenance.caller, e.provenance.caller);
        assert_eq!(row.input.value, json!({ "op": "field.read" })); // cause
                                                                    // Genesis-less first entry: no prior, no surprise claimed.
        assert_eq!(row.expected, None);
        assert_eq!(row.imbalance, None);
    }

    // -- interop with the sealed core ledger ---------------------------------

    #[test]
    fn round_trips_through_json_and_converts_to_the_core_ledger() {
        let mut history = CellLedger::with_genesis("cell.s", json!(7.0), 0);
        history.append(CellEvent {
            agent_id: "agent".into(),
            op_name: "set".into(),
            ts: 1_000,
            value: json!(8.0),
        });

        // serde round-trip (the wrapper is transparent over the core).
        let value = serde_json::to_value(&history).unwrap();
        let restored: CellLedger = serde_json::from_value(value).unwrap();
        assert_eq!(restored.cell_id(), "cell.s");
        assert_eq!(restored.chain_hash(), history.chain_hash());
        assert!(restored.verify_chain().intact);

        // Conversions share the chain bit-for-bit.
        let core: SealedLedger = restored.into();
        assert_eq!(core.len(), 1);
        let rebuilt: CellLedger = core.into();
        assert_eq!(rebuilt.chain_hash(), history.chain_hash());

        // The escape hatch reaches the full core API (e.g. explicit forecasts).
        let mut history = rebuilt;
        history.inner_mut().record_with(
            json!({"q": "next?"}),
            json!(9.0),
            2_000,
            Provenance {
                origin: LedgerOrigin::Call,
                caller: Some("predictor".into()),
                trace: vec![],
            },
            Some(json!(8.5)), // sealed before the outcome exists
        );
        let head = history.head().unwrap();
        assert!((head.imbalance.unwrap() - 0.5).abs() < 1e-12);
        assert!(history.reconcile().balanced);
    }
}
