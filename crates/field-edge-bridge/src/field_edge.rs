//! # field_edge.rs
//!
//! The field view of a sealed ledger edge: `d_mu`, signed `d_warmth`,
//! `radial` — the elephant's `vmf.edge()` reads of the one directed
//! edge `Δ = after − before` whose norm the ledger seals.
//!
//! ## Role in the system
//!
//! The Rust side of `docs/field-edge-ledger-bridge.md` (the numpy proof
//! is `bridge_demo.py`): the ledger reads the **norm** of the edge
//! (unsigned surprise, sealed into every entry); the field reads its
//! **direction** (drift on the unit sphere) and its **length** (radial
//! growth). [`FieldEdge`] computes both views of one edge and exposes
//! the four bridge identities as checkable residuals, golden-tested
//! against `compat/golden.json`'s `vector-field-edge`.
//!
//! ## Key decisions
//!
//! - **Honesty gates.** A zero-norm state has no direction and a
//!   length-mismatched pair has no edge — [`FieldEdge::compute`]
//!   returns `None` rather than faking a number (the same discipline as
//!   the core ledger's null-prior rule and the elephant's `κ = None`
//!   deadband).
//! - **The wire quantity is L2.** [`imbalance`] computes `‖Δ‖₂`, the
//!   `quilt-compat/1 op_d` wire quantity — distinct from the sealed
//!   `entry.imbalance`, which scores the core's total `value_distance`
//!   metric (mean over array elements). Two lenses, one edge.
//! - **The warm axis is a parameter.** Valence is per-cell configured
//!   (`ŵ_cell`); [`default_warm_axis`] is the elephant's room-stand-in
//!   (mood+, volume+, cynicism−), kept so the golden identities are
//!   reproducible.

use quilt_core::LedgerEntry;
use serde_json::Value;

/// The elephant's warm direction for room cells (mood+, volume+,
/// cynicism−), the 3-d stand-in of `bridge_demo.py`:
/// `[0.30, 0.10, −0.15] / 0.35 = [6/7, 2/7, −3/7]`.
pub fn default_warm_axis() -> [f64; 3] {
    [6.0 / 7.0, 2.0 / 7.0, -3.0 / 7.0]
}

/// The wire (op_d) imbalance of an edge: `‖after − before‖₂`.
///
/// Returns `None` on length mismatch or non-finite inputs — the wire
/// spec emits `null` there, never a fabricated number.
pub fn imbalance(before: &[f64], after: &[f64]) -> Option<f64> {
    if before.len() != after.len() || !before.iter().chain(after.iter()).all(|x| x.is_finite()) {
        return None;
    }
    Some(
        before
            .iter()
            .zip(after.iter())
            .map(|(b, a)| (a - b) * (a - b))
            .sum::<f64>()
            .sqrt(),
    )
}

/// The field view of one edge: both projections of `Δ = after − before`.
///
/// | field | reads |
/// |---|---|
/// | `imbalance` | `‖Δ‖₂` — the wire quantity (what the ledger's norm sees) |
/// | `d_mu` | `‖μ̂_a − μ̂_b‖₂` — direction drift on the unit sphere |
/// | `d_warmth` | `ŵ·(μ̂_a − μ̂_b)` — **signed** valence along the warm axis |
/// | `warm_leg`, `perp` | the raw edge's split into the signed warm leg and its orthogonal remainder |
/// | `radial` | `ln(‖a‖/‖b‖)` — length drift (κ's side) |
/// | `norm_before`, `norm_after` | `‖b‖`, `‖a‖` |
///
/// With `‖before‖ = ‖after‖ = 1` (a direction cell), `imbalance ≡ d_mu`
/// bit-for-bit — the unit-cell collapse (identity 4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldEdge {
    /// `‖Δ‖₂` — the wire op_d quantity.
    pub imbalance: f64,
    /// `‖μ̂_a − μ̂_b‖₂ = √(2 − 2·cosθ)` — direction drift.
    pub d_mu: f64,
    /// `ŵ·(μ̂_a − μ̂_b)` — signed warmth change along the warm axis.
    pub d_warmth: f64,
    /// `ŵ·Δ` — the signed warm leg of the *raw* edge (identity 2's leg).
    pub warm_leg: f64,
    /// `‖Δ − (ŵ·Δ)ŵ‖₂` — the raw edge's orthogonal remainder.
    pub perp: f64,
    /// `ln(‖a‖/‖b‖)` — radial drift.
    pub radial: f64,
    /// `‖before‖`.
    pub norm_before: f64,
    /// `‖after‖`.
    pub norm_after: f64,
}

impl FieldEdge {
    /// Compute both views of the edge `before → after`, read along
    /// `warm` (any non-zero direction; normalized here). `None` when
    /// there is no well-defined field view: length mismatch (including
    /// against `warm`), an empty vector, non-finite values, or a
    /// zero-norm state (no direction exists).
    pub fn compute(before: &[f64], after: &[f64], warm: &[f64]) -> Option<Self> {
        if before.is_empty()
            || before.len() != after.len()
            || before.len() != warm.len()
            || !before
                .iter()
                .chain(after.iter())
                .chain(warm.iter())
                .all(|x| x.is_finite())
        {
            return None;
        }

        let norm = |v: &[f64]| v.iter().map(|x| x * x).sum::<f64>().sqrt();
        let (nb, na, nw) = (norm(before), norm(after), norm(warm));
        if nb == 0.0 || na == 0.0 || nw == 0.0 {
            return None; // no direction: never fake a number
        }

        let unit = |v: &[f64], n: f64| v.iter().map(|x| x / n).collect::<Vec<f64>>();
        let (mu_b, mu_a, w_hat) = (unit(before, nb), unit(after, na), unit(warm, nw));
        let dot = |a: &[f64], b: &[f64]| a.iter().zip(b.iter()).map(|(x, y)| x * y).sum::<f64>();
        let diff = |a: &[f64], b: &[f64]| {
            a.iter()
                .zip(b.iter())
                .map(|(x, y)| x - y)
                .collect::<Vec<f64>>()
        };

        let delta = diff(after, before);
        let warm_leg = dot(&w_hat, &delta);
        let perp_vec: Vec<f64> = delta
            .iter()
            .zip(w_hat.iter())
            .map(|(d, w)| d - warm_leg * w)
            .collect();

        Some(Self {
            imbalance: norm(&delta),
            d_mu: norm(&diff(&mu_a, &mu_b)),
            d_warmth: dot(&w_hat, &mu_a) - dot(&w_hat, &mu_b),
            warm_leg,
            perp: norm(&perp_vec),
            radial: (na / nb).ln(),
            norm_before: nb,
            norm_after: na,
        })
    }

    /// Read the field view off a sealed ledger entry's edge
    /// (`delta.before` / `delta.after`), along `warm`. `None` when the
    /// cell's states are not equal-length numeric vectors or have no
    /// direction — scalar cells have a ledger view but no field view.
    pub fn from_entry(entry: &LedgerEntry, warm: &[f64]) -> Option<Self> {
        Self::compute(
            &as_vector(&entry.delta.before)?,
            &as_vector(&entry.delta.after)?,
            warm,
        )
    }

    /// Identity 1 — `imbalance² = (‖a‖−‖b‖)² + ‖a‖·‖b‖·d_mu²`:
    /// surprise splits into magnitude drift + direction drift. Returns
    /// the residual (lhs − rhs); zero to floating rounding when the
    /// identity holds.
    pub fn radial_direction_split_residual(&self) -> f64 {
        self.imbalance * self.imbalance
            - (self.norm_after - self.norm_before).powi(2)
            - self.norm_after * self.norm_before * self.d_mu * self.d_mu
    }

    /// Identity 2 — `imbalance² = (ŵ·Δ)² + ‖Δ⊥‖²` (Pythagoras on the
    /// raw edge): warmth is one **signed leg** of the ledger's own
    /// surprise. Returns the residual; zero when the identity holds.
    pub fn warm_pythagoras_residual(&self) -> f64 {
        self.imbalance * self.imbalance - self.warm_leg * self.warm_leg - self.perp * self.perp
    }

    /// Identity 3 — `|d_warmth| ≤ d_mu ≤ imbalance`: the projection
    /// chain. Signed warmth is the weakest projection, direction drift
    /// the middle, the full norm the strongest — the three never
    /// disagree.
    pub fn projection_chain_holds(&self, eps: f64) -> bool {
        self.d_warmth.abs() <= self.d_mu + eps && self.d_mu <= self.imbalance + eps
    }
}

/// A JSON value as a numeric vector, if it is one (the field view is
/// vector-valued; anything else is not a field state).
fn as_vector(v: &Value) -> Option<Vec<f64>> {
    v.as_array()?
        .iter()
        .map(|x| x.as_f64())
        .collect::<Option<Vec<f64>>>()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn golden() -> Value {
        serde_json::from_str(include_str!("../../../compat/golden.json"))
            .expect("golden.json parses")
    }

    fn vector_field_edge() -> (Vec<f64>, Vec<f64>, f64) {
        let vec = golden()["op_d_edge"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == json!("vector-field-edge"))
            .unwrap()
            .clone();
        let nums = |key: &str| {
            vec[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect::<Vec<f64>>()
        };
        (
            nums("before"),
            nums("after"),
            vec["expect"]["imbalance"].as_f64().unwrap(),
        )
    }

    // -- The golden vector: both views, all four identities ------------------

    #[test]
    fn golden_vector_two_views_and_all_four_identities_hold() {
        let (before, after, want_imbalance) = vector_field_edge();
        let warm = default_warm_axis();
        let edge = FieldEdge::compute(&before, &after, &warm).unwrap();

        // Ledger lens: ||Δ||₂ is the golden wire imbalance, bit-for-bit.
        assert!((edge.imbalance - want_imbalance).abs() <= 1e-12);
        assert_eq!(edge.imbalance, imbalance(&before, &after).unwrap());

        // Field lens: the README's signs and magnitudes.
        // d_mu = 0.1542..., d_warmth = +0.1112 (the room warmed),
        // radial = +0.2446 (the field grew).
        assert!((edge.d_mu - 0.15417).abs() < 1e-4);
        assert!((edge.d_warmth - 0.11123).abs() < 1e-4);
        assert!(edge.d_warmth > 0.0);
        assert!((edge.radial - 0.24462).abs() < 1e-4);
        assert!(edge.radial > 0.0);

        // Identity 1: magnitude + direction split.
        assert!(edge.radial_direction_split_residual().abs() < 1e-12);
        // Identity 2: Pythagoras on the raw edge.
        assert!(edge.warm_pythagoras_residual().abs() < 1e-12);
        // Identity 3: the projection chain.
        assert!(edge.projection_chain_holds(1e-12));
        assert!(edge.d_mu < edge.imbalance); // not a unit cell: strict
    }

    #[test]
    fn unit_cell_collapse_imbalance_equals_d_mu_bit_for_bit() {
        // Identity 4: with ||before|| = ||after|| = 1 (a direction
        // cell), the ledger's imbalance and the elephant's d_mu are the
        // same number.
        let (before, after, _) = vector_field_edge();
        let norm = |v: &[f64]| v.iter().map(|x| x * x).sum::<f64>().sqrt();
        let (nb, na) = (norm(&before), norm(&after));
        let unit = |v: &[f64], n: f64| v.iter().map(|x| x / n).collect::<Vec<f64>>();
        let (ub, ua) = (unit(&before, nb), unit(&after, na));

        let edge = FieldEdge::compute(&ub, &ua, &default_warm_axis()).unwrap();
        assert!((edge.norm_before - 1.0).abs() < 1e-15);
        assert!((edge.norm_after - 1.0).abs() < 1e-15);
        assert!(
            (edge.imbalance - edge.d_mu).abs() < 1e-15,
            "imbalance {} vs d_mu {}",
            edge.imbalance,
            edge.d_mu
        );
    }

    #[test]
    fn wire_imbalance_matches_golden_for_every_numeric_op_d_vector() {
        let g = golden();
        for v in g["op_d_edge"].as_array().unwrap() {
            if v["before"].is_null() {
                continue; // null-prior: the wire emits null, we emit None
            }
            // Scalars count as 1-d vectors (the wire computes |a − b|).
            let nums = |key: &str| -> Vec<f64> {
                match &v[key] {
                    Value::Number(n) => vec![n.as_f64().unwrap()],
                    Value::Array(a) => a.iter().map(|x| x.as_f64().unwrap()).collect::<Vec<f64>>(),
                    _ => unreachable!("golden op_d before/after are numeric"),
                }
            };
            let got = imbalance(&nums("before"), &nums("after")).unwrap();
            let want = v["expect"]["imbalance"].as_f64().unwrap();
            assert!(
                (got - want).abs() <= 1e-12,
                "{}: wire imbalance {got} vs golden {want}",
                v["name"].as_str().unwrap()
            );
        }
    }

    // -- Honesty gates --------------------------------------------------------

    #[test]
    fn degenerate_states_return_none_never_a_fabricated_number() {
        let warm = default_warm_axis();
        // Zero-norm state: no direction exists.
        assert!(FieldEdge::compute(&[0.0, 0.0, 0.0], &[1.0, 2.0, 3.0], &warm).is_none());
        // Length mismatch between the states...
        assert!(FieldEdge::compute(&[1.0, 2.0], &[1.0, 2.0, 3.0], &warm).is_none());
        // ...or against the warm axis.
        assert!(FieldEdge::compute(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0], &[1.0]).is_none());
        // Empty vector, non-finite values, zero warm axis.
        assert!(FieldEdge::compute(&[], &[], &[]).is_none());
        assert!(FieldEdge::compute(&[f64::NAN, 1.0, 2.0], &[1.0, 2.0, 3.0], &warm).is_none());
        assert!(FieldEdge::compute(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0], &[0.0, 0.0, 0.0]).is_none());
        // The wire imbalance holds the same gates.
        assert!(imbalance(&[1.0], &[1.0, 2.0]).is_none());
        assert!(imbalance(&[f64::NAN], &[0.0]).is_none());
    }

    // -- The bridge onto sealed entries ---------------------------------------

    #[test]
    fn from_entry_reads_the_field_view_off_a_sealed_edge() {
        use quilt_core::CellLedger as SealedLedger;

        let mut ledger =
            SealedLedger::with_genesis("room.field", json!([0.25, -0.125, 0.5]), 1_000);
        let entry = ledger.record(
            json!([0.375, -0.0625, 0.625]),
            json!([0.375, -0.0625, 0.625]),
            3_000,
        );

        let edge = FieldEdge::from_entry(&entry, &default_warm_axis()).unwrap();
        assert!((edge.imbalance - 0.1875).abs() < 1e-12);
        assert!(edge.radial_direction_split_residual().abs() < 1e-12);
        assert!(edge.warm_pythagoras_residual().abs() < 1e-12);
        assert!(edge.projection_chain_holds(1e-12));

        // A scalar cell has a ledger view but no field view.
        let mut scalar = SealedLedger::with_genesis("bilge.level", json!(40.0), 0);
        let e = scalar.record(json!(85.0), json!(85.0), 1_000);
        assert!(FieldEdge::from_entry(&e, &default_warm_axis()).is_none());
    }
}
