#!/usr/bin/env node
// test.mjs — golden-vector conformance for the Node.js binding.
//
// Reproduces compat/golden.json ops (a)-(e) THROUGH the C ABI against
// libquilt_cabi.so — the Node tier's proof for "one native core, N
// bindings". All constants are read from compat/golden.json (never
// hand-copied); semantics mirror bindings/python-cabi/test_ffi.py and
// compat/conformance_test.rs:
//
//   (a) value cell read, tolerance 0.0 (exact canonical JSON text)
//   (b) formula eval, initial + reactive post-push, tolerance 1e-12
//   (c) implied by (b): the set() that propagates downstream
//   (d) edge delta/imbalance/provenance (harness-side wire math, exactly
//       like the reference tier; scalar imbalance is additionally
//       cross-checked against the native ledger's reconcile via the ABI)
//   (e) ledger chain: seals + chain_hash BIT-FOR-BIT, reconcile fields
//   (w) bonus: the web-tier quilt_core_wasm.wasm artifact is instantiated
//       with Node's built-in WebAssembly and must self-verify (golden_check
//       → 1) under V8 — same golden contract, second artifact.
//
// Number-literal fidelity: the golden file is ryu-canonical, so number
// literals are preserved verbatim by the parser below (JNum). JS number
// formatting happens to be shortest-round-trip too, but JSON.parse loses
// 40.0 → "40", which would break every seal. Literals in, literals out.
//
// Run from anywhere:  node bindings/node/test.mjs   (or: npm test)

import { createHash } from 'node:crypto';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import * as q from './quilt-ffi.mjs';

const TOL = 1e-12;

let failures = 0;
let checks = 0;

function check(cond, msg) {
  checks += 1;
  console.log(`  ${cond ? 'PASS' : 'FAIL'} ${msg}`);
  if (!cond) failures += 1;
}

// -- golden JSON with preserved number literals -----------------------------------

class JNum {
  constructor(text) {
    this.text = text; // the literal as written (already ryu-canonical)
    this.value = Number(text);
  }
}

/** Minimal recursive-descent JSON parser that keeps number literals. */
function parsePreserving(text) {
  let i = 0;
  const n = text.length;
  const ws = () => { while (i < n && ' \t\r\n'.includes(text[i])) i += 1; };
  const expect = (word) => {
    if (!text.startsWith(word, i)) throw new Error(`golden.json: expected ${word} at ${i}`);
    i += word.length;
  };
  function value() {
    ws();
    const c = text[i];
    if (c === '{') return object();
    if (c === '[') return array();
    if (c === '"') return string();
    if (c === 't') { expect('true'); return true; }
    if (c === 'f') { expect('false'); return false; }
    if (c === 'n') { expect('null'); return null; }
    return number();
  }
  function number() {
    const start = i;
    if (text[i] === '-') i += 1;
    while (i < n && text[i] >= '0' && text[i] <= '9') i += 1;
    if (text[i] === '.') { i += 1; while (i < n && text[i] >= '0' && text[i] <= '9') i += 1; }
    if (text[i] === 'e' || text[i] === 'E') {
      i += 1; if (text[i] === '+' || text[i] === '-') i += 1;
      while (i < n && text[i] >= '0' && text[i] <= '9') i += 1;
    }
    if (i === start) throw new Error(`golden.json: bad number at ${start}`);
    return new JNum(text.slice(start, i));
  }
  function string() {
    const start = i;
    i += 1; // opening quote
    while (i < n) {
      if (text[i] === '\\') { i += 2; continue; }
      if (text[i] === '"') { i += 1; return JSON.parse(text.slice(start, i)); }
      i += 1;
    }
    throw new Error('golden.json: unterminated string');
  }
  function array() {
    i += 1; ws();
    const out = [];
    if (text[i] === ']') { i += 1; return out; }
    for (;;) {
      out.push(value()); ws();
      if (text[i] === ',') { i += 1; continue; }
      if (text[i] === ']') { i += 1; return out; }
      throw new Error(`golden.json: expected , or ] at ${i}`);
    }
  }
  function object() {
    i += 1; ws();
    const out = {};
    if (text[i] === '}') { i += 1; return out; }
    for (;;) {
      ws();
      const k = string(); ws();
      if (text[i] !== ':') throw new Error(`golden.json: expected : at ${i}`);
      i += 1;
      out[k] = value(); ws();
      if (text[i] === ',') { i += 1; continue; }
      if (text[i] === '}') { i += 1; return out; }
      throw new Error(`golden.json: expected , or } at ${i}`);
    }
  }
  const v = value(); ws();
  if (i !== n) throw new Error('golden.json: trailing content');
  return v;
}

/** Canonical JSON text, golden spec form: compact, keys sorted, literals verbatim. */
function canon(v) {
  if (v instanceof JNum) return v.text;
  if (v === null) return 'null';
  if (typeof v === 'boolean') return v ? 'true' : 'false';
  if (typeof v === 'string') return JSON.stringify(v);
  if (typeof v === 'number') return String(v); // harness-side integers only
  if (Array.isArray(v)) return `[${v.map(canon).join(',')}]`;
  const keys = Object.keys(v).sort(); // ASCII keys: byte-order == default sort
  return `{${keys.map((k) => `${JSON.stringify(k)}:${canon(v[k])}`).join(',')}}`;
}

function unwrap(v) {
  if (v instanceof JNum) return v.value;
  if (Array.isArray(v)) return v.map(unwrap);
  if (v !== null && typeof v === 'object') {
    return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, unwrap(x)]));
  }
  return v;
}

function close(got, want, tol) {
  got = unwrap(got); want = unwrap(want);
  if (typeof got === 'boolean' || typeof want === 'boolean') return got === want;
  if (typeof got === 'number' && typeof want === 'number') return Math.abs(got - want) <= tol;
  if (Array.isArray(got) && Array.isArray(want)) {
    return got.length === want.length && got.every((g, idx) => close(g, want[idx], tol));
  }
  return got === want;
}

const isNum = (v) => v instanceof JNum || typeof v === 'number';

// -- the conformance run ------------------------------------------------------------

const goldenPath = join(q.REPO_ROOT, 'compat', 'golden.json');
const g = parsePreserving(readFileSync(goldenPath, 'utf8'));

console.log('=== quilt node-cabi conformance (koffi over libquilt_cabi.so) ===');
console.log(`library: ${q.LIBRARY_PATH}`);
console.log(`node: ${process.version}  koffi FFI, contract: ${g.contract}  golden: compat/golden.json`);

check(g.contract === 'quilt-compat/1', 'golden contract is quilt-compat/1');
check(q.abiVersion() === q.ABI_VERSION, `ABI version matches quilt_cabi.h (${q.ABI_VERSION})`);
q.ledgersReset();

const sheet = readFileSync(join(q.REPO_ROOT, 'crates', 'quilt-cabi', 'smoke', 'sheet.yaml'), 'utf8');

// -- op (a): value cell read — exact canonical JSON text ---------------------------

{
  const e = q.engineIn(sheet);
  try {
    for (const v of g.op_a_value_read) {
      const raw = e.get(v.cell);
      const ok = raw === canon(v.expect);
      if (!ok) console.log(`    get(${v.cell}): got ${JSON.stringify(raw)}, want ${canon(v.expect)}`);
      check(ok, `(a) read ${v.cell} == ${canon(v.expect)}`);
    }

    // -- op (b): formula eval, initial + reactive post-push -----------------------

    for (const v of g.op_b_formula_eval.initial) {
      const got = JSON.parse(e.get(v.cell));
      check(close(got, v.expect, TOL), `(b) initial ${v.cell} == ${canon(v.expect)}`);
    }
    const push = g.op_b_formula_eval.after_push;
    e.set(push.cell, canon(push.value));
    for (const v of g.op_b_formula_eval.post) {
      const got = JSON.parse(e.get(v.cell));
      check(close(got, v.expect, TOL), `(b) post ${v.cell} == ${canon(v.expect)} (reactive after set)`);
    }
    // op (c) is implied by (b): the set() above is the propagation.
  } finally {
    e.free();
  }
}

// -- op (d): edge delta / imbalance / provenance -----------------------------------
// Wire math is harness-side, mirroring conformance_test.rs (the ledger's
// internal surprise metric is mean-of-abs; the wire imbalance for vectors
// is Euclidean — both tiers compute op (d) outside the chain).

for (const v of g.op_d_edge) {
  const { name, before, after, inputs } = v;
  const exp = v.expect;

  let delta; let imbalance;
  if (isNum(before) && isNum(after)) {
    delta = unwrap(after) - unwrap(before);
    imbalance = Math.abs(unwrap(after) - unwrap(before));
  } else if (Array.isArray(before) && Array.isArray(after) && before.length === after.length) {
    const ds = after.map((a, idx) => unwrap(a) - unwrap(before[idx]));
    delta = ds;
    imbalance = Math.sqrt(ds.reduce((s, d) => s + d * d, 0));
  } else {
    delta = null;
    imbalance = null;
  }

  const provenance = createHash('sha256').update(canon(inputs), 'utf8').digest('hex');

  check(close(delta, exp.delta, TOL), `(d) ${name} delta`);
  check(close(imbalance, exp.imbalance, TOL), `(d) ${name} imbalance`);
  check(provenance === exp.provenance, `(d) ${name} provenance`);

  if (exp.imbalance === null) {
    // null-prior edge through the ABI: record with NO genesis so the first
    // edge is a null-prior edge — no surprise is claimed.
    q.ledgersReset();
    const seal = q.ledgerRecord(v.cell, canon(after), canon(after), Math.trunc(v.ts.value));
    const rep = q.ledgerReconcile(v.cell);
    check(
      seal.length === 64 && rep.entries === 1 && rep.total_surprise === 0.0 && rep.balanced,
      `(d) ${name} null-prior edge: no surprise via ABI`,
    );
  } else if (!Array.isArray(before)) {
    // Scalar edge through the ABI: genesis commits `before`, the record
    // commits `after`; reconcile's total_surprise IS the wire imbalance
    // under the persistence prior.
    q.ledgersReset();
    q.ledgerInit(v.cell, canon(before), Math.trunc(v.ts.value) - 1);
    q.ledgerRecord(v.cell, canon(after), canon(after), Math.trunc(v.ts.value));
    const rep = q.ledgerReconcile(v.cell);
    check(close(rep.total_surprise, exp.imbalance, TOL), `(d) ${name} imbalance cross-checked via ABI reconcile`);
  }
}

// -- op (e): ledger chain — seals BIT-FOR-BIT, reconcile ----------------------------

q.ledgersReset();
const tr = g.op_e_chain.transcript;
const cell = tr.cell;
q.ledgerInit(cell, canon(tr.genesis), Math.trunc(tr.genesis_ts.value));

const root = q.ledgerChainHash(cell);
check(root === g.op_e_chain.entries[0].prev_hash, '(e) genesis root pinned (entry 1 prev-link)');

let doubleInitRejected = false;
try {
  q.ledgerInit(cell, canon(tr.genesis), Math.trunc(tr.genesis_ts.value));
} catch (ex) {
  doubleInitRejected = ex instanceof q.QuiltError;
}
check(doubleInitRejected, '(e) double ledger_init is rejected');

tr.records.forEach((rec, idx) => {
  const want = g.op_e_chain.entries[idx];
  const seal = q.ledgerRecord(cell, canon(rec.input), canon(rec.output), Math.trunc(rec.ts.value));
  check(seal === want.hash, `(e) seal ${want.seq.value} bit-for-bit`);
});

check(q.ledgerVerify(cell) === 1, '(e) chain verifies (1)');
check(q.ledgerVerify('no.such.cell') === -1, '(e) unknown ledger -> -1');

const head = q.ledgerChainHash(cell);
const chainHash = g.op_e_chain.chain_hash;
check(head === chainHash, '(e) chain_hash == golden head (bit-for-bit)');

const rep = q.ledgerReconcile(cell);
const wr = unwrap(g.op_e_chain.reconcile);
const recOk = rep.cell_id === cell
  && rep.entries === wr.entries
  && rep.open_inputs === wr.open_inputs
  && rep.matched_pairs === wr.matched_pairs
  && rep.chain_intact === wr.chain_intact
  && rep.continuity_intact === wr.continuity_intact
  && rep.balanced === wr.balanced
  && close(rep.total_surprise, wr.total_surprise, TOL)
  && close(rep.mean_surprise, wr.mean_surprise, TOL);
if (!recOk) console.log(`    reconcile got: ${JSON.stringify(rep)}`);
check(recOk, `(e) reconcile matches golden (balanced, total ${wr.total_surprise}, mean ${wr.mean_surprise})`);

// -- error discipline -----------------------------------------------------------------

{
  const e = q.engineIn(sheet);
  try {
    let unknownOk = false;
    try {
      e.get('no.such.cell');
    } catch (ex) {
      unknownOk = ex instanceof q.QuiltError && ex.message.length > 0 && q.lastError().length > 0;
    }
    check(unknownOk, 'unknown cell errors with a last_error detail');

    // NULL engine tolerated natively: NULL in, NULL out + last_error set.
    const r = q.symbols.engineGet(null, 'x');
    check((r === null || r === 0n) && q.lastError().length > 0, 'NULL engine tolerated (NULL out, error set)');

    let badJsonOk = false;
    try {
      q.ledgerRecord('x.cell', '{not json', '1', 1);
    } catch (ex) {
      badJsonOk = ex instanceof q.QuiltError;
    }
    check(badJsonOk, 'bad JSON input to ledger_record errors');

    q.symbols.stringFree(null); // must be a no-op
    check(true, 'string_free(NULL) is a no-op');
  } finally {
    e.free();
  }
}

// -- bonus (w): the wasm artifact self-verifies under V8 -------------------------------

const wasmPath = join(q.REPO_ROOT, 'target', 'wasm32-unknown-unknown', 'release', 'quilt_core_wasm.wasm');
if (existsSync(wasmPath)) {
  const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), {});
  const { quilt_core_wasm_abi_version: abi, quilt_core_wasm_golden_check: goldenCheck } = instance.exports;
  check(abi() === 1, '(w) wasm ABI anchor == 1');
  check(goldenCheck() === 1, '(w) quilt_core_wasm golden_check -> 1 inside V8 (web tier agrees)');
} else {
  console.log('  SKIP (w) quilt_core_wasm.wasm not built (cargo build -p quilt-core-wasm --release --target wasm32-unknown-unknown)');
}

// -- summary ----------------------------------------------------------------------------

q.ledgersReset();
const result = failures === 0 ? 'PASS' : 'FAIL';
console.log(`RESULT: ${result} — ${checks} checks, ${failures} failures`);
console.log(`chain_hash: ${head}`);
process.exitCode = failures === 0 ? 0 : 1;
