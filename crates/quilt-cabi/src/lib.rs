//! # quilt-cabi — the stable C ABI over the native quilt core
//!
//! ## Role in the system
//!
//! This is the **C tier** of `docs/quilt-compat-contract.md` §5: the ABI
//! boundary any runtime can `dlopen`. Python (ctypes), Go (cgo), Julia
//! (`@ccall`), R (`.Call`/`dyn.load`), Node (ffi-napi), and WASM toolchains
//! all bind **the same native engine and ledger** instead of reimplementing
//! them — "one native core, N language bindings". The per-op conformance
//! class is *passthrough + bit-for-bit*: this layer computes nothing it does
//! not own (hashes and evaluation happen in `quilt-core`), passes bytes
//! through, and still verifies chain seals bit-for-bit against
//! `compat/golden.json`.
//!
//! ## Depends on
//!
//! - `quilt-core` — `QuiltEngine` (sync `get`/`set`/`load_sheet`) and
//!   `CellLedger` (record / verify_chain / reconcile / chain_hash).
//! - `serde_json` — values cross the boundary as JSON text; the caller's
//!   language parses what it understands.
//! - `once_cell` — the process-global ledger registry ("book of books").
//!
//! ## Memory contract (the whole discipline, in one place)
//!
//! - **Caller allocates, caller owns** every `const char *` argument. The
//!   library borrows them only for the duration of the call; it never stores
//!   or frees them. Strings must be NUL-terminated UTF-8.
//! - **Library-allocated return strings** (`char *` from `quilt_engine_get`,
//!   `quilt_ledger_record`, `quilt_ledger_reconcile`,
//!   `quilt_ledger_chain_hash`) must be released by the caller with
//!   [`quilt_string_free`] — never with `free()`, which may live on a
//!   different allocator.
//! - **Handles** (`QuiltEngine *`) are owned by the caller: create with
//!   [`quilt_engine_new`], destroy with [`quilt_engine_free`]. Every handle
//!   function tolerates `NULL` (an error, not a crash).
//! - **Errors** never cross as exceptions. Integer returns use `0` = ok /
//!   negative = error; string returns use `NULL` = error; the human-readable
//!   detail is available from [`quilt_last_error`] until the next call on
//!   that thread.
//! - **No panics cross the boundary.** Every entry point catches unwinding
//!   panics and converts them to the error convention above.
//!
//! ## Thread safety
//!
//! The engine handle is `Send + Sync` (interior locking in `quilt-core`).
//! The ledger registry is a global `Mutex`-guarded map. `quilt_last_error`
//! is per-thread. It is safe to call any function from any thread; it is
//! *not* safe to free a handle or a string while another thread is using it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::null_mut;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde_json::Value;

use quilt_core::ledger::CellLedger;
use quilt_core::types::CallerContext;

/// The ABI version. Bump on any breaking change to the function set,
/// signatures, or memory contract. Pinned as `QUILT_ABI_VERSION` in
/// `quilt_cabi.h`; bindings should check it at load time.
pub const QUILT_ABI_VERSION: u32 = 1;

/// Opaque engine handle. Created by [`quilt_engine_new`], destroyed by
/// [`quilt_engine_free`]. Foreign code must treat it as an opaque pointer.
pub struct QuiltEngine {
    inner: quilt_core::QuiltEngine,
}

/// The process-global "book of books": one hash-chained [`CellLedger`] per
/// cell id, so the C surface can stay handle-free for ledger operations
/// (the task's `quilt_ledger_record(cell_id, …)` shape). Keyed by the
/// stable cell address, exactly like the wire edge's `cell` field.
static LEDGERS: Lazy<Mutex<HashMap<String, CellLedger>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

thread_local! {
    /// The last error message produced on this thread. Borrowed by
    /// `quilt_last_error`; valid until the next library call.
    static LAST_ERROR: RefCell<String> = RefCell::new(String::new());
}

// ---------------------------------------------------------------------------
// Internal helpers — the error/panic discipline
// ---------------------------------------------------------------------------

type FfiResult<T> = Result<T, String>;

fn clear_err() {
    LAST_ERROR.with(|slot| slot.borrow_mut().clear());
}

fn set_err(msg: impl Into<String>) {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = msg.into());
}

/// Run `f`, converting any Rust panic into an `Err` — the C boundary must
/// never see an unwind.
fn catch_panics<T>(f: impl FnOnce() -> FfiResult<T>) -> FfiResult<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(payload) => {
            let detail = if let Some(s) = payload.downcast_ref::<&'static str>() {
                (*s).to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "non-string panic payload".to_string()
            };
            Err(format!(
                "quilt-cabi: rust panic caught at the C boundary: {detail}"
            ))
        }
    }
}

/// Borrow a caller-owned C string. NULL or invalid UTF-8 is an error, not
/// a crash. The borrow lives only as long as the call.
unsafe fn borrow_cstr<'a>(what: &str, p: *const c_char) -> FfiResult<&'a str> {
    if p.is_null() {
        return Err(format!("{what}: NULL pointer"));
    }
    CStr::from_ptr(p)
        .to_str()
        .map_err(|_| format!("{what}: not valid UTF-8"))
}

fn parse_json(what: &str, s: &str) -> FfiResult<Value> {
    serde_json::from_str(s).map_err(|e| format!("{what}: invalid JSON: {e}"))
}

/// Convert a `Result<String>` into a library-owned C string (`NULL` on
/// error, with `last_error` set).
fn return_string(r: FfiResult<String>) -> *mut c_char {
    match r {
        Ok(s) => match CString::new(s) {
            Ok(c) => c.into_raw(),
            Err(_) => {
                set_err("quilt-cabi: result contained an interior NUL");
                null_mut()
            }
        },
        Err(e) => {
            set_err(e);
            null_mut()
        }
    }
}

/// Convert a `Result<()>>` into the `0` / negative-error convention.
fn return_code(r: FfiResult<()>) -> c_int {
    match r {
        Ok(()) => 0,
        Err(e) => {
            set_err(e);
            -1
        }
    }
}

// ---------------------------------------------------------------------------
// Version
// ---------------------------------------------------------------------------

/// The ABI version of this library (see [`QUILT_ABI_VERSION`]).
///
/// Bindings should compare this against the `QUILT_ABI_VERSION` their
/// header was generated from and fail loudly on mismatch — the contract's
/// "never silently guess" rule, applied to the seam itself.
#[no_mangle]
pub extern "C" fn quilt_abi_version() -> u32 {
    QUILT_ABI_VERSION
}

// ---------------------------------------------------------------------------
// Engine lifecycle
// ---------------------------------------------------------------------------

/// Create a fresh engine (empty sheet). Returns an owned handle, or `NULL`
/// on allocation failure. Destroy it with [`quilt_engine_free`].
#[no_mangle]
pub extern "C" fn quilt_engine_new() -> *mut QuiltEngine {
    clear_err();
    let engine = catch_panics(|| {
        Ok(Box::new(QuiltEngine {
            inner: quilt_core::QuiltEngine::new("quilt-cabi"),
        }))
    });
    match engine {
        Ok(boxed) => Box::into_raw(boxed),
        Err(e) => {
            set_err(e);
            null_mut()
        }
    }
}

/// Destroy an engine created by [`quilt_engine_new`]. Tolerates `NULL`.
#[no_mangle]
pub extern "C" fn quilt_engine_free(engine: *mut QuiltEngine) {
    clear_err();
    if !engine.is_null() {
        drop(unsafe { Box::from_raw(engine) });
    }
}

/// Load a YAML sheet into the engine (resets all cell state).
///
/// `yaml` is borrowed for the call only. Returns `0` on success, `-1` on a
/// parse or load error (see [`quilt_last_error`]).
#[no_mangle]
pub extern "C" fn quilt_engine_load_sheet(engine: *mut QuiltEngine, yaml: *const c_char) -> c_int {
    clear_err();
    return_code(catch_panics(|| {
        let engine = unsafe { borrow_engine(engine)? };
        let yaml = unsafe { borrow_cstr("quilt_engine_load_sheet: yaml", yaml)? };
        let sheet = quilt_core::parse_sheet(yaml)
            .map_err(|e| format!("quilt_engine_load_sheet: {e}"))?;
        engine
            .inner
            .load_sheet(sheet)
            .map_err(|e| format!("quilt_engine_load_sheet: {e}"))
    }))
}

/// Borrow the engine behind a handle, checking for NULL.
unsafe fn borrow_engine<'a>(ptr: *mut QuiltEngine) -> FfiResult<&'a QuiltEngine> {
    if ptr.is_null() {
        return Err("engine handle is NULL".to_string());
    }
    Ok(&*ptr)
}

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

/// Read a cell's current value as JSON text (e.g. `"80.0"`, `"true"`,
/// `"\"idle\""`). Evaluates formula cells first. The returned string is
/// library-allocated: free it with [`quilt_string_free`]. `NULL` on error.
#[no_mangle]
pub extern "C" fn quilt_engine_get(engine: *mut QuiltEngine, cell_id: *const c_char) -> *mut c_char {
    clear_err();
    return_string(catch_panics(|| {
        let engine = unsafe { borrow_engine(engine)? };
        let cell_id = unsafe { borrow_cstr("quilt_engine_get: cell_id", cell_id)? };
        let value = engine
            .inner
            .get(cell_id, CallerContext::default())
            .map_err(|e| format!("quilt_engine_get: {e}"))?;
        if let Some(err) = value.error {
            return Err(format!("quilt_engine_get: cell {cell_id} errored: {}", err.message));
        }
        serde_json::to_string(&value.data).map_err(|e| format!("quilt_engine_get: {e}"))
    }))
}

/// Write a cell's value and propagate downstream (Kahn order, dependents
/// recomputed on next read). Works for every cell kind — for sensor/io
/// cells this is exactly a push. `value_json` is any JSON value text.
/// Returns `0` on success, `-1` on error.
#[no_mangle]
pub extern "C" fn quilt_engine_set(
    engine: *mut QuiltEngine,
    cell_id: *const c_char,
    value_json: *const c_char,
) -> c_int {
    clear_err();
    return_code(catch_panics(|| {
        let engine = unsafe { borrow_engine(engine)? };
        let cell_id = unsafe { borrow_cstr("quilt_engine_set: cell_id", cell_id)? };
        let value =
            unsafe { borrow_cstr("quilt_engine_set: value_json", value_json)? };
        let value = parse_json("quilt_engine_set: value_json", value)?;
        engine
            .inner
            .set(cell_id, value, CallerContext::default())
            .map_err(|e| format!("quilt_engine_set: {e}"))
    }))
}

// ---------------------------------------------------------------------------
// The ledger — record / verify / reconcile over a global book of books
// ---------------------------------------------------------------------------

/// Create the ledger for `cell_id` with a genesis state (JSON) committed at
/// `ts_millis`. The genesis is sealed into the chain root, so the very
/// first record scores against the persistence prior and the seals match
/// the golden vectors. Fails (`-1`) if a ledger already exists for the
/// cell — retrofitting a genesis would fork the chain. Returns `0` on
/// success.
#[no_mangle]
pub extern "C" fn quilt_ledger_init(
    cell_id: *const c_char,
    genesis_json: *const c_char,
    ts_millis: u64,
) -> c_int {
    clear_err();
    return_code(catch_panics(|| {
        let cell_id = unsafe { borrow_cstr("quilt_ledger_init: cell_id", cell_id)? };
        let genesis =
            unsafe { borrow_cstr("quilt_ledger_init: genesis_json", genesis_json)? };
        let genesis = parse_json("quilt_ledger_init: genesis_json", genesis)?;
        let mut books = LEDGERS
            .lock()
            .map_err(|_| "quilt_ledger_init: ledger registry poisoned")?;
        if books.contains_key(cell_id) {
            return Err(format!(
                "quilt_ledger_init: ledger for '{cell_id}' already exists (a genesis cannot be retrofitted)"
            ));
        }
        books.insert(cell_id.to_string(), CellLedger::with_genesis(cell_id, genesis, ts_millis));
        Ok(())
    }))
}

/// Record a complete double entry — `input_json` in, `output_json` out, at
/// `ts_millis` — and return the entry's **seal** (64-char lowercase SHA-256
/// hex) as a library-allocated string; free with [`quilt_string_free`].
///
/// If no ledger exists for the cell yet, one is created **without** a
/// genesis, so the first edge is a null-prior edge (no `expected`, no
/// `imbalance` — the ledger never fakes a number). Use [`quilt_ledger_init`]
/// first when the golden chain roots are required. `NULL` on error.
#[no_mangle]
pub extern "C" fn quilt_ledger_record(
    cell_id: *const c_char,
    input_json: *const c_char,
    output_json: *const c_char,
    ts_millis: u64,
) -> *mut c_char {
    clear_err();
    return_string(catch_panics(|| {
        let cell_id = unsafe { borrow_cstr("quilt_ledger_record: cell_id", cell_id)? };
        let input = unsafe { borrow_cstr("quilt_ledger_record: input_json", input_json)? };
        let output = unsafe { borrow_cstr("quilt_ledger_record: output_json", output_json)? };
        let (input, output) = (
            parse_json("quilt_ledger_record: input_json", input)?,
            parse_json("quilt_ledger_record: output_json", output)?,
        );
        let mut books = LEDGERS
            .lock()
            .map_err(|_| "quilt_ledger_record: ledger registry poisoned")?;
        let ledger = books
            .entry(cell_id.to_string())
            .or_insert_with(|| CellLedger::new(cell_id.to_string()));
        Ok(ledger.record(input, output, ts_millis).hash)
    }))
}

/// Recompute every seal and prev-link for the cell's chain.
///
/// Returns `1` if intact, `0` if the chain is broken (tamper evidence),
/// `-1` if no such ledger exists.
#[no_mangle]
pub extern "C" fn quilt_ledger_verify(cell_id: *const c_char) -> c_int {
    clear_err();
    let result = catch_panics(|| {
        let cell_id = unsafe { borrow_cstr("quilt_ledger_verify: cell_id", cell_id)? };
        let books = LEDGERS
            .lock()
            .map_err(|_| "quilt_ledger_verify: ledger registry poisoned")?;
        let ledger = books
            .get(cell_id)
            .ok_or_else(|| format!("quilt_ledger_verify: no ledger for '{cell_id}'"))?;
        Ok(ledger.verify_chain().intact)
    });
    match result {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(e) => {
            set_err(e);
            -1
        }
    }
}

/// Reconcile the books for `cell_id`: matched pairs, open inputs, chain
/// and continuity integrity, total/mean surprise, `balanced`. The returned
/// JSON string matches the `Reconciliation` schema of `docs/cell-ledger.md`
/// (the same fields `compat/golden.json` op (e) pins). Free with
/// [`quilt_string_free`]; `NULL` on error.
#[no_mangle]
pub extern "C" fn quilt_ledger_reconcile(cell_id: *const c_char) -> *mut c_char {
    clear_err();
    return_string(catch_panics(|| {
        let cell_id = unsafe { borrow_cstr("quilt_ledger_reconcile: cell_id", cell_id)? };
        let books = LEDGERS
            .lock()
            .map_err(|_| "quilt_ledger_reconcile: ledger registry poisoned")?;
        let ledger = books
            .get(cell_id)
            .ok_or_else(|| format!("quilt_ledger_reconcile: no ledger for '{cell_id}'"))?;
        serde_json::to_string(&ledger.reconcile())
            .map_err(|e| format!("quilt_ledger_reconcile: {e}"))
    }))
}

/// The chain head for `cell_id`: the last entry's seal, or — for an
/// empty ledger — the genesis commit (identity + genesis + every
/// transaction, in one hash). Free with [`quilt_string_free`].
#[no_mangle]
pub extern "C" fn quilt_ledger_chain_hash(cell_id: *const c_char) -> *mut c_char {
    clear_err();
    return_string(catch_panics(|| {
        let cell_id = unsafe { borrow_cstr("quilt_ledger_chain_hash: cell_id", cell_id)? };
        let books = LEDGERS
            .lock()
            .map_err(|_| "quilt_ledger_chain_hash: ledger registry poisoned")?;
        let ledger = books
            .get(cell_id)
            .ok_or_else(|| format!("quilt_ledger_chain_hash: no ledger for '{cell_id}'"))?;
        Ok(ledger.chain_hash())
    }))
}

/// Drop every ledger in the process-global registry. Mostly for tests and
/// clean shutdowns; engine handles are unaffected. Returns `0`.
#[no_mangle]
pub extern "C" fn quilt_ledgers_reset() -> c_int {
    clear_err();
    let result = catch_panics(|| {
        LEDGERS
            .lock()
            .map(|mut books| books.clear())
            .map_err(|_| "quilt_ledgers_reset: ledger registry poisoned".to_string())
    });
    match result {
        Ok(()) => 0,
        Err(e) => {
            set_err(e);
            -1
        }
    }
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

/// Free a string returned by this library (`quilt_engine_get`,
/// `quilt_ledger_record`, `quilt_ledger_reconcile`,
/// `quilt_ledger_chain_hash`). Do **not** call `free()` — the library may
/// use a different allocator. Tolerates `NULL`. Passing any other pointer
/// is undefined behavior.
#[no_mangle]
pub extern "C" fn quilt_string_free(s: *mut c_char) {
    clear_err();
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

/// The last error message produced on **this thread** by any quilt call,
/// or `""` if the last call succeeded. The pointer is borrowed from
/// thread-local storage: valid until the next quilt call on the same
/// thread; never `NULL`. Copy it if you need it beyond the next call.
#[no_mangle]
pub extern "C" fn quilt_last_error() -> *const c_char {
    LAST_ERROR.with(|slot| slot.borrow().as_ptr() as *const c_char)
}
