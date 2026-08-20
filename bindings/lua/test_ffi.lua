--[[
test_ffi.lua — quilt Lua binding test against compat/golden.json.

Reproduces, THROUGH the C ABI (no engine logic here), the golden ops:
  (a) value cell read — exact JSON text equality
  (b) formula eval, initial + post-push (reactive propagation)
  (d) edge delta/imbalance — surfaces through the ledger: the transcript's
      per-edge imbalances (45.0, 2.5, 0.0) sum into total_surprise == 47.5,
      and a null-prior first edge (no genesis) seals and reconciles balanced
  (e) ledger record/verify/reconcile — seals and chain_hash bit-for-bit

Golden constants mirror crates/quilt-cabi/smoke/golden_vectors.h, which is
generated from compat/golden.json.

Run from the repo root:
  luajit bindings/lua/test_ffi.lua   (preferred, no shim needed)
  lua    bindings/lua/test_ffi.lua   (plain Lua 5.1; needs quilt_shim.so —
                                      see build line in quilt_shim.c)
Exit code 0 on PASS, 1 on FAIL.
]]

local dir = (arg and arg[0] or "test_ffi.lua"):match("^(.*)/[^/]*$") or "."
package.path = dir .. "/?.lua;" .. package.path

local quilt = require("quilt_ffi").load()

-- The golden sheet, YAML form (from smoke/golden_vectors.h).
local SHEET_YAML = [=[
id: "bilge-reflex"
description: "The golden sheet: a sensor, a threshold, two formulas, a status value. Every cell id, kind, and dependency edge below is part of the contract."
cells:
  - id: "bilge.level"
    kind: "sensor"
    source: "simulated"
    default: 40.0
  - id: "bilge.threshold"
    kind: "value"
    value: 80.0
  - id: "pump.should_run"
    kind: "formula"
    expr: "=bilge.level >= bilge.threshold"
  - id: "pump.relay_cmd"
    kind: "formula"
    expr: "=clamp((bilge.level - bilge.threshold) * 0.5, -30.0, 30.0)"
  - id: "status"
    kind: "value"
    value: "idle"
]=]

-- op_e_chain: per-entry seals and the genesis root, bit-for-bit.
local G_ENTRY1_PREV = "470bc52774d2c173c46c5b0a8733676500fbc3b0b0cbf6d3e57662ff2ac4c1d3"
local G_ENTRY1 = "00b88886f4a22747e7e546144d18af42605c34e2ba07d3040ba18d6f24ce3f30"
local G_ENTRY2 = "3f3a94eb2e88c0e3bddfc8dfcb5554af2fe7b5cd80e20ac0ff972ae7a31caa3d"
local G_ENTRY3 = "4a7ad64830d5c2a843bf7b6f4d7253d43f2603753611fe58f0bb7c4d87ea62b8"
local G_CHAIN_HASH = "4a7ad64830d5c2a843bf7b6f4d7253d43f2603753611fe58f0bb7c4d87ea62b8"

-- op_e reconcile: substrings the JSON report must contain.
local G_RECONCILE_NEEDLES = {
    '"cell_id":"bilge.level"',
    '"entries":3',
    '"open_inputs":0',
    '"matched_pairs":3',
    '"chain_intact":true',
    '"continuity_intact":true',
    '"balanced":true',
    '"total_surprise":47.5', -- 45.0 + 2.5 + 0.0: the per-edge imbalances
    '"mean_surprise":15.833333333333334',
}

local checks, failures = 0, 0
local function check(cond, msg)
    checks = checks + 1
    if cond then
        print("  PASS " .. msg)
    else
        failures = failures + 1
        print("  FAIL " .. msg)
    end
end

-- Read a cell and compare its JSON text exactly.
local function get_is(e, cell, want)
    local got = quilt.engine_get(e, cell)
    if got == nil then
        print(("    get(%s) returned nil: %s"):format(cell, quilt.last_error()))
        return false
    end
    if got ~= want then
        print(("    get(%s): got %q, want %q"):format(cell, got, want))
        return false
    end
    return true
end

local function record(cell, i, o, ts)
    local seal = quilt.ledger_record(cell, i, o, ts)
    if seal == nil then
        print("    ledger_record failed: " .. quilt.last_error())
    end
    return seal
end

print(("=== quilt Lua FFI test (backend: %s) ==="):format(quilt.backend or "?"))
check(quilt.abi_version() == 1, "ABI version matches quilt_cabi.h")

quilt.ledgers_reset()

-- engine + golden sheet ------------------------------------------------------

local e = quilt.engine_new()
check(e ~= nil, "engine_new")
check(quilt.engine_load_sheet(e, SHEET_YAML) == 0, "load_sheet (golden YAML)")

-- op (a): value cell read — exact JSON equality -------------------------------

check(get_is(e, "bilge.threshold", "80.0"), "(a) read bilge.threshold == 80.0")
check(get_is(e, "status", '"idle"'), "(a) read status == \"idle\"")
check(get_is(e, "bilge.level", "40.0"), "(a) read bilge.level == 40.0")

-- op (b): formula eval, initial + post-push -----------------------------------

check(get_is(e, "pump.should_run", "false"), "(b) initial should_run == false")
check(get_is(e, "pump.relay_cmd", "-20.0"), "(b) initial relay_cmd == -20.0")
check(quilt.engine_set(e, "bilge.level", "85.0") == 0, "(b) push level=85.0")
check(get_is(e, "bilge.level", "85.0"), "(b) post level == 85.0")
check(get_is(e, "pump.should_run", "true"), "(b) post should_run == true")
check(get_is(e, "pump.relay_cmd", "2.5"), "(b) post relay_cmd == 2.5")

-- op (e): ledger record / verify / reconcile, seals bit-for-bit ---------------

check(quilt.ledger_init("bilge.level", "40.0", 1000) == 0,
      "(e) ledger_init genesis 40.0 @1000")
check(quilt.ledger_init("bilge.level", "40.0", 1000) == -1,
      "(e) double ledger_init is rejected")

local root = quilt.ledger_chain_hash("bilge.level")
check(root ~= nil and root == G_ENTRY1_PREV, "(e) genesis root pinned")

local s1 = record("bilge.level", "85.0", "85.0", 2000)
local s2 = record("bilge.level", "87.5", "87.5", 3000)
local s3 = record("bilge.level", "87.5", "87.5", 4000)
check(s1 ~= nil and s1 == G_ENTRY1, "(e) seal 1 bit-for-bit")
check(s2 ~= nil and s2 == G_ENTRY2, "(e) seal 2 bit-for-bit")
check(s3 ~= nil and s3 == G_ENTRY3, "(e) seal 3 bit-for-bit")

check(quilt.ledger_verify("bilge.level") == 1, "(e) chain verifies (1)")
check(quilt.ledger_verify("no.such.cell") == -1, "(e) unknown ledger -> -1")

local head = quilt.ledger_chain_hash("bilge.level")
check(head ~= nil and head == G_CHAIN_HASH, "(e) chain_hash == golden head")

local report = quilt.ledger_reconcile("bilge.level")
local rec_ok = report ~= nil
if rec_ok then
    for _, needle in ipairs(G_RECONCILE_NEEDLES) do
        if not report:find(needle, 1, true) then
            print("    reconcile missing " .. needle .. "\n    got: " .. report)
            rec_ok = false
        end
    end
end
check(rec_ok, "(e) reconcile report matches golden (balanced, surprise 47.5)")

-- op (d): edge delta/imbalance through the ABI --------------------------------
-- A ledger with no genesis: the first edge is a null-prior edge — no faked
-- delta/imbalance — and the books still reconcile balanced.
local fs = record("fresh.sensor", "7.0", "7.0", 5000)
check(fs ~= nil and #fs == 64 and fs:match("^[0-9a-f]+$") == fs,
      "(d) null-prior first edge sealed (64 lowercase hex)")
local frep = quilt.ledger_reconcile("fresh.sensor")
check(frep ~= nil
      and frep:find('"entries":1', 1, true) ~= nil
      and frep:find('"balanced":true', 1, true) ~= nil,
      "(d) null-prior edge reconciles balanced")

-- error discipline ------------------------------------------------------------

check(quilt.engine_get(e, "no.such.cell") == nil, "unknown cell returns nil")
check(#quilt.last_error() > 0, "last_error explains the failure")
check(quilt.engine_get(nil, "x") == nil, "nil engine tolerated")
check(quilt.ledger_record("x.cell", "{not json", "1", 1) == nil,
      "bad JSON input returns nil")

quilt.engine_free(e)

print("chain_hash: " .. tostring(head))
print(("RESULT: %s — %d checks, %d failures"):format(
    failures == 0 and "PASS" or "FAIL", checks, failures))
os.exit(failures == 0 and 0 or 1)
