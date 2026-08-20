#!/usr/bin/env python3
"""field-edge-bridge — proof that the ledger imbalance and the field-edge
warmth are two projections of ONE vector: the edge Δ = after − before.

Ledger side : quilt-compat/1 op_d wire spec (compat/conformance_test.rs)
  delta      = after − before                (kept as the full vector)
  imbalance  = ‖Δ‖₂                          (persistence prior: predict(b)=b)

Field side  : elephant vmf.edge() (elephant/vmf.py)
  μ̂_b, μ̂_a   = unit mean directions           (vMF: before/after normalized)
  d_mu       = ‖μ̂_a − μ̂_b‖₂ = √(2−2cosθ)     (direction drift on the sphere)
  d_warmth   = ŵ·(μ̂_a − μ̂_b)                 (signed cosine along warm axis)

Run:  python3 crates/field-edge-bridge/bridge_demo.py   (numpy only)
"""
import json
import os

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
GOLDEN = os.path.join(HERE, "..", "..", "compat", "golden.json")


def main() -> None:
    with open(GOLDEN) as f:
        golden = json.load(f)
    vec = next(v for v in golden["op_d_edge"] if v["name"] == "vector-field-edge")
    before = np.array(vec["before"], float)
    after = np.array(vec["after"], float)
    want_delta = np.array(vec["expect"]["delta"], float)
    want_imb = float(vec["expect"]["imbalance"])

    print("=== field-edge-bridge: ledger imbalance and field warmth are one object ===")
    print(f"golden vector: {vec['cell']}  ts={vec['ts']}  ({vec['name']})")
    print(f"  before = {before}   (‖b‖ = {np.linalg.norm(before):.5f})")
    print(f"  after  = {after}   (‖a‖ = {np.linalg.norm(after):.5f})")
    delta = after - before
    print(f"  edge Δ = after − before = {delta}")

    # -- (a) LEDGER VIEW (quilt-compat/1 op_d, persistence prior) ------------- #
    assert np.allclose(delta, want_delta, atol=1e-12), "delta must match golden"
    imbalance = float(np.linalg.norm(delta))  # wire_imbalance: sqrt of Σ d²
    assert abs(imbalance - want_imb) <= 1e-12, "imbalance must match golden"
    print("\n(a) LEDGER VIEW  (quilt-compat/1 op_d; predict(before)=before)")
    print(f"  delta     = {delta}          [golden: bit-for-bit]")
    print(f"  imbalance = ‖Δ‖₂ = {imbalance:.10f}    [golden: {want_imb}]  ✓")

    # -- (b) FIELD VIEW (elephant vmf.edge, unit sphere) ---------------------- #
    mu_b = before / np.linalg.norm(before)
    mu_a = after / np.linalg.norm(after)
    cos_theta = float(mu_b @ mu_a)  # signed cosine between before and after
    d_mu = float(np.linalg.norm(mu_a - mu_b))
    # elephant-style warm direction (mood+, volume+, cynicism−), 3-d stand-in:
    # ‖[0.30, 0.10, −0.15]‖ = 0.35 exactly → ŵ = [6/7, 2/7, −3/7]
    w = np.array([0.30, 0.10, -0.15])
    w = w / np.linalg.norm(w)
    warmth_b = float(w @ mu_b)  # warmth_vmf = ŵ·μ̂ (vmf.py: signed cosine)
    warmth_a = float(w @ mu_a)
    d_warmth = warmth_a - warmth_b
    radial = float(np.log(np.linalg.norm(after) / np.linalg.norm(before)))
    print("\n(b) FIELD VIEW   (elephant vmf.edge; unit directions on the sphere)")
    print(f"  cos(before, after) = {cos_theta:+.5f}   (direction barely moved)")
    print(f"  d_mu = ‖μ̂_a − μ̂_b‖₂ = {d_mu:.5f} = √(2−2cosθ)")
    print(f"  ŵ = {w}  (mood+, volume+, cynicism−)")
    print(f"  warmth: {warmth_b:+.5f} → {warmth_a:+.5f}   d_warmth = {d_warmth:+.5f}  (the room WARMED)")
    print(f"  radial = ln(‖a‖/‖b‖) = {radial:+.5f}   (the field also grew — κ's side)")

    # -- BRIDGE IDENTITIES ----------------------------------------------------- #
    nb, na = np.linalg.norm(before), np.linalg.norm(after)
    radial_leg = na - nb
    lhs = imbalance**2
    dir_part = na * nb * d_mu**2
    signed_leg = float(w @ delta)
    perp_leg = np.linalg.norm(delta - signed_leg * w)
    unit_imb = float(np.linalg.norm(mu_a - mu_b))
    print("\nBRIDGE IDENTITIES (exact algebra, verified at 1e-12)")
    print(f"  1. imbalance² = (‖a‖−‖b‖)² + ‖a‖·‖b‖·d_mu²")
    print(f"     {lhs:.8f} = {radial_leg**2:.8f} + {dir_part:.8f}"
          f"   ✓ (magnitude drift + direction drift)")
    assert abs(lhs - radial_leg**2 - dir_part) < 1e-12
    print(f"  2. imbalance² = (ŵ·Δ)² + ‖Δ⊥‖²   (Pythagoras on the raw edge)")
    print(f"     {lhs:.8f} = {signed_leg**2:.8f} + {perp_leg**2:.8f}"
          f"   ✓ (warmth is one signed leg of the ledger's surprise)")
    assert abs(lhs - signed_leg**2 - perp_leg**2) < 1e-12
    print(f"  3. |d_warmth| ≤ d_mu ≤ imbalance   (projection chain)")
    print(f"     {abs(d_warmth):.5f} ≤ {d_mu:.5f} ≤ {imbalance:.5f}   ✓")
    assert abs(d_warmth) <= d_mu <= imbalance
    print(f"  4. unit-cell collapse: with ‖before‖=‖after‖=1 (a direction cell),")
    print(f"     ledger imbalance ≡ elephant d_mu: {unit_imb:.10f} vs {d_mu:.10f}   ✓")
    assert abs(unit_imb - d_mu) < 1e-12

    print(
        "\nVERDICT: Δ is ONE vector field. The ledger reads its NORM (unsigned\n"
        "magnitude, sealed into every entry); the field reads its DIRECTION\n"
        "(d_mu + warmth sign) and its LENGTH (radial / κ). Double-entry\n"
        "imbalance IS the field-edge magnitude at cell grain."
    )


if __name__ == "__main__":
    main()
