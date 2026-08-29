//! # field-edge-bridge
//!
//! The bridge between field-edge systems (the elephant's `vmf.edge`) and
//! this repo's cell-ledger vocabulary — the Rust side of
//! `docs/field-edge-ledger-bridge.md` and `docs/bridge-cell-ledger.md`,
//! with the standalone numpy proof in `bridge_demo.py`.
//!
//! ## Role in the system
//!
//! The thesis (proven to 1e-12 against `compat/golden.json`): the
//! ledger's `imbalance` and the elephant's field-edge are two
//! projections of **one vector**, the directed edge `Δ = after − before`.
//! This crate is the seam where the two vocabularies meet in Rust:
//!
//! - [`field_edge::FieldEdge`] reads the *field view* of a sealed edge
//!   (`d_mu`, signed `d_warmth`, `radial`) and exposes the four bridge
//!   identities as checkable residuals.
//! - [`cell_ledger::CellLedger`] is the *ledger view*: an append-only
//!   cell-history surface (`append`, `iter`, `iter_range`) backed by
//!   `quilt_core::CellLedger`'s hash-chained entries — the shape the
//!   quilt grid viewer's history endpoints expect (actor, cause,
//!   old/new value, ts, sealed).
//! - [`cell_ledger::record_with`] is the chronicle helper other cell
//!   bridges call: it wraps an operation with `(agent_id, ts,
//!   before/after field state)` recording, sealed as a `push`-origin
//!   entry per `docs/bridge-cell-ledger.md`.
//!
//! ## Depends on
//!
//! - `quilt-core` — the sealed ledger itself: `CellLedger`, `record_with`,
//!   `Provenance`, `LedgerOrigin`, `verify_chain`, `reconcile`. This crate
//!   computes nothing the core already owns; it adds the field lens and
//!   the history surface on top.
//! - `serde` / `serde_json` — events and ledgers serialize; the chain
//!   seal runs over the core's canonical JSON form.
//!
//! ## Used by
//!
//! - Cell bridges draining remote field feeds (the elephant's
//!   `CellLedgerProducer`, the crab-traps relay) into sealed local
//!   ledgers — the "rebuild with `record_with`, then `verify_chain`"
//!   recipe of `docs/cohesion-and-fascia.md` §6.
//! - Viewer/audit tooling that wants a cell's full change history
//!   (seq-ordered, tamper-evident) without touching the engine.
//!
//! ## Key decisions
//!
//! - **The core ledger is the storage; this crate is a lens.** No
//!   second chain, no second hash, no re-derivation. Every append goes
//!   through `quilt_core::CellLedger::record_with`, so bridge history is
//!   bit-for-bit identical to engine-side history of the same events.
//! - **Two projections, one edge — kept distinct on purpose.** The
//!   ledger's sealed `entry.imbalance` scores the core's total
//!   `value_distance` metric (mean-metric on arrays); the wire `op_d`
//!   quantity and the field view use the L2 norm `‖Δ‖₂`
//!   ([`field_edge::imbalance`]). Neither is wrong; they are the two
//!   lenses of `docs/field-edge-ledger-bridge.md`.
//! - **Honesty gates.** No prior → no surprise claim (the core's
//!   null-prior rule); no direction (zero-norm state) → no field view
//!   (`FieldEdge::compute` returns `None`). Never fake a number.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

pub mod cell_ledger;
pub mod field_edge;

pub use crate::cell_ledger::{record_with, CellEvent, CellLedger};
pub use crate::field_edge::{default_warm_axis, imbalance, FieldEdge};
