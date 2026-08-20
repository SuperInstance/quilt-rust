/*
 * quilt_cabi.h — the stable C ABI over the native quilt core.
 *
 * One native core, N language bindings: any runtime (Python ctypes, Go
 * cgo, Julia @ccall, R dyn.load, Node FFI, WASM toolchains) binds the
 * reference Rust engine + ledger through these symbols instead of
 * reimplementing them. This is the C tier of docs/quilt-compat-contract.md
 * §5: passthrough + bit-for-bit chain verification against
 * compat/golden.json. Full contract: docs/c-abi.md.
 *
 * MEMORY CONTRACT
 * ---------------
 *   - Caller allocates and owns every `const char *` argument. The
 *     library borrows them only for the duration of the call; they must
 *     be NUL-terminated UTF-8 and are never stored or freed by the
 *     library.
 *   - Strings RETURNED by the library (char *, non-const) are
 *     library-allocated and must be released with quilt_string_free() —
 *     never free().
 *   - Handles are caller-owned: quilt_engine_new() / quilt_engine_free().
 *     All handle functions tolerate NULL (an error, not a crash).
 *   - Errors never unwind: int returns use 0 = ok / negative = error;
 *     string returns use NULL = error; detail sits in quilt_last_error()
 *     until the next call on that thread.
 *
 * THREAD SAFETY
 * -------------
 *   - Engine handles are Send + Sync; the ledger registry is a global
 *     mutex-guarded map; quilt_last_error() is per-thread. Any function
 *     may be called from any thread. Do not free a handle or string
 *     while another thread is using it.
 */

#ifndef QUILT_CABI_H
#define QUILT_CABI_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ABI version of this header. Check against quilt_abi_version() at load
 * time and fail loudly on mismatch. */
#define QUILT_ABI_VERSION 1

/* Opaque engine handle. Create with quilt_engine_new, destroy with
 * quilt_engine_free. */
typedef struct QuiltEngine QuiltEngine;

/* ---- version ---------------------------------------------------------- */

/* The library's ABI version (compare to QUILT_ABI_VERSION). */
uint32_t quilt_abi_version(void);

/* ---- engine lifecycle --------------------------------------------------- */

/* Create a fresh, empty engine. NULL only on allocation failure. */
QuiltEngine *quilt_engine_new(void);

/* Load a YAML sheet (resets all cell state). `yaml` is borrowed for the
 * call only. Returns 0 on success, -1 on error (quilt_last_error). */
int quilt_engine_load_sheet(QuiltEngine *engine, const char *yaml);

/* Read a cell's current value as JSON text ("80.0", "true", "\"idle\"").
 * Evaluates formula cells first. Returns a library-allocated string the
 * caller frees with quilt_string_free, or NULL on error. */
char *quilt_engine_get(QuiltEngine *engine, const char *cell_id);

/* Write a cell's value and propagate downstream. `value_json` is any JSON
 * value text. Works for every cell kind; for sensor/io cells this is
 * exactly a push. Returns 0 on success, -1 on error. */
int quilt_engine_set(QuiltEngine *engine, const char *cell_id,
                     const char *value_json);

/* Destroy an engine. Tolerates NULL. */
void quilt_engine_free(QuiltEngine *engine);

/* ---- the ledger (process-global book of books, keyed by cell id) ------- */

/* Create the cell's ledger with a genesis state (JSON) committed at
 * ts_millis; the genesis is sealed into the chain root. Fails (-1) if a
 * ledger already exists for the cell. 0 on success. */
int quilt_ledger_init(const char *cell_id, const char *genesis_json,
                      uint64_t ts_millis);

/* Record a complete double entry (input in, output out, at ts_millis) and
 * return the entry's seal — 64 lowercase hex chars — as a
 * library-allocated string (free with quilt_string_free), or NULL on
 * error. If no ledger exists yet, one is created WITHOUT a genesis, so
 * the first edge is a null-prior edge; call quilt_ledger_init first when
 * golden chain roots are required. */
char *quilt_ledger_record(const char *cell_id, const char *input_json,
                          const char *output_json, uint64_t ts_millis);

/* Recompute every seal and prev-link. Returns 1 intact, 0 broken,
 * -1 no such ledger. */
int quilt_ledger_verify(const char *cell_id);

/* Reconcile the books: matched pairs, open inputs, chain + continuity
 * integrity, total/mean surprise, balanced. Returns the Reconciliation
 * JSON (docs/cell-ledger.md schema, the fields compat/golden.json op (e)
 * pins) as a library-allocated string, or NULL on error. */
char *quilt_ledger_reconcile(const char *cell_id);

/* The chain head: the last entry's seal, or the genesis commit for an
 * empty ledger. Library-allocated string; NULL on error. */
char *quilt_ledger_chain_hash(const char *cell_id);

/* Drop every ledger in the global registry (tests, clean shutdown).
 * Engine handles are unaffected. Returns 0. */
int quilt_ledgers_reset(void);

/* ---- memory ------------------------------------------------------------- */

/* Free a string returned by this library. Do not call free(). Tolerates
 * NULL. Any other pointer is undefined behavior. */
void quilt_string_free(char *s);

/* The last error message from this thread's most recent quilt call, or ""
 * if it succeeded. Borrowed pointer: valid until the next quilt call on
 * the same thread; never NULL. */
const char *quilt_last_error(void);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* QUILT_CABI_H */
