# field-edge-bridge

Proof-of-identity prototype for
[docs/field-edge-ledger-bridge.md](../../docs/field-edge-ledger-bridge.md):
the cell-ledger's `imbalance` and the elephant's field-edge (`vmf.edge`) are
two projections of one vector — the edge `Δ = after − before`.

Runs the golden `vector-field-edge` from `compat/golden.json`, computes both
views, and asserts four exact bridge identities (incl. `imbalance == d_mu`
for unit states) at 1e-12. Python + numpy only; no build, not a workspace
member, nothing to commit but the two files.

```sh
python3 crates/field-edge-bridge/bridge_demo.py
```
