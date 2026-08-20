# ============================================================================
# quilt.jl — the Julia tier of the quilt-compat contract (Base stdlib only)
#
# Same semantics as packages/core/src/{ledger,engine}.rs and the Python/Go
# bindings:
#
#   * canonical JSON  — compact, keys sorted by UTF-8 byte order, integers
#                       as integers, floats as shortest-round-trip decimals
#                       with the float marker preserved (ryu / serde_json
#                       semantics: 85.0 -> "85.0", 85 -> "85"), strings via
#                       standard JSON escaping;
#   * the wire edge   — quilt-compat/1 §1: {v, cell, ts, before, after,
#                       delta, imbalance, provenance, chain[, seq]} with
#                       delta = after - before (vectors element-wise,
#                       anything else null — never fake a number),
#                       imbalance = |after - predict(before)| (persistence
#                       prior; L2 norm for vectors; null when no prior);
#   * the sealed unit — CellLedger port of ledger.rs: append-only,
#                       hash-chained double entries; hash = sha256 over
#                       canonical JSON of the entry minus its hash;
#                       prev_hash links to the prior seal or the genesis
#                       root; reconcile() audits the books;
#   * the engine      — value/formula/sensor cells, lazy reactive
#                       recompute (set/push mark dependents stale; the
#                       next get recomputes from a dependency snapshot),
#                       Kahn propagation order with lexicographic
#                       (UTF-8 byte) tie-break.
#
# Everything is deterministic: the same entries produce the same SHA-256
# chain hashes as the Rust reference tier — bit-for-bit.
# ============================================================================

module Quilt

import SHA

export parse_json, canonical_json, sha256_hex, fmt_float,
    value_distance, serde_eq,
    wire_delta, wire_imbalance, wire_provenance, wire_edge,
    CellLedger, LedgerEntry, with_genesis, record!, open_input!, settle_output!,
    genesis_commit, chain_hash, verify_chain, reconcile, seal,
    le_seq, le_ts, le_before, le_after, le_changed, le_magnitude,
    le_imbalance, le_expected, le_prev_hash, le_hash,
    le_input_value, le_output_value, to_wire,
    cell_value, cell_expr, cell_default,
    FormulaError, parse_formula, compile_expr,
    detect_dependencies,
    parse_yaml, YamlError, parse_sheet, sheet_from_dict,
    SheetDef, CellDef,
    CellValue, QuiltEngine, from_yaml,
    set!, dependencies, dependents, propagation_order, now_millis,
    entries_of, head_of, wire_edges

const GENESIS_KIND = "quilt-cell-ledger/1"

# Sentinel distinguishing "no prediction supplied" (persistence prior)
# from an explicit forecast value.
struct _Unset end
const UNSET = _Unset()

# ---------------------------------------------------------------------------
# Hashing — stdlib SHA-256, same bytes as the Rust inline implementation
# ---------------------------------------------------------------------------

const _HEXTAB = UInt8[UInt8(c) for c in "0123456789abcdef"]

function sha256_hex(data)::String
    d = SHA.sha256(data)
    out = Base.StringVector(length(d) * 2)
    i = 1
    for b in d
        out[i] = _HEXTAB[(b >> 4) + 0x01]
        out[i+1] = _HEXTAB[(b & 0x0f) + 0x01]
        i += 2
    end
    return String(out)
end
sha256_hex(s::AbstractString) = sha256_hex(codeunits(s))

# ---------------------------------------------------------------------------
# Minimal JSON parser — the int/float distinction is part of the contract
# ---------------------------------------------------------------------------

function parse_json(s::AbstractString)
    b = codeunits(String(s))
    i = Ref(1)
    v = _jp_value(b, i)
    _jp_ws(b, i)
    i[] <= length(b) && error("trailing content in JSON at byte $(i[])")
    return v
end

function _jp_ws(b, i)
    while i[] <= length(b) &&
          (b[i[]] == 0x20 || b[i[]] == 0x09 || b[i[]] == 0x0a || b[i[]] == 0x0d)
        i[] += 1
    end
end

function _jp_lit!(b, i, word::String)
    w = codeunits(word)
    if i[] + length(w) - 1 <= length(b)
        for k in 1:length(w)
            b[i[]+k-1] == w[k] || return false
        end
        i[] += length(w)
        return true
    end
    return false
end

function _jp_value(b, i)
    _jp_ws(b, i)
    i[] > length(b) && error("unexpected end of JSON")
    c = b[i[]]
    if c == UInt8('{')
        return _jp_object(b, i)
    elseif c == UInt8('[')
        return _jp_array(b, i)
    elseif c == UInt8('"')
        return _jp_string(b, i)
    elseif c == UInt8('t')
        _jp_lit!(b, i, "true") || error("bad literal")
        return true
    elseif c == UInt8('f')
        _jp_lit!(b, i, "false") || error("bad literal")
        return false
    elseif c == UInt8('n')
        _jp_lit!(b, i, "null") || error("bad literal")
        return nothing
    elseif c == UInt8('-') || (UInt8('0') <= c <= UInt8('9'))
        return _jp_number(b, i)
    end
    error("unexpected character in JSON: $(Char(c))")
end

function _jp_number(b, i)
    start = i[]
    if b[i[]] == UInt8('-')
        i[] += 1
    end
    while i[] <= length(b)
        c = b[i[]]
        if (UInt8('0') <= c <= UInt8('9')) || c == UInt8('.') ||
           c == UInt8('e') || c == UInt8('E')
            i[] += 1
        elseif (c == UInt8('+') || c == UInt8('-')) &&
               (b[i[]-1] == UInt8('e') || b[i[]-1] == UInt8('E'))
            i[] += 1
        else
            break
        end
    end
    tok = String(b[start:i[]-1])
    # JSON: any '.' or exponent makes it a float; otherwise an Int64.
    if occursin('.', tok) || occursin('e', tok) || occursin('E', tok)
        return parse(Float64, tok)
    end
    return parse(Int64, tok)
end

function _jp_hex4(b, i)
    v = 0
    for _ in 1:4
        i[] > length(b) && error("bad unicode escape")
        c = b[i[]]
        d = if UInt8('0') <= c <= UInt8('9')
            Int(c - UInt8('0'))
        elseif UInt8('a') <= c <= UInt8('f')
            Int(c - UInt8('a')) + 10
        elseif UInt8('A') <= c <= UInt8('F')
            Int(c - UInt8('A')) + 10
        else
            error("bad hex in unicode escape")
        end
        v = v * 16 + d
        i[] += 1
    end
    return v
end

function _jp_string(b, i)
    b[i[]] == UInt8('"') || error("expected string")
    i[] += 1
    io = IOBuffer()
    while true
        i[] > length(b) && error("unterminated JSON string")
        c = b[i[]]
        if c == UInt8('"')
            i[] += 1
            break
        elseif c == UInt8('\\')
            i[] += 1
            i[] > length(b) && error("unterminated escape")
            e = b[i[]]
            if e == UInt8('"')
                write(io, '"')
            elseif e == UInt8('\\')
                write(io, '\\')
            elseif e == UInt8('/')
                write(io, '/')
            elseif e == UInt8('b')
                write(io, '\b')
            elseif e == UInt8('f')
                write(io, '\f')
            elseif e == UInt8('n')
                write(io, '\n')
            elseif e == UInt8('r')
                write(io, '\r')
            elseif e == UInt8('t')
                write(io, '\t')
            elseif e == UInt8('u')
                cp = _jp_hex4(b, i)
                if 0xD800 <= cp <= 0xDBFF && i[] + 1 <= length(b) &&
                   b[i[]] == UInt8('\\') && b[i[]+1] == UInt8('u')
                    i[] += 2
                    lo = _jp_hex4(b, i)
                    cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00)
                end
                write(io, Char(cp))
            else
                error("bad escape in JSON string")
            end
            i[] += 1
        else
            write(io, c)  # raw byte; multi-byte UTF-8 passes through
            i[] += 1
        end
    end
    return String(take!(io))
end

function _jp_array(b, i)
    i[] += 1  # '['
    out = Any[]
    _jp_ws(b, i)
    if i[] <= length(b) && b[i[]] == UInt8(']')
        i[] += 1
        return out
    end
    while true
        push!(out, _jp_value(b, i))
        _jp_ws(b, i)
        i[] > length(b) && error("unterminated array")
        c = b[i[]]
        if c == UInt8(',')
            i[] += 1
        elseif c == UInt8(']')
            i[] += 1
            return out
        else
            error("expected ',' or ']' in array")
        end
    end
end

function _jp_object(b, i)
    i[] += 1  # '{'
    out = Dict{String,Any}()
    _jp_ws(b, i)
    if i[] <= length(b) && b[i[]] == UInt8('}')
        i[] += 1
        return out
    end
    while true
        _jp_ws(b, i)
        i[] > length(b) && error("unterminated object")
        b[i[]] == UInt8('"') || error("expected object key")
        k = _jp_string(b, i)
        _jp_ws(b, i)
        (i[] <= length(b) && b[i[]] == UInt8(':')) || error("expected ':' in object")
        i[] += 1
        out[k] = _jp_value(b, i)
        _jp_ws(b, i)
        i[] > length(b) && error("unterminated object")
        c = b[i[]]
        if c == UInt8(',')
            i[] += 1
        elseif c == UInt8('}')
            i[] += 1
            return out
        else
            error("expected ',' or '}' in object")
        end
    end
end

# ---------------------------------------------------------------------------
# Canonical JSON — the hash preimage form (contract §2)
# ---------------------------------------------------------------------------

"""
    fmt_float(x) -> String

Shortest-round-trip float rendering, serde_json/ryu style: `85.0` ->
`"85.0"`, `2.5` -> `"2.5"`, `1e-5` -> `"1e-5"` (not `1e-05`), `1e16` ->
`"1e16"` (not `1e+16`). Fixed notation for `1e-4 <= |x| < 1e16`,
exponential otherwise — matching the Python tier's repr-normalized
form so the canonical bytes are identical across tiers.
"""
function fmt_float(x::Float64)::String
    isfinite(x) || return "null"  # never corrupt the chain with Inf/NaN
    if x == 0.0
        return signbit(x) ? "-0.0" : "0.0"
    end
    s = repr(x)                    # Julia's shortest round-trip digits
    neg = startswith(s, '-')
    body = neg ? s[2:end] : s
    mant, exp = if occursin('e', body)
        parts = split(body, 'e'; limit = 2)
        String(parts[1]), parse(Int, parts[2])
    else
        body, 0
    end
    ip, fp = if occursin('.', mant)
        parts = split(mant, '.'; limit = 2)
        String(parts[1]), String(parts[2])
    else
        mant, ""
    end
    digits = ip * fp
    lz = 0
    while lz < length(digits) && digits[lz+1] == '0'
        lz += 1
    end
    digits = digits[lz+1:end]
    while length(digits) > 1 && digits[end] == '0'
        digits = digits[1:prevind(digits, end)]
    end
    pointpos = length(ip) - lz + exp  # integer-digit count before the point
    n = length(digits)
    sign = neg ? "-" : ""
    if -3 <= pointpos <= 16           # fixed notation window
        if pointpos <= 0
            return sign * "0." * "0"^(-pointpos) * digits
        elseif pointpos >= n
            return sign * digits * "0"^(pointpos - n) * ".0"
        else
            return sign * digits[1:pointpos] * "." * digits[nextind(digits, pointpos):end]
        end
    else                              # exponential, no '+', no leading zeros
        m = n == 1 ? digits : digits[1:1] * "." * digits[nextind(digits, 1):end]
        return sign * m * "e" * string(pointpos - 1)
    end
end

function canonical_json(v)
    io = IOBuffer()
    _cj(io, v)
    return String(take!(io))
end

_cj(io, ::Nothing) = write(io, "null")
_cj(io, b::Bool) = write(io, b ? "true" : "false")
_cj(io, s::String) = _cj_str(io, s)
_cj(io, n::Integer) = write(io, string(Int64(n)))
_cj(io, x::AbstractFloat) = write(io, fmt_float(Float64(x)))

function _cj(io, v::AbstractVector)
    write(io, '[')
    for (k, item) in enumerate(v)
        k > 1 && write(io, ',')
        _cj(io, item)
    end
    write(io, ']')
end

function _cj(io, d::AbstractDict)
    write(io, '{')
    ks = sort!(collect(String[k for k in keys(d)]))  # UTF-8 byte order
    for (k, key) in enumerate(ks)
        k > 1 && write(io, ',')
        _cj_str(io, key)
        write(io, ':')
        _cj(io, d[key])
    end
    write(io, '}')
end

function _cj_str(io, s::String)
    write(io, '"')
    for c in s
        if c == '"'
            write(io, "\\\"")
        elseif c == '\\'
            write(io, "\\\\")
        elseif c == '\b'
            write(io, "\\b")
        elseif c == '\f'
            write(io, "\\f")
        elseif c == '\n'
            write(io, "\\n")
        elseif c == '\r'
            write(io, "\\r")
        elseif c == '\t'
            write(io, "\\t")
        elseif c < ' '
            write(io, "\\u00")
            write(io, _HEXTAB[(UInt8(c) >> 4) + 0x01])
            write(io, _HEXTAB[(UInt8(c) & 0x0f) + 0x01])
        else
            write(io, c)
        end
    end
    write(io, '"')
end

function _cj(io, v)
    error("cannot canonicalize $(typeof(v)): $(repr(v))")
end

# ---------------------------------------------------------------------------
# The generic distance metric (sealed entries' delta.magnitude)
# ---------------------------------------------------------------------------

_is_json_num(v) = (v isa Integer || v isa AbstractFloat) && !(v isa Bool)

"""
    serde_eq(a, b) -> Bool

serde_json::Value equality: int/float are *different* numbers — `40 !=
40.0` — so an edge 40 -> 40.0 is `changed` with `magnitude 0.0`. The
float-vs-int hazard of cell-ledger.md §4, preserved faithfully.
"""
function serde_eq(a, b)
    if a isa Bool || b isa Bool
        return a isa Bool && b isa Bool && a == b
    end
    if _is_json_num(a) && _is_json_num(b)
        return (a isa Integer) == (b isa Integer) && a == b
    end
    if a isa String && b isa String
        return a == b
    end
    if a === nothing && b === nothing
        return true
    end
    if a isa AbstractVector && b isa AbstractVector
        length(a) == length(b) || return false
        return all(serde_eq(x, y) for (x, y) in zip(a, b))
    end
    if a isa AbstractDict && b isa AbstractDict
        ka = Set(String[k for k in keys(a)])
        kb = Set(String[k for k in keys(b)])
        ka == kb || return false
        return all(serde_eq(a[k], b[String(k)]) for k in keys(a))
    end
    return false
end

"""
    value_distance(a, b) -> Float64

Total metric between two JSON values (port of ledger.rs): numbers ->
`|a-b|`; arrays -> mean of element-wise distances, missing elements
cost 1.0; objects -> mean over the key union, missing keys cost 1.0;
any type shift -> 1.0.
"""
function value_distance(a, b)::Float64
    if _is_json_num(a) && _is_json_num(b)
        return abs(Float64(a) - Float64(b))
    end
    if a isa AbstractVector && b isa AbstractVector
        n = max(length(a), length(b))
        n == 0 && return 0.0
        total = 0.0
        for i in 1:n
            if i <= length(a) && i <= length(b)
                total += value_distance(a[i], b[i])
            else
                total += 1.0
            end
        end
        return total / n
    end
    if a isa AbstractDict && b isa AbstractDict
        ka = Set(String[k for k in keys(a)])
        kb = Set(String[k for k in keys(b)])
        keys_union = union(ka, kb)
        isempty(keys_union) && return 0.0
        total = 0.0
        for k in keys_union
            if k in ka && k in kb
                total += value_distance(a[k], b[k])
            else
                total += 1.0
            end
        end
        return total / length(keys_union)
    end
    serde_eq(a, b) && return 0.0
    return 1.0
end

# ---------------------------------------------------------------------------
# The wire edge — quilt-compat-contract §1
# ---------------------------------------------------------------------------

"""
    wire_delta(before, after)

§1.1 — `delta = after - before`, first-person. number -> scalar
difference; equal-length numeric vectors -> element-wise difference;
anything else (strings, booleans, objects, `before: null`) ->
`nothing`. Never fake a number.
"""
function wire_delta(before, after)
    if _is_json_num(before) && _is_json_num(after)
        return Float64(after) - Float64(before)
    end
    if before isa AbstractVector && after isa AbstractVector &&
       length(before) == length(after)
        out = Any[]
        for (b, a) in zip(before, after)
            (_is_json_num(b) && _is_json_num(a)) || return nothing
            push!(out, Float64(a) - Float64(b))
        end
        return out
    end
    return nothing
end

"""
    wire_imbalance(before, after; predicted=UNSET)

§1.2 — `|after - predict(before)|`, the JEPA loss at cell grain.
Default predictor: the persistence prior — scalars give `|after -
before|`, equal-length numeric vectors give the L2 norm (a norm, not a
vector). No prior (`before: null`, no explicit forecast) -> `nothing`.
Never fake a number.
"""
function wire_imbalance(before, after; predicted = UNSET)
    prior = predicted === UNSET ? before : predicted
    if _is_json_num(prior) && _is_json_num(after)
        return abs(Float64(after) - Float64(prior))
    end
    if prior isa AbstractVector && after isa AbstractVector &&
       length(prior) == length(after)
        total = 0.0
        for (b, a) in zip(prior, after)
            (_is_json_num(b) && _is_json_num(a)) || return nothing
            total += (Float64(a) - Float64(b))^2
        end
        return sqrt(total)
    end
    return nothing
end

"""
    wire_provenance(inputs) -> String

§1.3 — `sha256_hex(canonical_json(inputs))`: the JSON array of input
values in dependency-address order (ids sorted by UTF-8 byte order);
single inputs stay wrapped in the array.
"""
wire_provenance(inputs) = sha256_hex(canonical_json(Any[i for i in inputs]))

"""
    wire_edge(cell, ts, before, after, inputs, chain; seq=nothing)

Build the full wire edge record (quilt-compat/1 §1).
"""
function wire_edge(cell::AbstractString, ts, before, after, inputs,
                   chain::AbstractString; seq = nothing)
    edge = Dict{String,Any}(
        "v" => 1,
        "cell" => String(cell),
        "ts" => Float64(ts),
        "before" => before,
        "after" => after,
        "delta" => wire_delta(before, after),
        "imbalance" => wire_imbalance(before, after),
        "provenance" => wire_provenance(inputs),
        "chain" => String(chain),
    )
    if seq !== nothing
        edge["seq"] = seq
    end
    return edge
end

# ---------------------------------------------------------------------------
# The sealed unit — Rust CellLedger port (LedgerEntry shape)
# ---------------------------------------------------------------------------

"""
First-person 'who touched me' (Rust `Provenance`): origin in
get|set|push|system; caller and trace omitted when unset, matching
serde `skip_serializing_if`.
"""
function _provenance(origin::AbstractString; caller = nothing, trace = nothing)
    origin in ("get", "set", "push", "system") ||
        error("bad provenance origin: $(repr(origin))")
    d = Dict{String,Any}("origin" => String(origin))
    caller !== nothing && (d["caller"] = caller)
    trace !== nothing && !isempty(trace) && (d["trace"] = trace)
    return d
end

"""
One sealed double entry. The hash preimage is the body minus `hash`,
canonically serialized: seq, ts, input/output postings, provenance,
delta {before, after, changed, magnitude}, expected?/imbalance?,
prev_hash.
"""
mutable struct LedgerEntry
    body::Dict{String,Any}
end

le_seq(e::LedgerEntry) = e.body["seq"]::Int
le_ts(e::LedgerEntry) = e.body["ts"]::Int
le_before(e::LedgerEntry) = e.body["delta"]["before"]
le_after(e::LedgerEntry) = e.body["delta"]["after"]
le_changed(e::LedgerEntry) = e.body["delta"]["changed"]
le_magnitude(e::LedgerEntry) = e.body["delta"]["magnitude"]
le_imbalance(e::LedgerEntry) = get(e.body, "imbalance", nothing)
le_expected(e::LedgerEntry) = get(e.body, "expected", nothing)
le_prev_hash(e::LedgerEntry) = e.body["prev_hash"]::String
le_hash(e::LedgerEntry) = e.body["hash"]::String
le_input_value(e::LedgerEntry) = e.body["input"]["value"]
le_output_value(e::LedgerEntry) = e.body["output"]["value"]

"""sha256 over canonical JSON of the entry minus its hash."""
function seal(e::LedgerEntry)
    d = Dict{String,Any}()
    for (k, v) in e.body
        k == "hash" || (d[k] = v)
    end
    return sha256_hex(canonical_json(d))
end

"""Project a sealed entry onto the quilt-compat wire edge (§1.5)."""
function to_wire(e::LedgerEntry, cell_id::AbstractString)
    return wire_edge(
        cell_id,
        Float64(le_ts(e)),
        le_before(e),
        le_after(e),
        Any[le_input_value(e)],
        le_prev_hash(e);
        seq = le_seq(e),
    )
end

_to_millis(ts)::Int = Int(floor(Float64(ts)))

"""
A per-cell, append-only, hash-chained, double-entry ledger — port of
Rust `CellLedger` (ledger.rs), the sealed side of the compat contract.
Pure data: callers pass timestamps, no clocks, no I/O. `reconcile()`
audits the books; `wire_edges` projects onto the interchange schema.
"""
mutable struct CellLedger
    cell_id::String
    genesis::Any
    genesis_ts::Any
    has_genesis::Bool
    state::Any
    entries::Vector{LedgerEntry}
    pending::Vector{Dict{String,Any}}
    next_seq::Int
    next_ticket::Int
end

"""A fresh ledger: state null, no genesis (Rust `new`)."""
CellLedger(cell_id::AbstractString) =
    CellLedger(String(cell_id), nothing, nothing, false, nothing,
               LedgerEntry[], Dict{String,Any}[], 1, 1)

"""Seed a known initial state (Rust `with_genesis`) — committed by the
chain root; the first transaction scores against the persistence prior."""
with_genesis(cell_id::AbstractString, genesis, genesis_ts) =
    CellLedger(String(cell_id), genesis, _to_millis(genesis_ts), true, genesis,
               LedgerEntry[], Dict{String,Any}[], 1, 1)

"""Record a complete double entry atomically (Rust `record[_with]`).

Under the default persistence prior the prediction is the cell's
`before` state and surprise == edge magnitude. An explicit `expected`
is recorded — and hashed — either way."""
function record!(l::CellLedger, input_value, output_value, ts;
                 provenance = nothing, expected = UNSET)
    t = _to_millis(ts)
    return _append!(l, input_value, t, output_value, t,
                    provenance === nothing ? _provenance("system") : provenance,
                    expected)
end

"""Post a debit without its credit (async cells); returns the ticket
for `settle_output!`. Does not move state or the chain."""
function open_input!(l::CellLedger, input_value, ts; provenance = nothing)
    ticket = l.next_ticket
    l.next_ticket += 1
    push!(l.pending, Dict{String,Any}(
        "ticket" => ticket,
        "ts" => _to_millis(ts),
        "input" => input_value,
        "provenance" => provenance === nothing ? _provenance("system") : provenance,
    ))
    return ticket
end

"""Close an open input with its credit, sealing the pair."""
function settle_output!(l::CellLedger, ticket::Integer, output_value, ts; expected = UNSET)
    for (i, p) in enumerate(l.pending)
        p["ticket"] == ticket || continue
        splice!(l.pending, i)
        return _append!(l, p["input"], p["ts"], output_value, _to_millis(ts),
                        p["provenance"], expected)
    end
    error("ledger '$(l.cell_id)': no open input with ticket $ticket")
end

function _append!(l::CellLedger, input_value, input_ts::Int, output_value,
                  output_ts::Int, provenance::Dict{String,Any}, expected)
    before_v = l.state
    after_v = output_value
    magnitude = value_distance(before_v, after_v)
    changed = !serde_eq(before_v, after_v)

    # A prior exists iff genesis or a completed entry; without one no
    # surprise is claimed (never fake a number). expected === nothing
    # means "no forecast supplied" — the persistence prior applies.
    has_prior = l.has_genesis || !isempty(l.entries)
    expected_v = nothing
    imbalance_v = nothing
    if expected !== UNSET && expected !== nothing
        expected_v = expected
        imbalance_v = value_distance(expected, after_v)
    elseif has_prior
        expected_v = before_v
        imbalance_v = magnitude
    end

    body = Dict{String,Any}(
        "seq" => l.next_seq,
        "ts" => input_ts,
        "input" => Dict{String,Any}("side" => "input", "value" => input_value,
                                    "ts" => input_ts),
        "output" => Dict{String,Any}("side" => "output", "value" => output_value,
                                     "ts" => output_ts),
        "provenance" => provenance,
        "delta" => Dict{String,Any}(
            "before" => before_v, "after" => after_v,
            "changed" => changed, "magnitude" => magnitude,
        ),
        "prev_hash" => chain_hash(l),
    )
    expected_v !== nothing && (body["expected"] = expected_v)
    imbalance_v !== nothing && (body["imbalance"] = imbalance_v)

    l.next_seq += 1
    entry = LedgerEntry(body)
    body["hash"] = seal(entry)

    l.state = after_v
    push!(l.entries, entry)
    return entry
end

"""Chain root for an empty ledger — commits cell identity + genesis.
Byte-identical to Rust `CellLedger`."""
function genesis_commit(l::CellLedger)
    body = Dict{String,Any}(
        "kind" => GENESIS_KIND,
        "cell_id" => l.cell_id,
        "genesis" => l.genesis,        # null when genesis-less
        "genesis_ts" => l.genesis_ts,  # null when genesis-less
    )
    return sha256_hex(canonical_json(body))
end

"""Head of the chain: last seal, or the genesis commit."""
chain_hash(l::CellLedger) =
    isempty(l.entries) ? genesis_commit(l) : l.entries[end].body["hash"]

"""Recompute every seal and prev-link (Rust `ChainAudit`)."""
function verify_chain(l::CellLedger)
    expected_prev = genesis_commit(l)
    for entry in l.entries
        if entry.body["prev_hash"] != expected_prev ||
           entry.body["hash"] != seal(entry)
            return Dict{String,Any}(
                "verified" => entry.body["seq"] - 1,
                "intact" => false,
                "first_break" => entry.body["seq"],
            )
        end
        expected_prev = entry.body["hash"]
    end
    return Dict{String,Any}("verified" => length(l.entries),
                            "intact" => true, "first_break" => nothing)
end

"""The books: matched pairs, open inputs, chain, continuity, surprise
totals (Rust `Reconciliation`)."""
function reconcile(l::CellLedger)
    audit = verify_chain(l)
    continuity = true
    prior = l.genesis  # nothing stands in for Value::Null (Rust)
    for entry in l.entries
        if !serde_eq(entry.body["delta"]["before"], prior)
            continuity = false
            break
        end
        prior = entry.body["delta"]["after"]
    end

    matched = count(e -> e.body["input"]["side"] == "input" &&
                         e.body["output"]["side"] == "output", l.entries)
    scored = Float64[e.body["imbalance"] for e in l.entries
                     if haskey(e.body, "imbalance") &&
                        e.body["imbalance"] !== nothing]
    total = isempty(scored) ? 0.0 : sum(scored)
    return Dict{String,Any}(
        "cell_id" => l.cell_id,
        "entries" => length(l.entries),
        "open_inputs" => length(l.pending),
        "matched_pairs" => matched,
        "chain_intact" => audit["intact"],
        "first_break" => audit["first_break"],
        "continuity_intact" => continuity,
        "total_surprise" => total,
        "mean_surprise" => isempty(scored) ? nothing : total / length(scored),
        "balanced" => (isempty(l.pending) && matched == length(l.entries) &&
                       audit["intact"] == true && continuity),
    )
end

entries_of(l::CellLedger) = copy(l.entries)
head_of(l::CellLedger) = isempty(l.entries) ? nothing : l.entries[end]

"""The whole history projected onto the wire edge schema."""
wire_edges(l::CellLedger) = Any[to_wire(e, l.cell_id) for e in l.entries]

# ---------------------------------------------------------------------------
# Formula — the quilt expression language (JS-flavored), pure Julia
#
#   literals     42  2.5  1e3  'str'  "str"  true  false  null
#   cell refs    a  compass.heading      (resolved by the engine)
#   operators    + - * / %  ( )  < > <= >= == !=  && || !  ?:
#   helpers      abs(x)  min(a,b)  max(a,b)  clamp(n, lo, hi)
#
# Conformance decisions (same as the Python tier): division is real
# (IEEE) division; `%` is the JS/rhai truncated remainder; the
# int/float distinction is preserved (`2 + 3` is Int 5, `2.0 + 3` is
# 5.0 — it is part of the hash preimage); `+` with a string operand
# concatenates; division by zero is an evaluation error.
# ---------------------------------------------------------------------------

struct FormulaError <: Exception
    msg::String
end
Base.showerror(io::IO, e::FormulaError) = print(io, "FormulaError: ", e.msg)

# ASCII character classes (Julia 1.12 moved isalpha & co out of Base)
_isdigit(c::Char) = '0' <= c <= '9'
_isalpha(c::Char) = ('a' <= c <= 'z') || ('A' <= c <= 'Z')
_isalnum(c::Char) = _isalpha(c) || _isdigit(c)

const _TWO_CHAR = ("<=", ">=", "==", "!=", "&&", "||")
const _ONE_CHAR = Set(['+', '-', '*', '/', '%', '(', ')', '<', '>', ',', '!', '?', ':'])

function tokenize(src::String)
    toks = Tuple{Symbol,Any}[]
    chs = collect(src)
    i, n = 1, length(chs)
    while i <= n
        c = chs[i]
        if c == ' ' || c == '\t' || c == '\r' || c == '\n'
            i += 1
            continue
        end
        if c == '\'' || c == '"'
            j = i + 1
            buf = Char[]
            while j <= n && chs[j] != c
                if chs[j] == '\\' && j + 1 <= n
                    esc = chs[j+1]
                    push!(buf, esc == 'n' ? '\n' : esc == 't' ? '\t' :
                                esc == 'r' ? '\r' : esc)
                    j += 2
                else
                    push!(buf, chs[j])
                    j += 1
                end
            end
            j > n && throw(FormulaError("unterminated string in formula: $src"))
            push!(toks, (:str, String(buf)))
            i = j + 1
            continue
        end
        if _isdigit(c) || (c == '.' && i + 1 <= n && _isdigit(chs[i+1]))
            j = i
            seen_dot = false
            seen_exp = false
            while j <= n
                d = chs[j]
                if _isdigit(d)
                    j += 1
                elseif d == '.' && !seen_dot && !seen_exp
                    seen_dot = true
                    j += 1
                elseif (d == 'e' || d == 'E') && !seen_exp && j > i
                    seen_exp = true
                    j += 1
                    if j <= n && (chs[j] == '+' || chs[j] == '-')
                        j += 1
                    end
                else
                    break
                end
            end
            text = String(chs[i:j-1])
            if seen_dot || seen_exp
                push!(toks, (:num, parse(Float64, text)))
            else
                push!(toks, (:num, parse(Int64, text)))
            end
            i = j
            continue
        end
        if _isalpha(c) || c == '_'
            j = i
            while j <= n && (_isalnum(chs[j]) || chs[j] == '_' || chs[j] == '.')
                j += 1
            end
            push!(toks, (:id, String(chs[i:j-1])))
            i = j
            continue
        end
        if i + 1 <= n
            two = String(chs[i:i+1])
            if two in _TWO_CHAR
                push!(toks, (:op, two))
                i += 2
                continue
            end
        end
        if c in _ONE_CHAR
            push!(toks, (:op, string(c)))
            i += 1
            continue
        end
        throw(FormulaError("unexpected character '$c' in formula: $src"))
    end
    return toks
end

# AST — tuples: (:num, v) (:str, v) (:ref, name) (:un, op, a)
#        (:bin, op, a, b) (:ter, c, a, b) (:call, name, [args])

function parse_formula(src::AbstractString)
    s = String(src)
    body = startswith(s, "=") ? s[2:end] : s
    toks = tokenize(body)
    isempty(toks) && throw(FormulaError("empty formula"))
    p = _FParser(toks, 1)
    ast = _pternary(p)
    p.i <= length(p.toks) && throw(FormulaError("trailing tokens at $(p.toks[p.i])"))
    return ast
end

mutable struct _FParser
    toks::Vector{Tuple{Symbol,Any}}
    i::Int
end

_ppeek(p) = p.i <= length(p.toks) ? p.toks[p.i] : (:endtok, nothing)
_pnext(p) = (t = _ppeek(p); p.i += 1; t)

function _peat_op(p, ops...)
    (kind, val) = _ppeek(p)
    if kind == :op && val in ops
        p.i += 1
        return val
    end
    return nothing
end

# precedence: ternary < or < and < equality < relational < additive
#             < multiplicative < unary < primary

function _pternary(p)
    cond = _por(p)
    if _peat_op(p, "?") !== nothing
        a = _pternary(p)
        _peat_op(p, ":") !== nothing || throw(FormulaError("expected ':' in ternary"))
        b = _pternary(p)
        return (:ter, cond, a, b)
    end
    return cond
end

function _por(p)
    left = _pand(p)
    while _peat_op(p, "||") !== nothing
        left = (:bin, "||", left, _pand(p))
    end
    return left
end

function _pand(p)
    left = _pequality(p)
    while _peat_op(p, "&&") !== nothing
        left = (:bin, "&&", left, _pequality(p))
    end
    return left
end

function _pequality(p)
    left = _prelational(p)
    while true
        op = _peat_op(p, "==", "!=")
        op === nothing && return left
        left = (:bin, op, left, _prelational(p))
    end
end

function _prelational(p)
    left = _padditive(p)
    while true
        op = _peat_op(p, "<", ">", "<=", ">=")
        op === nothing && return left
        left = (:bin, op, left, _padditive(p))
    end
end

function _padditive(p)
    left = _pmult(p)
    while true
        op = _peat_op(p, "+", "-")
        op === nothing && return left
        left = (:bin, op, left, _pmult(p))
    end
end

function _pmult(p)
    left = _punary(p)
    while true
        op = _peat_op(p, "*", "/", "%")
        op === nothing && return left
        left = (:bin, op, left, _punary(p))
    end
end

function _punary(p)
    _peat_op(p, "!") !== nothing && return (:un, "!", _punary(p))
    _peat_op(p, "-") !== nothing && return (:un, "-", _punary(p))
    _peat_op(p, "+") !== nothing && return _punary(p)
    return _pprimary(p)
end

function _pprimary(p)
    (kind, val) = _pnext(p)
    if kind == :num
        return (:num, val)
    elseif kind == :str
        return (:str, val)
    elseif kind == :id
        val == "true" && return (:num, true)
        val == "false" && return (:num, false)
        val == "null" && return (:num, nothing)
        if _peat_op(p, "(") !== nothing
            args = Any[]
            if _peat_op(p, ")") === nothing
                while true
                    push!(args, _pternary(p))
                    _peat_op(p, ",") !== nothing && continue
                    _peat_op(p, ")") !== nothing && break
                    throw(FormulaError("expected ',' or ')' in call"))
                end
            end
            return (:call, val, args)
        end
        return (:ref, val)
    elseif kind == :op && val == "("
        inner = _pternary(p)
        _peat_op(p, ")") !== nothing || throw(FormulaError("expected ')'"))
        return inner
    end
    throw(FormulaError("unexpected token ($kind, $(repr(val)))"))
end

"""
    compile_expr(src) -> f(resolve)

Parse once; the returned evaluator maps a `resolve(name)` function
(cell id -> current value) to the formula's value.
"""
function compile_expr(src::AbstractString)
    ast = parse_formula(src)
    return function evaluate(resolve)
        return _eval(ast, resolve)
    end
end

# -- evaluation — JS-flavored semantics -------------------------------------

_fmtnum_js(x::Float64) = (x == floor(x) && abs(x) < 1e15) ? string(Int64(x)) : repr(x)

function _to_str(v)
    v === nothing && return "null"
    v === true && return "true"
    v === false && return "false"
    v isa AbstractFloat && return _fmtnum_js(Float64(v))
    v isa Integer && return string(v)
    return string(v)
end

function _truthy(v)
    v === nothing && return false
    v === false && return false
    v === true && return true
    if _is_json_num(v)
        return v != 0
    end
    if v isa String
        return v != ""
    end
    return true
end

function _numeric(a, b, op)
    (_is_json_num(a) && _is_json_num(b)) ||
        throw(FormulaError("'$op' on non-numbers: $(repr(a)) $op $(repr(b))"))
    if op == "+"
        return a + b
    elseif op == "-"
        return a - b
    elseif op == "*"
        return a * b
    elseif op == "/"
        b == 0 && throw(FormulaError("division by zero"))
        return a / b  # real division always (JS semantics)
    elseif op == "%"
        b == 0 && throw(FormulaError("modulo by zero"))
        return rem(a, b)  # JS truncated remainder; Int rem Int stays Int
    end
    throw(FormulaError("unknown operator $op"))
end

function _equals(a, b)
    if a isa Bool || b isa Bool
        return a isa Bool && b isa Bool && a == b
    end
    if _is_json_num(a) && _is_json_num(b)
        return Float64(a) == Float64(b)
    end
    typeof(a) == typeof(b) || return false
    return a == b
end

function _compare(op, a, b)
    if _is_json_num(a) && _is_json_num(b)
        a, b = Float64(a), Float64(b)
    elseif a isa String && b isa String
        # ok
    else
        throw(FormulaError("'$op' on incomparable values: $(repr(a)), $(repr(b))"))
    end
    op == "<" && return a < b
    op == ">" && return a > b
    op == "<=" && return a <= b
    return a >= b
end

function _call_helper(name, args)
    _num(v) = (_is_json_num(v) ||
               throw(FormulaError("$name() expects numbers, got $(repr(v))")); v)
    if name == "abs"
        length(args) == 1 || throw(FormulaError("abs() takes one argument"))
        return abs(_num(args[1]))
    elseif name == "min" || name == "max"
        isempty(args) &&
            throw(FormulaError("$name() requires at least one argument"))
        pick = _num(args[1])
        for v in args[2:end]
            v = _num(v)
            if (name == "min" ? v < pick : v > pick)
                pick = v
            end
        end
        return pick
    elseif name == "clamp"
        length(args) == 3 ||
            throw(FormulaError("clamp(n, lo, hi) takes three arguments"))
        n, lo, hi = _num(args[1]), _num(args[2]), _num(args[3])
        n < lo && return lo
        n > hi && return hi
        return n
    end
    throw(FormulaError("unknown function $name()"))
end

function _eval(node, resolve)
    tag = node[1]
    if tag === :num
        return node[2]
    elseif tag === :str
        return node[2]
    elseif tag === :ref
        return resolve(node[2])
    elseif tag === :un
        val = _eval(node[3], resolve)
        if node[2] == "-"
            _is_json_num(val) ||
                throw(FormulaError("unary '-' on non-number $(repr(val))"))
            return -val
        end
        return !_truthy(val)
    elseif tag === :ter
        return _truthy(_eval(node[2], resolve)) ? _eval(node[3], resolve) :
               _eval(node[4], resolve)
    elseif tag === :call
        args = Any[_eval(a, resolve) for a in node[3]]
        return _call_helper(node[2], args)
    elseif tag === :bin
        op = node[2]
        if op == "&&"
            left = _eval(node[3], resolve)
            return _truthy(left) ? _eval(node[4], resolve) : left
        elseif op == "||"
            left = _eval(node[3], resolve)
            return _truthy(left) ? left : _eval(node[4], resolve)
        end
        a = _eval(node[3], resolve)
        b = _eval(node[4], resolve)
        if op == "+"
            (a isa String || b isa String) && return _to_str(a) * _to_str(b)
            return _numeric(a, b, "+")
        elseif op in ("-", "*", "/", "%")
            return _numeric(a, b, op)
        elseif op == "=="
            return _equals(a, b)
        elseif op == "!="
            return !_equals(a, b)
        elseif op in ("<", ">", "<=", ">=")
            return _compare(op, a, b)
        end
    end
    throw(FormulaError("bad AST node $node"))
end

# ---------------------------------------------------------------------------
# Dependency detection — port of formula.rs::rewrite_known_ids scan
# ---------------------------------------------------------------------------

const _BOUNDARY = Set(vcat(collect('a':'z'), collect('A':'Z'),
                           collect('0':'9'), ['_', '.']))

function _startswithat(chs::Vector{Char}, i::Int, kid::Vector{Char})
    length(kid) == 0 && return false
    i + length(kid) - 1 <= length(chs) || return false
    for k in 1:length(kid)
        chs[i+k-1] == kid[k] || return false
    end
    return true
end

"""
    detect_dependencies(expr, known_ids) -> Vector{String}

Cell ids referenced by `expr`, in first-appearance order. Whole-token
matching (the char on each side must not be alphanumeric/underscore/
dot), longest-first per position so `compass.heading` matches before
`compass`. String literals are skipped.
"""
function detect_dependencies(expr_s::AbstractString, known_ids)
    known = sort!([String(k) for k in known_ids if !isempty(String(k))];
                  by = k -> -length(k), alg = Base.Sort.MergeSort)
    chs = collect(String(expr_s))
    kc = Dict{String,Vector{Char}}(k => collect(k) for k in known)
    deps = String[]
    seen = Set{String}()
    i, n = 1, length(chs)
    while i <= n
        c = chs[i]
        if c == '\'' || c == '"'
            i += 1
            while i <= n && chs[i] != c
                i += 1
            end
            i += 1
            continue
        end
        matched = false
        for kid in known
            if _startswithat(chs, i, kc[kid])
                left_ok = i == 1 || !(chs[i-1] in _BOUNDARY)
                j = i + length(kc[kid])
                right_ok = j > n || !(chs[j] in _BOUNDARY)
                if left_ok && right_ok
                    if !(kid in seen)
                        push!(seen, kid)
                        push!(deps, kid)
                    end
                    i = j
                    matched = true
                    break
                end
            end
        end
        matched || (i += 1)
    end
    return deps
end

# ---------------------------------------------------------------------------
# miniyaml — a Base-only YAML subset parser, sufficient for quilt sheets
#
# Block mappings and block sequences by indentation, `- key: value`
# list items with continuation keys, inline flow sequences `[a, b]` and
# flow mappings `{name: boat}`, block scalars `|`, comments outside
# quotes, blank lines, quoted scalars, and scalar typing
# (int / float / bool / null / string). NOT a general YAML parser.
# ---------------------------------------------------------------------------

struct YamlError <: Exception
    msg::String
end
Base.showerror(io::IO, e::YamlError) = print(io, "YamlError: ", e.msg)

const _YINT_RE = r"^[+-]?\d+$"
const _YFLOAT_RE = r"^[+-]?(\d+\.\d*|\.\d+|\d+)([eE][+-]?\d+)?$"

function _yaml_scalar(text)
    s = String(strip(text))
    s == "" && return nothing
    if length(s) >= 2 && s[1] == s[end] && (s[1] == '\'' || s[1] == '"')
        inner = s[2:prevind(s, ncodeunits(s))]
        if s[1] == '"'
            inner = replace(inner, "\\\"" => "\"")
            inner = replace(inner, "\\\\" => "\\")
            inner = replace(inner, "\\n" => "\n")
            inner = replace(inner, "\\t" => "\t")
        else
            inner = replace(inner, "''" => "'")
        end
        return String(inner)
    end
    s in ("null", "~", "Null", "NULL") && return nothing
    s in ("true", "True", "TRUE") && return true
    s in ("false", "False", "FALSE") && return false
    if occursin(_YINT_RE, s)
        return parse(Int64, s)
    end
    if occursin(_YFLOAT_RE, s) && ('.' in s || 'e' in s || 'E' in s)
        return parse(Float64, s)
    end
    return s
end

function _split_flow(text)
    parts = String[]
    buf = Char[]
    depth = 0
    q = nothing
    for ch in collect(text)
        if q !== nothing
            push!(buf, ch)
            ch == q && (q = nothing)
            continue
        end
        if ch == '\'' || ch == '"'
            q = ch
            push!(buf, ch)
        elseif ch == '[' || ch == '{'
            depth += 1
            push!(buf, ch)
        elseif ch == ']' || ch == '}'
            depth -= 1
            push!(buf, ch)
        elseif ch == ',' && depth == 0
            push!(parts, String(buf))
            buf = Char[]
        else
            push!(buf, ch)
        end
    end
    if !isempty(strip(String(buf)))
        push!(parts, String(buf))
    end
    return parts
end

function _parse_flow(text)
    s = String(strip(text))
    if startswith(s, "[") && endswith(s, "]")
        return Any[_parse_flow(p) for p in _split_flow(s[2:prevind(s, ncodeunits(s))])]
    end
    if startswith(s, "{") && endswith(s, "}")
        out = Dict{String,Any}()
        for part in _split_flow(s[2:prevind(s, ncodeunits(s))])
            occursin(":", part) || throw(YamlError("bad flow mapping entry: $part"))
            k, v = _split_first_colon(part)
            out[string(_yaml_scalar(k))] = _parse_flow(v)
        end
        return out
    end
    return _yaml_scalar(s)
end

function _split_first_colon(part)
    chs = collect(part)
    for i in 1:length(chs)
        chs[i] == ':' && return String(chs[1:i-1]), String(chs[i+1:end])
    end
    throw(YamlError("bad flow mapping entry: $part"))
end

"""Drop a trailing # comment that is not inside quotes."""
function _strip_comment(line)
    chs = collect(line)
    q = nothing
    for i in 1:length(chs)
        c = chs[i]
        if q !== nothing
            c == q && (q = nothing)
        elseif c == '\'' || c == '"'
            q = c
        elseif c == '#' && (i == 1 || chs[i-1] in (' ', '\t'))
            return String(chs[1:i-1])
        end
    end
    return line
end

mutable struct _YLine
    indent::Int
    content::String
    num::Int
end

function _yaml_lex(src::String)
    raw_lines = split(src, '\n')
    lines = _YLine[]
    i, n = 1, length(raw_lines)
    while i <= n
        raw = rstrip(String(raw_lines[i]), '\r')
        num = i
        stripped = rstrip(_strip_comment(raw))
        i += 1
        isempty(strip(stripped)) && continue
        strip(stripped) == "---" && continue
        indent = ncodeunits(stripped) - ncodeunits(lstrip(stripped, ' '))
        prefix = stripped[1:min(indent + 1, ncodeunits(stripped))]
        occursin('\t', prefix) &&
            throw(YamlError("line $num: tabs are not valid indentation"))
        push!(lines, _YLine(indent, String(strip(stripped)), num))
        body = rstrip(stripped)
        # A block scalar header: keep the following deeper lines RAW.
        if endswith(body, "|") || endswith(body, "|-") || endswith(body, "|+")
            while i <= n
                nxt = rstrip(String(raw_lines[i]), '\r')
                if isempty(strip(nxt))
                    push!(lines, _YLine(indent + 2, "", i))
                    i += 1
                    continue
                end
                n_indent = ncodeunits(nxt) - ncodeunits(lstrip(nxt, ' '))
                n_indent <= indent && break
                push!(lines, _YLine(n_indent, String(strip(nxt)), i))
                i += 1
            end
            while !isempty(lines) && isempty(lines[end].content)
                pop!(lines)
            end
        end
    end
    return lines
end

_peek_deeper(lines, pos, indent) = pos[] <= length(lines) && lines[pos[]].indent > indent

"""Return the text up to the first top-level ':' (or all of it)."""
function _split_key_span(text)
    chs = collect(text)
    q = nothing
    for i in 1:length(chs)
        c = chs[i]
        if q !== nothing
            c == q && (q = nothing)
        elseif c == '\'' || c == '"'
            q = c
        elseif c == ':'
            return String(chs[1:i])
        end
    end
    return text
end

function _split_kv(text)
    span = _split_key_span(text)
    if endswith(span, ":")
        key = String(strip(span[1:prevind(span, ncodeunits(span))]))
        return key, String(strip(text[ncodeunits(span)+1:end]))
    end
    return text, nothing
end

function _yaml_block_scalar(lines, pos, indent)
    body = String[]
    while pos[] <= length(lines) && lines[pos[]].indent > indent
        pad = max(lines[pos[]].indent - (indent + 2), 0)
        push!(body, " "^pad * lines[pos[]].content)
        pos[] += 1
    end
    return isempty(body) ? "" : join(body, "\n") * "\n"
end

function _yaml_parse_value(val, indent, lines, pos)
    if val == "|"
        return _yaml_block_scalar(lines, pos, indent)
    elseif val == ""
        if _peek_deeper(lines, pos, indent)
            return _yaml_parse_block(lines, pos, lines[pos[]].indent)
        end
        return nothing
    end
    return _parse_flow(val)
end

function _yaml_parse_block(lines, pos, indent)
    pos[] > length(lines) && return nothing
    line = lines[pos[]]
    if startswith(line.content, "- ") || line.content == "-"
        return _yaml_parse_seq(lines, pos, indent)
    end
    return _yaml_parse_map(lines, pos, indent)
end

function _yaml_continue_map(lines, pos, item, indent)
    while pos[] <= length(lines)
        line = lines[pos[]]
        (line.indent != indent || startswith(line.content, "- ")) && break
        key, val = _split_kv(line.content)
        if val === nothing && !endswith(line.content, ":")
            break
        end
        pos[] += 1
        item[key] = _yaml_parse_value(val, indent, lines, pos)
    end
    return item
end

function _yaml_parse_seq(lines, pos, indent)
    items = Any[]
    while pos[] <= length(lines)
        line = lines[pos[]]
        (line.indent != indent ||
         !(startswith(line.content, "- ") || line.content == "-")) && break
        pos[] += 1
        rest = String(strip(line.content[2:end]))
        if isempty(rest)
            push!(items, _peek_deeper(lines, pos, indent) ?
                       _yaml_parse_block(lines, pos, lines[pos[]].indent) : nothing)
            continue
        end
        if occursin(":", _split_key_span(rest))
            # `- key: value` — an inline first key of a mapping item; the
            # item's remaining keys sit two past the dash marker.
            key, val = _split_kv(rest)
            item_indent = line.indent + 2
            item = Dict{String,Any}()
            item[key] = _yaml_parse_value(val, item_indent, lines, pos)
            push!(items, _yaml_continue_map(lines, pos, item, item_indent))
        else
            push!(items, _parse_flow(rest))
        end
    end
    return items
end

function _yaml_parse_map(lines, pos, indent)
    item = Dict{String,Any}()
    while pos[] <= length(lines)
        line = lines[pos[]]
        if line.indent != indent
            line.indent > indent && throw(YamlError(
                "line $(line.num): unexpected indent $(line.indent) (expected $indent)"))
            break
        end
        (startswith(line.content, "- ") || line.content == "-") && break
        key, val = _split_kv(line.content)
        val === nothing && throw(YamlError("line $(line.num): expected 'key: value'"))
        pos[] += 1
        item[key] = _yaml_parse_value(val, indent, lines, pos)
    end
    return item
end

"""Parse the quilt-sheet YAML subset into dicts / lists / scalars."""
function parse_yaml(src::AbstractString)
    lines = _yaml_lex(String(src))
    isempty(lines) && return Dict{String,Any}()
    pos = Ref(1)
    result = _yaml_parse_block(lines, pos, lines[1].indent)
    pos[] != length(lines) + 1 &&
        throw(YamlError("line $(lines[pos[]].num): trailing content could not be parsed"))
    return result
end

# ---------------------------------------------------------------------------
# Sheet definitions + the reactive engine
# ---------------------------------------------------------------------------

const _KNOWN_KINDS = Set(["value", "formula", "api", "program",
                          "sensor", "io", "listener", "router"])

struct CellDef
    id::String
    kind::String
    extra::Dict{String,Any}
end

cell_value(c::CellDef) = get(c.extra, "value", nothing)
cell_expr(c::CellDef) = get(c.extra, "expr", nothing)
cell_default(c::CellDef) = get(c.extra, "default", nothing)

struct SheetDef
    id::String
    cells::Vector{CellDef}
    extra::Dict{String,Any}
end

function _cells_from_doc(raw)
    raw isa AbstractVector || throw(ArgumentError("`cells` must be a list"))
    cells = CellDef[]
    seen = Set{String}()
    for (i, entry) in enumerate(raw)
        entry isa AbstractDict || throw(ArgumentError("cell #$i must be a mapping"))
        cid = get(entry, "id", nothing)
        kind = get(entry, "kind", nothing)
        (cid isa AbstractString && !isempty(strip(String(cid)))) ||
            throw(ArgumentError("cell #$i requires a non-empty `id`"))
        cid = String(cid)
        cid in seen && throw(ArgumentError("duplicate cell id: $cid"))
        kind in _KNOWN_KINDS ||
            throw(ArgumentError("cell '$cid': unknown kind $(repr(kind))"))
        push!(seen, cid)
        if kind == "value" && !haskey(entry, "value")
            throw(ArgumentError("value cell '$cid' requires `value`"))
        end
        if kind == "formula" && !(get(entry, "expr", nothing) isa AbstractString)
            throw(ArgumentError("formula cell '$cid' requires `expr`"))
        end
        push!(cells, CellDef(cid, String(kind), entry))
    end
    return cells
end

"""Build a SheetDef from an already-parsed document (e.g. the golden
JSON `sheet` section), with the same validation as `parse_sheet`."""
function sheet_from_dict(doc)
    doc isa AbstractDict || throw(ArgumentError("sheet must be a mapping"))
    sheet_id = get(doc, "id", nothing)
    (sheet_id isa AbstractString && !isempty(String(sheet_id))) ||
        throw(ArgumentError("sheet requires a top-level `id`"))
    cells = _cells_from_doc(get(doc, "cells", Any[]))
    extra = Dict{String,Any}(k => v for (k, v) in doc if k != "id" && k != "cells")
    return SheetDef(String(sheet_id), cells, extra)
end

"""Parse quilt-sheet YAML into a SheetDef (validates the core rules)."""
parse_sheet(source::AbstractString) = sheet_from_dict(parse_yaml(source))

"""What a cell holds: the value plus its own freshness."""
mutable struct CellValue
    data::Any
    status::String   # idle | ready | error
    error::Union{String,Nothing}
end
CellValueOk(d) = CellValue(d, "ready", nothing)
CellValueErr(m) = CellValue(nothing, "error", m)

mutable struct Cell
    cdef::CellDef
    dependencies::Set{String}
    dependents::Set{String}
    value::Union{CellValue,Nothing}
    stale::Bool
    evaluator::Any
end

now_millis() = time() * 1000.0

"""
A reactive grid of cells with per-cell sealed edge ledgers — the
Python/Rust engine semantics:

* **value** cells hold static data; **formula** cells are pure reactive
  expressions evaluated lazily; **sensor** cells are push-only streams
  (they read their `default` until an adapter pushes);
* `set!`/`push!` write a cell and mark every transitive dependent
  formula *stale* (nothing recomputes yet — Excel discipline);
* the next `get` of a stale formula recomputes it from a snapshot of
  its dependencies, and appends a sealed double entry: input posting =
  the dependency snapshot in dependency-address order, output posting =
  the result.
"""
mutable struct QuiltEngine
    sheet::SheetDef
    record_edges::Bool
    cells::Dict{String,Cell}
    ledgers::Dict{String,CellLedger}
end

function QuiltEngine(sheet::SheetDef; record_edges::Bool = true)
    cells = Dict{String,Cell}()
    ledgers = Dict{String,CellLedger}()
    all_ids = [c.id for c in sheet.cells]

    for cdef in sheet.cells
        cell = Cell(cdef, Set{String}(), Set{String}(), nothing, true, nothing)
        if cdef.kind == "formula"
            try
                ex = cell_expr(cdef)
                cell.evaluator = compile_expr(ex === nothing ? "" : ex)
                cell.dependencies = Set(detect_dependencies(
                    ex === nothing ? "" : ex, all_ids))
            catch e
                e isa FormulaError || rethrow()
                cell.evaluator = nothing
            end
        end
        cells[cdef.id] = cell
    end

    # Wire the reverse index (dependents).
    for cell in values(cells)
        for dep in cell.dependencies
            haskey(cells, dep) && push!(cells[dep].dependents, cell.cdef.id)
        end
    end

    # Seed initial state + genesis ledgers (ts=0, the sheet's birth).
    for cell in values(cells)
        cdef = cell.cdef
        if cdef.kind == "value"
            cell.value = CellValueOk(cell_value(cdef))
            ledgers[cdef.id] = with_genesis(cdef.id, cell_value(cdef), 0)
        elseif cdef.kind == "sensor"
            cell.value = CellValueOk(cell_default(cdef))
            if haskey(cdef.extra, "default")
                ledgers[cdef.id] = with_genesis(cdef.id, cell_default(cdef), 0)
            else
                ledgers[cdef.id] = CellLedger(cdef.id)
            end
        else
            ledgers[cdef.id] = CellLedger(cdef.id)  # no genesis: computed later
        end
    end

    return QuiltEngine(sheet, record_edges, cells, ledgers)
end

from_yaml(source::AbstractString; kwargs...) =
    QuiltEngine(parse_sheet(source); kwargs...)

_cell(e::QuiltEngine, cell_id::AbstractString) =
    haskey(e.cells, cell_id) ? e.cells[cell_id] :
    error("cell not found: $cell_id")

"""Read a cell. A stale formula recomputes here (lazy, like Excel)."""
function Base.get(e::QuiltEngine, cell_id::AbstractString; ts = nothing)
    cell = _cell(e, cell_id)
    if cell.cdef.kind == "formula"
        if cell.stale || cell.value === nothing || cell.value.status != "ready"
            return _recompute!(e, cell, ts === nothing ? now_millis() : ts)
        end
        return cell.value
    end
    return cell.value !== nothing ? cell.value : CellValueOk(cell.value)
end

"""Write a cell, mark transitive dependents stale, record the edge."""
function set!(e::QuiltEngine, cell_id::AbstractString, value; ts = nothing)
    cell = _cell(e, cell_id)
    cell.value = CellValueOk(value)
    cell.stale = false
    if e.record_edges
        record!(e.ledgers[cell_id], value, value,
                ts === nothing ? now_millis() : ts;
                provenance = _provenance("set"))
    end
    _mark_stale!(e, cell_id, Set{String}())
    return nothing
end

"""Feed a sensor/io cell from an adapter (records a push edge)."""
function Base.push!(e::QuiltEngine, cell_id::AbstractString, value; ts = nothing)
    cell = _cell(e, cell_id)
    cell.cdef.kind in ("sensor", "io") ||
        error("push! is for sensor/io cells, not $(cell.cdef.kind)")
    cell.value = CellValueOk(value)
    cell.stale = false
    if e.record_edges
        record!(e.ledgers[cell_id], value, value,
                ts === nothing ? now_millis() : ts;
                provenance = _provenance("push"))
    end
    _mark_stale!(e, cell_id, Set{String}())
    return nothing
end

"""Dependency set, sorted (dependency-address order)."""
dependencies(e::QuiltEngine, cell_id::AbstractString) =
    sort!(collect(_cell(e, cell_id).dependencies))

dependents(e::QuiltEngine, cell_id::AbstractString) =
    sort!(collect(_cell(e, cell_id).dependents))

"""A cell's history projected onto the quilt-compat wire schema."""
wire_edges(e::QuiltEngine, cell_id::AbstractString) = wire_edges(e.ledgers[cell_id])

function _mark_stale!(e::QuiltEngine, cell_id::AbstractString, seen::Set{String})
    for dep_id in sort!(collect(e.cells[cell_id].dependents))
        dep_id in seen && continue
        push!(seen, dep_id)
        dep = e.cells[dep_id]
        if dep.cdef.kind == "formula"
            dep.stale = true
        end
        _mark_stale!(e, dep_id, seen)
    end
    return nothing
end

"""Evaluate a formula against a snapshot of its dependencies."""
function _recompute!(e::QuiltEngine, cell::Cell, ts)
    if cell.evaluator === nothing
        err = CellValueErr("formula does not compile: '$(cell_expr(cell.cdef))'")
        cell.value = err
        cell.stale = false
        return err
    end

    snapshot = Dict{String,Any}()
    function resolve(name)
        haskey(snapshot, name) && return snapshot[name]
        haskey(e.cells, name) || throw(FormulaError("unknown cell: $name"))
        v = get(e, name; ts = ts).data
        snapshot[name] = v
        return v
    end

    result = try
        CellValueOk(cell.evaluator(resolve))
    catch exc
        exc isa FormulaError || exc isa DivideError || rethrow()
        CellValueErr(exc isa FormulaError ? exc.msg : sprint(showerror, exc))
    end

    cell.value = result
    cell.stale = false

    if e.record_edges && result.status == "ready"
        # Input posting: the dependency snapshot in dependency-address
        # order (sorted ids) — §1.3/§1.5.
        inputs = Any[snapshot[d] for d in sort!(collect(cell.dependencies))
                     if haskey(snapshot, d)]
        record!(e.ledgers[cell.cdef.id], inputs, result.data, ts;
                provenance = _provenance("get"))
    end
    return result
end

"""
    propagation_order(e, root) -> Vector{String}

The deterministic propagation order for a mutation of `root`
(quilt-compat op c): Kahn's algorithm over the affected closure, ties
broken by lexicographic (UTF-8 byte) address order.
"""
function propagation_order(e::QuiltEngine, root::AbstractString)
    closure = Set{String}([root])
    queue = String[root]
    while !isempty(queue)
        cur = pop!(queue)
        for dep in sort!(collect(e.cells[cur].dependents))
            if !(dep in closure)
                push!(closure, dep)
                push!(queue, dep)
            end
        end
    end

    graph = Dict{String,Vector{String}}(
        cid => sort!(collect(c.dependencies)) for (cid, c) in e.cells)
    indegree = Dict{String,Int}(cid => 0 for cid in closure)
    deps_of = Dict{String,Vector{String}}(cid => String[] for cid in closure)
    for cid in closure
        for dep in graph[cid]
            if dep in closure
                indegree[cid] += 1
                push!(deps_of[dep], cid)
            end
        end
    end

    ready = sort!([cid for (cid, d) in indegree if d == 0])
    order = String[]
    while !isempty(ready)
        cid = popfirst!(ready)
        push!(order, cid)
        for dep_id in sort!(deps_of[cid])
            indegree[dep_id] -= 1
            indegree[dep_id] == 0 && push!(ready, dep_id)
        end
        sort!(ready)
    end
    length(order) != length(closure) && error("dependency graph has a cycle")
    return order
end

end # module Quilt
