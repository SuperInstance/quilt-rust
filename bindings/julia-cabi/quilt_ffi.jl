# ============================================================================
# quilt_ffi.jl — the thin Julia FFI tier over the quilt C ABI (Base only).
#
# One native core, N bindings: this module reimplements NOTHING. It
# dlopens target/release/libquilt_cabi.so and binds every symbol of
# crates/quilt-cabi/quilt_cabi.h 1:1 via @ccall, honoring the memory
# contract of that header:
#
#   * caller-owned NUL-terminated UTF-8 args  -> Cstring parameter types
#     (ccall itself rejects embedded NULs / invalid UTF-8 loudly);
#   * library-allocated char* returns         -> decode with unsafe_string,
#     then hand the allocation back via quilt_string_free — never Libc.free;
#   * borrowed const char* (quilt_last_error) -> decode only, never freed;
#   * int returns: 0 = ok / negative = error, passed through as Cint;
#   * string returns: C_NULL (error) surfaces as `nothing`, with the
#     detail retrievable via last_error().
#
# Companion test: test_ffi.jl reproduces compat/golden.json ops (a)-(e)
# bit-for-bit through exactly these wrappers.
# ============================================================================

module QuiltFFI

using Libdl

export QUILT_ABI_VERSION, QUILT_LIB, Engine,
    abi_version,
    engine_new, engine_load_sheet, engine_get, engine_set, engine_free,
    ledger_init, ledger_record, ledger_verify, ledger_reconcile,
    ledger_chain_hash, ledgers_reset,
    quilt_string_free, last_error

# Must match QUILT_ABI_VERSION in quilt_cabi.h.
const QUILT_ABI_VERSION = UInt32(1)

const QUILT_LIB = abspath(joinpath(@__DIR__, "..", "..", "target",
                                   "release", "libquilt_cabi.so"))

# Load the cdylib once and resolve every symbol up front: a missing or
# un-dlopen-able library fails at module load, not mid-call.
const _lib = Libdl.dlopen(QUILT_LIB)

const _abi_version     = Libdl.dlsym(_lib, :quilt_abi_version)
const _engine_new      = Libdl.dlsym(_lib, :quilt_engine_new)
const _engine_load     = Libdl.dlsym(_lib, :quilt_engine_load_sheet)
const _engine_get      = Libdl.dlsym(_lib, :quilt_engine_get)
const _engine_set      = Libdl.dlsym(_lib, :quilt_engine_set)
const _engine_free     = Libdl.dlsym(_lib, :quilt_engine_free)
const _ledger_init     = Libdl.dlsym(_lib, :quilt_ledger_init)
const _ledger_record   = Libdl.dlsym(_lib, :quilt_ledger_record)
const _ledger_verify   = Libdl.dlsym(_lib, :quilt_ledger_verify)
const _ledger_reconcile = Libdl.dlsym(_lib, :quilt_ledger_reconcile)
const _ledger_chain_hash = Libdl.dlsym(_lib, :quilt_ledger_chain_hash)
const _ledgers_reset   = Libdl.dlsym(_lib, :quilt_ledgers_reset)
const _string_free     = Libdl.dlsym(_lib, :quilt_string_free)
const _last_error      = Libdl.dlsym(_lib, :quilt_last_error)

# Opaque engine handle (QuiltEngine *).
const Engine = Ptr{Cvoid}

# Decode a library-allocated char* and hand the allocation straight back —
# the header's "decode, then quilt_string_free" rule. NULL -> nothing.
function _take(p::Ptr{Cchar})
    p == C_NULL && return nothing
    s = unsafe_string(p)
    quilt_string_free(p)
    return s
end

# ---- version --------------------------------------------------------------

"The loaded library's ABI version (compare to QUILT_ABI_VERSION)."
abi_version() = @ccall $_abi_version()::UInt32

function __init__()
    v = abi_version()
    v == QUILT_ABI_VERSION ||
        error("quilt_cabi ABI version $v != header version $QUILT_ABI_VERSION " *
              "($QUILT_LIB)")
end

# ---- engine lifecycle ------------------------------------------------------

"Create a fresh, empty engine handle (caller-owned; engine_free it)."
engine_new() = @ccall $_engine_new()::Engine

"Load a YAML sheet, resetting all cell state. 0 = ok, -1 = error."
engine_load_sheet(e::Engine, yaml::AbstractString) =
    @ccall $_engine_load(e::Engine, yaml::Cstring)::Cint

"""
    engine_get(e, cell) -> Union{String, Nothing}

Read a cell's current value as JSON text ("80.0", "true", "\\"idle\\"").
Evaluates formula cells first. Returns `nothing` on error (last_error).
"""
engine_get(e::Engine, cell::AbstractString) =
    _take(@ccall $_engine_get(e::Engine, cell::Cstring)::Ptr{Cchar})

"""
    engine_set(e, cell, value_json) -> Cint

Write a cell's value and propagate downstream. For sensor/io cells this
is exactly a push. 0 = ok, -1 = error.
"""
engine_set(e::Engine, cell::AbstractString, value_json::AbstractString) =
    @ccall $_engine_set(e::Engine, cell::Cstring, value_json::Cstring)::Cint

"Destroy an engine. Tolerates NULL."
engine_free(e::Engine) = @ccall $_engine_free(e::Engine)::Cvoid

# ---- the ledger (process-global book of books, keyed by cell id) -----------

"""
    ledger_init(cell, genesis_json, ts_millis) -> Cint

Create the cell's ledger with a genesis state sealed into the chain
root. Fails (-1) if a ledger already exists for the cell. 0 on success.
"""
ledger_init(cell::AbstractString, genesis_json::AbstractString,
            ts_millis::Integer) =
    @ccall $_ledger_init(cell::Cstring, genesis_json::Cstring,
                         ts_millis::UInt64)::Cint

"""
    ledger_record(cell, input_json, output_json, ts_millis) -> Union{String, Nothing}

Record a complete double entry and return its 64-hex-char seal, or
`nothing` on error. With no prior ledger, one is created without a
genesis (null-prior first edge).
"""
ledger_record(cell::AbstractString, input_json::AbstractString,
              output_json::AbstractString, ts_millis::Integer) =
    _take(@ccall $_ledger_record(cell::Cstring, input_json::Cstring,
                                 output_json::Cstring,
                                 ts_millis::UInt64)::Ptr{Cchar})

"Recompute every seal and prev-link: 1 intact, 0 broken, -1 no such ledger."
ledger_verify(cell::AbstractString) =
    @ccall $_ledger_verify(cell::Cstring)::Cint

"The Reconciliation JSON for the cell's books, or `nothing` on error."
ledger_reconcile(cell::AbstractString) =
    _take(@ccall $_ledger_reconcile(cell::Cstring)::Ptr{Cchar})

"The chain head: last entry's seal, or the genesis commit when empty."
ledger_chain_hash(cell::AbstractString) =
    _take(@ccall $_ledger_chain_hash(cell::Cstring)::Ptr{Cchar})

"Drop every ledger in the global registry (tests, clean shutdown). Returns 0."
ledgers_reset() = @ccall $_ledgers_reset()::Cint

# ---- memory ----------------------------------------------------------------

"Free a string returned by the library (never Libc.free). NULL/nothing ok."
quilt_string_free(p::Ptr{Cchar}) = @ccall $_string_free(p::Ptr{Cchar})::Cvoid
quilt_string_free(::Nothing) = nothing

"""
    last_error() -> String

The last error message from this thread's most recent quilt call ("" if
it succeeded). Borrowed pointer underneath: decoded here, never freed.
"""
function last_error()
    p = @ccall $_last_error()::Ptr{Cchar}
    return p == C_NULL ? "" : unsafe_string(p)
end

end # module QuiltFFI
