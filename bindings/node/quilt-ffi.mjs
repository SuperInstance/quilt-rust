// quilt-ffi.mjs — a thin koffi binding over the quilt C ABI
// (crates/quilt-cabi/quilt_cabi.h) for Node.js: the web/connecting
// tier's runtime binding, completing the FFI set (Python/Go/Julia/R/Lua).
//
// One native core, N bindings: this module is pure passthrough. It never
// reimplements the engine or the ledger — every semantic (formula eval,
// reactive propagation, seal computation, reconciliation) happens inside
// libquilt_cabi.so and is only marshalled across the ABI.
//
// WHY THIS PATH (vs the WASM artifact):
//   The wasm crate deliberately exports only two C-ABI anchors —
//   quilt_core_wasm_abi_version() and quilt_core_wasm_golden_check() —
//   a sealed 0/1 self-check; its real JS surface is future wasm-bindgen
//   work (see crates/quilt-core-wasm/src/lib.rs). The C ABI, by design
//   ("any runtime ... Node FFI ... binds through these symbols"), is the
//   only path that exposes the full engine+ledger surface needed to
//   reproduce each golden op with observable outputs: values, seals, the
//   chain_hash string, reconcile reports. The .wasm is still exercised
//   as a cross-check in test.mjs via Node's built-in WebAssembly.
//
// koffi is the single third-party dependency (Node has no built-in FFI).
//
// MEMORY CONTRACT honored here:
//   - borrowed `const char *` arguments: koffi encodes JS strings to
//     NUL-terminated UTF-8 for the duration of the call only;
//   - returned `char *` strings: every string-returning symbol is bound
//     with a `void *` prototype, read byte-wise (UTF-8), then released
//     with quilt_string_free() in the same tick — never free(), and
//     never a second call of a side-effecting symbol just to re-read;
//   - engine handles are opaque and caller-owned (use free(), or the
//     engineIn() helper for deterministic teardown).
//
// KNOWN NATIVE QUIRK (guarded, see README.md): quilt_last_error()
// returns String::new().as_ptr() — a dangling non-null pointer (1) —
// when no error was ever set on the thread, despite the header's
// "never NULL / returns \"\"" promise. lastError() below treats
// pointers 0 and 1 as "" and never dereferences them.
//
// Library resolution order:
//   1. $QUILT_CABI_SO (explicit override);
//   2. target/release/libquilt_cabi.so found by walking up from this
//      file to the repository root.

import koffi from 'koffi';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const ABI_VERSION = 1; // must match QUILT_ABI_VERSION in quilt_cabi.h

const MAX_STR = 1 << 20; // 1 MiB: every string this ABI returns is tiny

export class QuiltError extends Error {} // message carries quilt_last_error()

// -- library discovery ---------------------------------------------------------

function findLibrary() {
  const env = process.env.QUILT_CABI_SO;
  if (env) {
    if (!existsSync(env)) throw new Error(`QUILT_CABI_SO=${env} does not exist`);
    return { repoRoot: resolve(env, '../../..'), path: resolve(env) };
  }
  let dir = dirname(fileURLToPath(import.meta.url));
  for (;;) {
    const so = join(dir, 'target/release/libquilt_cabi.so');
    if (existsSync(so)) return { repoRoot: dir, path: so };
    const parent = dirname(dir);
    if (parent === dir) throw new Error('libquilt_cabi.so not found (walk-up from bindings/node); build it or set $QUILT_CABI_SO');
    dir = parent;
  }
}

export const { repoRoot: REPO_ROOT, path: LIBRARY_PATH } = findLibrary();
const lib = koffi.load(LIBRARY_PATH);

// -- symbol binding --------------------------------------------------------------

const QuiltEngine = koffi.opaque('QuiltEngine');

// char *-returning symbols are bound as void * so the pointer survives
// for quilt_string_free() after the bytes are copied out (see header docs).
const sym = {
  abiVersion:       lib.func('uint32_t quilt_abi_version()'),
  engineNew:        lib.func('QuiltEngine *quilt_engine_new()'),
  engineLoadSheet:  lib.func('int quilt_engine_load_sheet(QuiltEngine *engine, const char *yaml)'),
  engineGet:        lib.func('void *quilt_engine_get(QuiltEngine *engine, const char *cell_id)'),
  engineSet:        lib.func('int quilt_engine_set(QuiltEngine *engine, const char *cell_id, const char *value_json)'),
  engineFree:       lib.func('void quilt_engine_free(QuiltEngine *engine)'),
  ledgerInit:       lib.func('int quilt_ledger_init(const char *cell_id, const char *genesis_json, uint64_t ts_millis)'),
  ledgerRecord:     lib.func('void *quilt_ledger_record(const char *cell_id, const char *input_json, const char *output_json, uint64_t ts_millis)'),
  ledgerVerify:     lib.func('int quilt_ledger_verify(const char *cell_id)'),
  ledgerReconcile:  lib.func('void *quilt_ledger_reconcile(const char *cell_id)'),
  ledgerChainHash:  lib.func('void *quilt_ledger_chain_hash(const char *cell_id)'),
  ledgersReset:     lib.func('int quilt_ledgers_reset()'),
  stringFree:       lib.func('void quilt_string_free(void *s)'),
  lastErrorPtr:     lib.func('void *quilt_last_error()'),
};

// -- string plumbing --------------------------------------------------------------

/** Read a NUL-terminated UTF-8 string from native memory at `ptr`. */
function readCString(ptr) {
  const bytes = [];
  for (let off = 0; off < MAX_STR; off++) {
    const b = koffi.decode(ptr, off, 'char');
    if (b === 0) return Buffer.from(bytes).toString('utf8');
    bytes.push(b & 0xff);
  }
  throw new QuiltError('unterminated string from quilt ABI');
}

/**
 * Call a char*-returning symbol once, copy the string out, and release
 * the allocation with quilt_string_free() — all inside the same tick.
 * NULL return raises QuiltError with the thread's last_error detail.
 */
function takeString(call) {
  const p = call();
  if (p === null || p === 0n) throw new QuiltError(lastError());
  const s = readCString(p);
  sym.stringFree(p);
  return s;
}

/**
 * The thread's last error detail (borrowed; copied here). Guards the
 * empty-String dangling pointer: see the native quirk note up top.
 */
export function lastError() {
  const p = sym.lastErrorPtr();
  if (p === null || p === 0n || p === 1n) return '';
  return readCString(p); // borrowed — never freed
}

// -- version ------------------------------------------------------------------------

export function abiVersion() {
  return sym.abiVersion();
}

// -- engine --------------------------------------------------------------------------

export class Engine {
  #handle;

  constructor() {
    this.#handle = sym.engineNew();
    if (this.#handle === null || this.#handle === 0n) {
      throw new QuiltError(lastError() || 'quilt_engine_new returned NULL');
    }
  }

  /** Load a YAML sheet (resets all cell state). Throws on error. */
  loadSheet(yaml) {
    this.#guard();
    const rc = sym.engineLoadSheet(this.#handle, yaml);
    if (rc !== 0) throw new QuiltError(lastError());
    return this;
  }

  /** Read a cell's current value as canonical JSON text ("80.0", "true", "\"idle\""). */
  get(cellId) {
    this.#guard();
    return takeString(() => sym.engineGet(this.#handle, cellId));
  }

  /** Write a cell's value (any JSON text) and propagate downstream. */
  set(cellId, valueJson) {
    this.#guard();
    const rc = sym.engineSet(this.#handle, cellId, valueJson);
    if (rc !== 0) throw new QuiltError(lastError());
  }

  /** Destroy the engine (idempotent; tolerates NULL natively too). */
  free() {
    if (this.#handle !== null) {
      sym.engineFree(this.#handle);
      this.#handle = null;
    }
  }

  #guard() {
    if (this.#handle === null) throw new QuiltError('engine handle already freed');
  }
}

/** `const e = engineIn(yaml); try { ... } finally { e.free(); }` helper. */
export function engineIn(yaml) {
  const e = new Engine().loadSheet(yaml);
  return e;
}

// -- ledger (process-global registry keyed by cell id) --------------------------------

export function ledgerInit(cellId, genesisJson, tsMillis) {
  const rc = sym.ledgerInit(cellId, genesisJson, BigInt(tsMillis));
  if (rc !== 0) throw new QuiltError(lastError());
}

/** Record a complete double entry; returns the entry's seal (64 lowercase hex). */
export function ledgerRecord(cellId, inputJson, outputJson, tsMillis) {
  return takeString(() => sym.ledgerRecord(cellId, inputJson, outputJson, BigInt(tsMillis)));
}

/** 1 intact, 0 broken, -1 no such ledger. */
export function ledgerVerify(cellId) {
  return sym.ledgerVerify(cellId);
}

/** Reconciliation report (parsed JSON object). */
export function ledgerReconcile(cellId) {
  return JSON.parse(takeString(() => sym.ledgerReconcile(cellId)));
}

/** The chain head seal (genesis commit for an empty ledger). */
export function ledgerChainHash(cellId) {
  return takeString(() => sym.ledgerChainHash(cellId));
}

export function ledgersReset() {
  return sym.ledgersReset();
}

// -- low level (error-discipline checks reach for these) -----------------------------

/** Bound symbol table, for tests that poke the ABI directly. */
export const symbols = sym;
