# The C ABI — one native core, N language bindings

Status: **v1, in force.** Artifacts: `crates/quilt-cabi` (cdylib + staticlib),
header `crates/quilt-cabi/quilt_cabi.h`, smoke harness `crates/quilt-cabi/smoke/`.
Conformance class (per `docs/quilt-compat-contract.md` §4): **passthrough + bit-for-bit**
— the ABI computes nothing it does not own; hashes and evaluation happen in
`quilt-core`; seals are still verified bit-for-bit against `compat/golden.json`.

## Why this is the keystone

The compat contract's default strategy is *differential testing*: independent
ports (Python, Go, …) each reimplement the edge math and prove agreement against
`golden.json`. The C ABI is the other strategy, and it is the "clever with code"
keystone because it makes the reference implementation itself the shared
substrate: any language with an FFI — Python ctypes, Go cgo, Julia `@ccall`,
R `dyn.load`, Node, WASM toolchains — binds the *same* Rust engine and ledger
instead of porting them, so reference-tier semantics, canonical JSON, and chain
seals arrive by construction rather than by re-proof, and raw speed needs no
reimplementation. The two strategies back each other: the ports verify that the
*spec* is portable (the contract is real, not Rust-shaped), while the ABI makes
portability *unnecessary for performance* — and `golden.json` guards the seam so
an ABI regression is caught as a contract failure, not a binding bug.

## Build

```sh
cargo build -p quilt-cabi              # target/debug/libquilt_cabi.{so,a}
cargo build -p quilt-cabi --release    # target/release/libquilt_cabi.{so,a}
cargo test -p quilt-cabi               # Rust-side golden smoke (ops a, b, e)
crates/quilt-cabi/smoke/run.sh         # C harness: compiles + links the .so, runs 26 golden checks
crates/quilt-cabi/smoke/run.sh release
```

The smoke fixtures (`smoke/sheet.yaml`, `smoke/golden_vectors.h`) are
**generated** from `compat/golden.json` by `smoke/gen-sheet.py` — never
hand-copied; the generator refuses a `golden.json` whose contract id it does
not know.

## Function signatures

`quilt_cabi.h` is normative; this is the summary.

| Function | Signature | Returns |
| --- | --- | --- |
| `quilt_abi_version` | `uint32_t quilt_abi_version(void)` | ABI version; compare to `QUILT_ABI_VERSION` (currently `1`) at load time |
| `quilt_engine_new` | `QuiltEngine *quilt_engine_new(void)` | owned handle, or `NULL` on allocation failure |
| `quilt_engine_load_sheet` | `int quilt_engine_load_sheet(QuiltEngine *, const char *yaml)` | `0` ok / `-1` parse-or-load error; resets all cell state |
| `quilt_engine_get` | `char *quilt_engine_get(QuiltEngine *, const char *cell_id)` | the cell's current value as JSON text (`"80.0"`, `"true"`, `"\"idle\""`); evaluates formulas first; `NULL` on error |
| `quilt_engine_set` | `int quilt_engine_set(QuiltEngine *, const char *cell_id, const char *value_json)` | writes the value and propagates downstream; works for every cell kind (for sensor/io cells this is exactly a push); `0` / `-1` |
| `quilt_engine_free` | `void quilt_engine_free(QuiltEngine *)` | destroys the handle; tolerates `NULL` |
| `quilt_ledger_init` | `int quilt_ledger_init(const char *cell_id, const char *genesis_json, uint64_t ts_millis)` | creates the cell's ledger with a genesis committed at `ts_millis` (sealed into the chain root); `-1` if the ledger already exists — a genesis cannot be retrofitted |
| `quilt_ledger_record` | `char *quilt_ledger_record(const char *cell_id, const char *input_json, const char *output_json, uint64_t ts_millis)` | the new entry's **seal** (64 lowercase hex); auto-creates a genesis-less ledger if absent, so a first edge is a null-prior edge (no `expected`, no `imbalance` — never fake a number) |
| `quilt_ledger_verify` | `int quilt_ledger_verify(const char *cell_id)` | `1` intact / `0` broken / `-1` no such ledger |
| `quilt_ledger_reconcile` | `char *quilt_ledger_reconcile(const char *cell_id)` | the `Reconciliation` JSON (`docs/cell-ledger.md`; the fields `golden.json` op (e) pins) |
| `quilt_ledger_chain_hash` | `char *quilt_ledger_chain_hash(const char *cell_id)` | head seal, or the genesis commit for an empty ledger |
| `quilt_ledgers_reset` | `int quilt_ledgers_reset(void)` | drops the process-global ledger registry (tests, shutdown) |
| `quilt_string_free` | `void quilt_string_free(char *s)` | frees a library-returned string; tolerates `NULL` |
| `quilt_last_error` | `const char *quilt_last_error(void)` | detail for the last failed call on this thread; `""` if none |

Ledger functions operate on a process-global **book of books** (one
hash-chained `CellLedger` per cell id, mutex-guarded) — which is why their
signatures take only `cell_id`. The engine is `Send + Sync` (interior locking);
every function may be called from any thread.

## Memory contract

1. **Caller allocates, caller owns** every `const char *` argument. The library
   borrows it only for the duration of the call (NUL-terminated UTF-8); it is
   never stored or freed.
2. **Library-allocated return strings** (`quilt_engine_get`,
   `quilt_ledger_record`, `quilt_ledger_reconcile`, `quilt_ledger_chain_hash`)
   must be freed with `quilt_string_free` — never `free()`, which may live on
   another allocator.
3. **Handles** are caller-owned (`quilt_engine_new` / `quilt_engine_free`);
   all handle functions tolerate `NULL` (an error, not a crash).
4. **Errors never unwind.** Integer returns: `0` = ok, negative = error.
   String returns: `NULL` = error. Detail: `quilt_last_error()`, valid until
   the next call on that thread. Rust panics are caught at the boundary and
   converted to this convention.
5. **Timestamps** are `uint64_t` milliseconds since the unix epoch — the wire
   `ts` of the edge schema, as u64.

## Binding recipes (the N in "N bindings")

**Python (ctypes)**

```python
import ctypes
lib = ctypes.CDLL("target/release/libquilt_cabi.so")
lib.quilt_engine_get.restype = ctypes.c_char_p
lib.quilt_engine_get.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
engine = lib.quilt_engine_new()
# free returned strings via lib.quilt_string_free (restype c_void_p to keep the pointer)
```

**Go (cgo)** — `// #cgo LDFLAGS: -L../target/release -lquilt_cabi` + `// #include "quilt_cabi.h"`,
then `C.quilt_engine_new()`, `C.GoString(C.quilt_engine_get(...))`, and
`C.quilt_string_free(...)` when done.

**Julia** — `const lib = Libc.dlopen("libquilt_cabi.so")` and
`ccall((:quilt_engine_get, lib), Cstring, (Ptr{Cvoid}, Cstring), e, id)`;
free with `ccall((:quilt_string_free, lib), Cvoid, (Cstring,), s)`.

**R / Node / WASM** — same pattern: `dyn.load`, `ffi-napi`/Koffi, or a
WASM build of the same crate; the header is the contract in every case.

## Conformance

The smoke suites assert `compat/golden.json` ops **(a)** value read (exact JSON
equality), **(b)** formula eval + reactive propagation (`40.0/80.0 → -20.0`,
post-push `85.0 → 2.5`, `true`), and **(e)** ledger chain — genesis root, three
seals **bit-for-bit**, chain head, and the reconcile report (entries 3, matched
pairs 3, balanced, total surprise `47.5`, mean `15.833333333333334`). Op (c)
(propagation order) is engine-internal; op (d) (edge math) belongs to the
producing tier — the ABI passes both through untouched. Per the tier map this
layer's numbers are the reference tier's numbers.

The Rust harness (`tests/abi_smoke.rs`) parses `golden.json` at runtime, like
the reference harness; the C harness (`smoke/smoke.c`) links the actual
`libquilt_cabi.so` and asserts the same vectors — 26 checks, exit code 0/1.
