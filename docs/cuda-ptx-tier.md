# The CUDA / PTX tier — raw speed for the gym, golden vectors as the gate

Status: **design note.** No GPU on this box; nothing here runs today. This is
the design + kernel-pseudocode note for the **raw speed** tier of the compat
contract §5 — the floor Cosmos, Isaac, and PyTorch all lower onto.

## The shape (one paragraph)

CUDA C++ — and PTX, its low-level instruction set — is where the edge
function stops being a per-record bookkeeping call and becomes a batched
kernel: N edges in, deltas and imbalance out, with the vMF fit and
world-model stepping as the heavy tail. The contract does not bend for the
hardware: fast-math is allowed at 1e-6 on ops (b)/(d), chain hashes stay
bit-for-bit, and the kernel earns its row by reproducing `golden.json` in
the gym's CI like every float tier. PTX appears here only as the escape
hatch we reach for last.

## 1. What gets accelerated

The per-edge math is embarrassingly parallel — this is why the tier exists:

- **The edge record (op d):** `delta = after − before` element-wise (scalar
  or fixed-width field vector), `imbalance = ‖after − before‖₂` under the
  persistence prior; null-prior edges emit `null/null`.
- **The vMF fit (op b″ class):** the iterative solver over a batch of field
  edges — `fit ≤ 1e-6` precedent — the first genuinely heavy kernel.
- **Batch reconcile (op e′):** totals and surprise are reductions over N
  entries; the reconciling *verdict* stays exact. World-model stepping is
  the same shape at larger grain, fed by ledger corpora.

## 2. The golden gate

Tolerance row for this tier (already pinned in the contract): (a) exact,
(b) 1e-6, (c) exact, (d) 1e-6, (e) **bit-for-bit**, (e′) 1e-6. The gate is
`golden.json` ops (d)–(e) reproduced on whatever GPU the gym rents —
conformance per-release, not per-claim. One real hazard: fast-math is
irrelevant to SHA-256 (integer math, exact), but the hash *preimage*
includes canonically rendered floats (ryū shortest-round-trip, float marker
preserved) — a GPU sealer that prints `85` for `85.0` breaks every seal
silently. **Recommended split:** the kernel computes (b)/(d)-class math at
1e-6; canonical serialization + hashing (e) stays on the host through the
reference core — hash-on-GPU only with an exact ryū port.

## 3. PTX, reached for LAST

The escalation ladder: compiler flags (`-O3`, `-use_fast_math` where the 1e-6
budget allows, `__launch_bounds__`) → warp primitives (`__shfl_down_sync`,
`__reduce_add_sync`) → inline `asm` PTX for one measured hot instruction →
standalone PTX only when `ncu` profiling *proves* the compiler leaves
throughput on the table. Why last: PTX couples us to ISA churn (sm_70 →
sm_90), is unreadable in review, and every hand-written blob must
independently re-prove golden conformance. The 1e-6 budget already licenses
the compiler's fastest float paths; take the free speed first.

## 4. The memory story — one warp per edge

Batch of N edges, each `{before, after, ts}` = **one warp's work** (32 lanes
cover the widest field vector; a 3-component field leaves 29 lanes idle —
fine, the batch is the point). Structure-of-arrays device buffers so
consecutive warps read consecutive edges coalesced; `ts` rides along as u64
millis; a per-warp shuffle reduction produces the L2 norm.

```
__global__ void edge_batch(const float* before, const float* after,  // SoA: N × D
                           float* delta, float* imbalance, int N, int D)
{
    int e  = (blockIdx.x * blockDim.x + threadIdx.x) / 32;   // one edge per warp
    int ln = threadIdx.x & 31;
    if (e >= N) return;
    float d = 0.f;
    if (ln < D) d = after[e*D + ln] - before[e*D + ln];      // coalesced loads
    if (ln < D) delta[e*D + ln] = d;                         // op (d), gate 1e-6
    float sq = d * d;
    for (int s = 16; s; s >>= 1) sq += __shfl_down_sync(~0u, sq, s);
    if (ln == 0) imbalance[e] = sqrtf(sq);                   // ‖after−before‖₂
    // (e): host-side — canonical JSON + SHA-256 via the C ABI, bit-for-bit
}
```

## 5. The bridge — C ABI + CUDA runtime

The host process (Rust via `crates/quilt-cabi`, or the Python gym through
the same `.so` / a torch extension) links `libquilt_cabi.so` for sheet
semantics, ledger, seals — passthrough + bit-for-bit, per `docs/c-abi.md` —
and the CUDA runtime for throughput. The kernel is just another producing
tier posting edges; op (e) lands bit-for-bit or the tier does not conform.

## Next moves (3)

1. Land the CUDA conformance harness skeleton (ops a–e, `golden.json` parsed
   at runtime, contract id checked) so any rented GPU CI can run it unchanged.
2. Implement `edge_batch` + the host bridge; assert the op (d) vectors
   (scalar 45.0, 3-vector 0.1875, null-prior) and the op (e) transcript with
   the §2 host-hashing split.
3. Benchmark vs the Rust reference batch on the first rented GPU, profile
   with `ncu`, then rule on PTX — expected answer: not yet.
