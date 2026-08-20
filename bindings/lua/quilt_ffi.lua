--[[
quilt_ffi.lua — thin Lua binding over the quilt C ABI (libquilt_cabi.so).

LuaJIT (preferred): binds every symbol directly via ffi.cdef/ffi.load.
Plain Lua 5.1/5.4:  falls back to the companion C shim (quilt_shim.c,
built as quilt_shim.so next to this file). Both paths expose the same
table of functions:

    local quilt = require("quilt_ffi").load()

    quilt.abi_version()                        -> number
    quilt.engine_new()                         -> handle | nil
    quilt.engine_free(h)
    quilt.engine_load_sheet(h, yaml)           -> 0 | -1
    quilt.engine_get(h, cell)                  -> string | nil
    quilt.engine_set(h, cell, value_json)      -> 0 | -1
    quilt.ledger_init(cell, genesis_json, ts)  -> 0 | -1
    quilt.ledger_record(cell, in_json, out_json, ts) -> seal-hex | nil
    quilt.ledger_verify(cell)                  -> 1 | 0 | -1
    quilt.ledger_reconcile(cell)               -> json | nil
    quilt.ledger_chain_hash(cell)              -> hex | nil
    quilt.ledgers_reset()                      -> 0
    quilt.last_error()                         -> string

Memory contract (crates/quilt-cabi/quilt_cabi.h): strings returned by the
library are copied into Lua strings and released with quilt_string_free()
inside the wrapper — callers never touch raw pointers. Engine handles are
caller-owned; call engine_free explicitly.

This binding contains no engine logic: it forwards to the ABI, per the
compat contract (docs/quilt-compat-contract.md §5).
]]

local M = {}

local QUILT_ABI_VERSION = 1 -- QUILT_ABI_VERSION in quilt_cabi.h

local function script_dir()
    local src = debug.getinfo(1, "S").source
    if src:sub(1, 1) == "@" then
        return src:sub(2):match("^(.*)/[^/]*$") or "."
    end
    return "."
end

local DIR = script_dir()

local LIB_CANDIDATES = {
    DIR .. "/../../target/release/libquilt_cabi.so",
    DIR .. "/../../target/debug/libquilt_cabi.so",
    "libquilt_cabi.so", -- system search path as last resort
}

--[[ ------------------------------------------------------------------ LuaJIT ]]

local function load_luajit()
    local ffi = require("ffi")
    ffi.cdef [[
        typedef struct QuiltEngine QuiltEngine;
        uint32_t quilt_abi_version(void);
        QuiltEngine *quilt_engine_new(void);
        int quilt_engine_load_sheet(QuiltEngine *engine, const char *yaml);
        char *quilt_engine_get(QuiltEngine *engine, const char *cell_id);
        int quilt_engine_set(QuiltEngine *engine, const char *cell_id,
                             const char *value_json);
        void quilt_engine_free(QuiltEngine *engine);
        int quilt_ledger_init(const char *cell_id, const char *genesis_json,
                              uint64_t ts_millis);
        char *quilt_ledger_record(const char *cell_id, const char *input_json,
                                  const char *output_json, uint64_t ts_millis);
        int quilt_ledger_verify(const char *cell_id);
        char *quilt_ledger_reconcile(const char *cell_id);
        char *quilt_ledger_chain_hash(const char *cell_id);
        int quilt_ledgers_reset(void);
        void quilt_string_free(char *s);
        const char *quilt_last_error(void);
    ]]

    local lib
    for _, path in ipairs(LIB_CANDIDATES) do
        local ok, l = pcall(ffi.load, path)
        if ok then
            lib = l
            break
        end
    end
    if not lib then
        error("quilt_ffi: cannot load libquilt_cabi.so from any of: "
            .. table.concat(LIB_CANDIDATES, ", "))
    end

    -- Copy a library-owned char* into a Lua string and free it; nil passes through.
    local function take(s)
        if s == nil then
            return nil
        end
        local out = ffi.string(s)
        lib.quilt_string_free(s)
        return out
    end

    return {
        backend = "luajit-ffi",
        abi_version = function() return lib.quilt_abi_version() end,
        engine_new = lib.quilt_engine_new,
        engine_free = lib.quilt_engine_free,
        engine_load_sheet = lib.quilt_engine_load_sheet,
        engine_get = function(h, cell) return take(lib.quilt_engine_get(h, cell)) end,
        engine_set = lib.quilt_engine_set,
        ledger_init = lib.quilt_ledger_init,
        ledger_record = function(cell, i, o, ts)
            return take(lib.quilt_ledger_record(cell, i, o, ts))
        end,
        ledger_verify = lib.quilt_ledger_verify,
        ledger_reconcile = function(cell) return take(lib.quilt_ledger_reconcile(cell)) end,
        ledger_chain_hash = function(cell) return take(lib.quilt_ledger_chain_hash(cell)) end,
        ledgers_reset = lib.quilt_ledgers_reset,
        last_error = function() return ffi.string(lib.quilt_last_error()) end,
    }
end

--[[ --------------------------------------------------------- plain Lua (shim) ]]

local function load_shim()
    package.cpath = DIR .. "/?.so;" .. package.cpath
    local shim = require("quilt_shim")
    shim.backend = "c-shim"
    return shim
end

--[[ ------------------------------------------------------------------- loader ]]

function M.load()
    local quilt
    if type(jit) == "table" then
        local ok, res = pcall(load_luajit)
        if ok then
            quilt = res
        end
    end
    if not quilt then
        quilt = load_shim()
    end
    if quilt.abi_version() ~= QUILT_ABI_VERSION then
        error(string.format("quilt_ffi: ABI mismatch — header says %d, library says %d",
                            QUILT_ABI_VERSION, quilt.abi_version()))
    end
    return quilt
end

return M
