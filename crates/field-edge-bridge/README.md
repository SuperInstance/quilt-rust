# field-edge-bridge

**Proof-of-identity prototype:** the cell-ledger's `imbalance` and the
elephant's field-edge (`vmf.edge`) are two projections of **one vector** —
the edge `Δ = after − before`.

Companion to [docs/field-edge-ledger-bridge.md](../../docs/field-edge-ledger-bridge.md).
Every identity below is verified to `1e-12` against the golden
`vector-field-edge` from `compat/golden.json`.

```sh
python3 crates/field-edge-bridge/bridge_demo.py    # numpy only, self-checking
```

---

## The claim

A cell transition is a directed edge `Δ = after − before`, recorded by the
thing that changed. Two systems read that one edge, each through its own lens:

| Lens | Reader | What it reads | Quantity |
|------|--------|---------------|----------|
| **Ledger** | quilt-compat/1 `op_d` wire spec | the **norm** of the edge | `imbalance = ‖Δ‖₂` |
| **Field** | elephant `vmf.py` | the **direction** and **length** of the edge | `d_mu`, `d_warmth`, radial/κ |

The ledger seals unsigned surprise into every hash-chained entry; the field
keeps signed valence and the distribution shape. It is the same object,
double-entry at cell grain.

## The two views

### (a) Ledger view — `imbalance = ‖Δ‖₂`

Under the persistence prior (`predict(b) = b`), surprise is the edge itself:

```
delta      = after − before            # kept as the full vector
imbalance  = ‖Δ‖₂                      # sealed into the ledger entry
```

### (b) Field view — the unit sphere

The elephant normalizes before/after states to unit directions and reads
drift on the sphere:

```
μ̂_b, μ̂_a  = before/after unit directions          (vMF)
d_mu       = ‖μ̂_a − μ̂_b‖₂ = √(2 − 2·cosθ)         (direction drift)
d_warmth   = ŵ·(μ̂_a − μ̂_b)                          (signed cosine along warm axis)
radial     = ln(‖a‖ / ‖b‖)                          (the field also grew — κ's side)
```

For the golden vector `room.field`, `ŵ = [6/7, 2/7, −3/7]` — the elephant's
warm direction (mood+, volume+, cynicism−), a 3-d stand-in.

## The four bridge identities (verified at 1e-12)

1. **Magnitude + direction split**

   ```
   imbalance² = (‖a‖ − ‖b‖)² + ‖a‖·‖b‖·d_mu²
   ```
   The ledger's scalar conflates radial and directional drift; this recovers
   both exactly.

2. **Pythagoras on the raw edge**

   ```
   imbalance² = (ŵ·Δ)² + ‖Δ⊥‖²
   ```
   Warmth is one *signed* leg of the ledger's surprise. The ledger discards
   the sign; the field keeps it.

3. **Projection chain**

   ```
   |d_warmth| ≤ d_mu ≤ imbalance
   ```
   Signed warmth is the weakest projection, direction drift the middle, the
   full norm the strongest — the three never disagree.

4. **Unit-cell collapse**

   With `‖before‖ = ‖after‖ = 1` (a direction cell), the ledger imbalance and
   the elephant's `d_mu` are the **same number**:

   ```
   imbalance ≡ d_mu      (bit-for-bit at 1e-12)
   ```

## The golden vector

`room.field` (from `compat/golden.json`, `op_d_edge`):

| Quantity | Value |
|----------|-------|
| `before` | `[0.25, −0.125, 0.5]` |
| `after`  | `[0.375, −0.0625, 0.625]` |
| `Δ`      | `[0.125, 0.0625, 0.125]` |
| `imbalance` | `0.1875` (golden: bit-for-bit) |
| `cos(before, after)` | `+0.9881` |
| `d_mu`   | `0.1542` |
| `d_warmth` | `+0.1112` (the room warmed) |
| `radial` | `ln(‖a‖/‖b‖) = +0.2446` (the field grew) |

## Files

```
field-edge-bridge/
├── README.md        # this document
└── bridge_demo.py   # the proof — computes both views, asserts all four identities
```

## Why this matters

The ledger and the field are not two systems that happen to agree — they are
two reads of one transition. `imbalance` **is** the field-edge magnitude at
cell grain, and the field-edge is the ledger's surprise with its sign and
shape restored. The fleet is a fractal JEPA because every cell is a room with
its own first-person edge.

## Status

Python + numpy only; no build; **not a workspace member** — nothing to
compile, nothing to commit but these two files. Run it anywhere:
