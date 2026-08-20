# The Mojo tier — Python ergonomics at CUDA speed, one sheet underneath

Status: **design note.** Mojo (modular.com) is young, moving fast, and its
toolchain is not on this box — nothing here runs today. This note is where it
slots in the tier map, how it binds, and the first concrete use that would
earn it a row in `docs/quilt-compat-contract.md` §5.

## The shape (one paragraph)

Mojo is Python-syntax-at-CUDA-speed: it compiles through MLIR/LLVM to native
and GPU code for the same AI workloads CUDA serves, but with Python
ergonomics and fine-grained control (`fn` strictness, SIMD/vector types,
`@always_inline`, compile-time parameters). For the fleet it is a **dual-role
tier claimant**: a **gym** substrate alongside CUDA/PyTorch (field math,
world-model stepping, vMF-class fits over ledger corpora) *and* a
**parallel/optimizing** substrate alongside Go — not routing-parallel like the
relay, but compiler-parallel: batch edge processing where what removes
overhead is lowering, not scheduling. It joins the fleet the way every tier
does, by reproducing `compat/golden.json`, and while the language settles the
C ABI (`docs/c-abi.md`) is the bridge that hands it reference semantics by
construction.

## 1. Where Mojo slots in

| Role | Neighbors | What Mojo would own |
| --- | --- | --- |
| gym (cortex-and-gym speed) | CUDA/PTX, PyTorch, Julia/R | kernel-shaped ledger math: batch edges, field fits, the JEPA-side consumption of committed corpora |
| parallel / optimizing | Go | the Go synapse buffers and forwards edges; Mojo would *transform* them in bulk — the relay's fast cousin for compute, not routing |

Three-speed reading (`fleet-as-fractal-jepa`): Mojo lives in the gym, the
work the cortex ships out when it has budget.

## 2. Bind the C ABI vs reimplement the vectors

The two strategies of `docs/c-abi.md`, both apply, in order:

- **Near-term — bind.** `extern "C"` declarations of `quilt_cabi.h`'s
  signatures against `libquilt_cabi.so`: engine, ledger, seals all arrive by
  construction; conformance is the ABI smoke suite re-run from Mojo, plus
  golden ops (a)/(b)/(e) asserted from Mojo-side. The C ABI is *more stable
  than the language* — binding it insulates a young tier from Mojo churn.
- **Long-term — reimplement.** A native Mojo port of the five ops is the
  differential test that proves the *spec* portable (the contract is real,
  not Rust-shaped). Mojo's MLIR foundation is the interesting lever: the same
  edge `fn` lowers to CPU or GPU backends, and a Mojo-native ledger could
  eventually share dialects with other MLIR frontends. Conformance is
  declared once per op, not per backend — the vectors pin *outputs*, not
  lowerings.

## 3. One sheet, many substrates, when the substrate is an MLIR kernel

The sheet is data; the edge is smaller than any IR. "One sheet, many
substrates" means a Mojo kernel is just another interpreter of the same
`bilge-reflex` graph: author once in the canonical sheet experience, and
whether op (d) executes in the Rust engine, a Mojo `@always_inline fn`, or a
CUDA kernel is a throughput decision the golden vectors make invisible. The
one discipline MLIR lowering adds: fast-math reassociation may drift within
the declared tolerance, so dyadic golden values (0.125, 0.1875) must come out
*exactly* — they are canaries for accidental reassociation.

## 4. Recommended first use

The field-edge batch kernel + vMF fit, as a single Mojo file:

- `@always_inline fn edge(before, after) -> (delta, imbalance)` over the op
  (d) vectors — scalar `45.0/45.0`, 3-vector delta `[0.125, 0.0625, 0.125]`
  and imbalance `0.1875` (dyadic → exact), null-prior → `null, null` (never
  fake a number).
- Sealing the op (e) transcript: genesis root and three seals bit-for-bit
  (`4a7ad648…`), which forces the port of canonical JSON — the float-marker
  rule (`85.0`, never `85`) is the known hazard, §2 of the contract.
- Declared class: (a) exact, (b) 1e-9, (c) exact, (d) 1e-6 fit-class,
  (e) bit-for-bit, (e′) 1e-6 — at or below every gate.

## 5. Maturity, honestly

Mojo is young: breaking syntax changes, stdlib churn, packaging in flux, and
a single vendor. The bet is hedged twice — the C-ABI path works *today* and
survives language churn, and the native port only pays off if the language
holds still long enough to be worth targeting. Revisit quarterly: if the
MLIR/GPU story matures, Mojo is the natural home for the gym's non-PyTorch
kernels; if it stalls, the C ABI loses nothing.

## Next moves (3)

1. Mojo conformance harness skeleton mirroring `compat/conformance_test.rs`,
   driving `libquilt_cabi.so` via `extern "C"` — parse `golden.json` at
   runtime, check the contract id, assert ops (a)–(e).
2. The `@always_inline` field-edge kernel of §4 as one self-testing file:
   dyadic deltas exact, provenance + seals bit-for-bit, null-prior honored.
3. Differential vMF fit: consume a committed ledger corpus through the ABI,
   fit in Mojo, compare against the Julia/R representation tier at fit ≤ 1e-6
   — the first cross-tier differential test with Mojo in it.
