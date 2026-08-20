#!/usr/bin/env julia
# ============================================================================
# test_compat.jl — quilt-jl conformance harness (the Julia tier's proof)
#
# Mirrors compat/conformance_test.rs (the reference harness) against the
# normative compat/golden.json, at the Julia tier's declared conformance
# class (quilt-compat-contract.md §4 — Julia / R row):
#
#     (a) value read      exact
#     (b) formula eval    1e-12
#     (c) propagation     exact (ordered list)
#     (d) edge            1e-9   (dyadic golden vectors hold exactly)
#     (e) chain hashes    bit-for-bit
#     (e') reconcile      1e-6   (holds exactly here)
#
# Run:  ~/.juliaup/bin/julia bindings/julia/test_compat.jl
# Exit: 0 = PASS, 1 = FAIL. Prints PASS/FAIL per op + the golden numbers.
# ============================================================================

include(joinpath(@__DIR__, "quilt.jl"))
using .Quilt

const GOLDEN_PATH = normpath(joinpath(@__DIR__, "..", "..", "compat", "golden.json"))
isfile(GOLDEN_PATH) || (println("golden.json not found at $GOLDEN_PATH"); exit(1))
const GOLDEN = parse_json(read(GOLDEN_PATH, String))

# -- the Julia tier's declared conformance class (contract §4) ---------------
const TOL_FORMULA = 1e-12
const TOL_EDGE = 1e-9
const TOL_RECONCILE = 1e-6

const failures = String[]

function check(what, cond, detail = "")
    if !cond
        push!(failures, isempty(detail) ? what : "$what — $detail")
    end
    return cond
end

function assert_close(what, got, want, tol)
    if got isa Union{Int64,Float64} && !(got isa Bool) &&
       want isa Union{Int64,Float64} && !(want isa Bool)
        return check(what, abs(Float64(got) - Float64(want)) <= tol,
                     "got $got, want $want (tol $tol)")
    elseif got isa AbstractVector && want isa AbstractVector
        check(what, length(got) == length(want),
              "length $(length(got)) vs $(length(want)): $got vs $want") || return false
        okall = true
        for (i, (gv, wv)) in enumerate(zip(got, want))
            okall &= assert_close("$what[$i]", gv, wv, tol)
        end
        return okall
    else
        return check(what, got == want, "got $(repr(got)), want $(repr(want))")
    end
end

function assert_sha256_hex(what, got, want)
    check(what, got isa AbstractString && length(got) == 64,
          "not a sha256 hex string: $got")
    check(what, got isa AbstractString && all(c in "0123456789abcdef" for c in got),
          "must be lowercase hex: $got")
    return check(what, got == want, "got $got, want $want (must be bit-for-bit)")
end

fresh_engine() = QuiltEngine(sheet_from_dict(GOLDEN["sheet"]))

# ---------------------------------------------------------------- the five core ops

function op_a_value_read()
    ok = true
    e = fresh_engine()
    for v in GOLDEN["op_a_value_read"]
        got = get(e, v["cell"])
        ok &= check("(a) status $(v["cell"])", got.status == "ready", got.status)
        ok &= assert_close("(a) value read $(v["cell"])", got.data, v["expect"], 0.0)
    end
    return ok
end

function op_b_formula_eval()
    ok = true
    e = fresh_engine()
    section = GOLDEN["op_b_formula_eval"]
    for v in section["initial"]
        got = get(e, v["cell"])
        ok &= assert_close("(b) formula $(v["cell"]) (initial)", got.data, v["expect"],
                           TOL_FORMULA)
    end
    push_ok = try
        push!(e, section["after_push"]["cell"], section["after_push"]["value"])
        true
    catch err
        push!(failures, "(b) push — $(sprint(showerror, err))")
        false
    end
    ok &= push_ok
    for v in section["post"]
        got = get(e, v["cell"])
        ok &= assert_close("(b) formula $(v["cell"]) (post-push)", got.data, v["expect"],
                           TOL_FORMULA)
    end
    return ok
end

function op_c_propagation()
    ok = true
    graph = GOLDEN["graph"]
    section = GOLDEN["op_c_propagation"]
    root = section["mutate"]["cell"]

    e = fresh_engine()
    order = propagation_order(e, root)
    want = String[s for s in section["expected_order"]]
    ok &= check("(c) propagation order", order == want, "got $order, want $want")

    # The engine's live dependency sets must equal the golden graph.
    got_graph = Dict{String,Any}(cid => dependencies(e, cid) for cid in keys(graph))
    want_graph = Dict{String,Any}(cid => sort!(Any[d for d in graph[cid]])
                                  for cid in keys(graph))
    ok &= check("(c) engine dependency graph matches golden", got_graph == want_graph,
                "$got_graph vs $want_graph")
    for (cell, deps) in section["engine_dependency_graph_must_match"]
        ok &= check("(c) deps of $cell",
                    dependencies(e, cell) == sort!(Any[d for d in deps]),
                    "$(dependencies(e, cell)) vs $deps")
    end

    push!(e, root, section["mutate"]["value"])
    got = get(e, "bilge.level")
    ok &= assert_close("(c) post-mutation read", got.data, 85.0, 0.0)
    return ok
end

function op_d_edge()
    ok = true
    for v in GOLDEN["op_d_edge"]
        name = v["name"]
        delta = wire_delta(v["before"], v["after"])
        ok &= assert_close("(d) edge $name delta", delta, v["expect"]["delta"], TOL_EDGE)
        imbalance = wire_imbalance(v["before"], v["after"])
        ok &= assert_close("(d) edge $name imbalance", imbalance,
                           v["expect"]["imbalance"], TOL_EDGE)
        prov = wire_provenance(Any[i for i in v["inputs"]])
        ok &= assert_sha256_hex("(d) edge $name provenance", prov,
                                v["expect"]["provenance"])
    end
    # The full wire-edge record shape (§1) with a seq extension.
    edge = wire_edge("x", 1000.0, 40.0, 85.0, Any[85.0], "ab"^32; seq = 1)
    want_edge = Dict{String,Any}(
        "v" => 1, "cell" => "x", "ts" => 1000.0,
        "before" => 40.0, "after" => 85.0,
        "delta" => 45.0, "imbalance" => 45.0,
        "provenance" => sha256_hex("[85.0]"),
        "chain" => "ab"^32, "seq" => 1,
    )
    ok &= check("(d) full wire edge record", edge == want_edge,
                canonical_json(edge))
    # Non-numeric edges: recorded as having happened, not faked.
    ok &= check("(d) string edge delta is null", wire_delta("idle", "running") === nothing)
    ok &= check("(d) null-prior edge delta is null", wire_delta(nothing, 7.0) === nothing)
    return ok
end

function op_e_chain()
    ok = true
    section = GOLDEN["op_e_chain"]
    transcript = section["transcript"]
    cell = transcript["cell"]
    ledger = with_genesis(cell, transcript["genesis"],
                          Int(transcript["genesis_ts"]))
    for rec in transcript["records"]
        record!(ledger, rec["input"], rec["output"], Int(rec["ts"]))
    end

    es = entries_of(ledger)
    ok &= check("(e) entry count", length(es) == length(section["entries"]),
                "$(length(es)) vs $(length(section["entries"]))")
    for (entry, want) in zip(es, section["entries"])
        ok &= check("(e) seq $(want["seq"]) contiguous from 1",
                    le_seq(entry) == want["seq"])
        ok &= assert_sha256_hex("(e) entry $(want["seq"]) prev_hash (chain link)",
                                le_prev_hash(entry), want["prev_hash"])
        ok &= assert_sha256_hex("(e) entry $(want["seq"]) seal",
                                le_hash(entry), want["hash"])
    end
    ok &= assert_sha256_hex("(e) chain_hash (head)", chain_hash(ledger),
                            section["chain_hash"])

    report = reconcile(ledger)
    want = section["reconcile"]
    ok &= check("(e) reconcile cell_id", report["cell_id"] == cell)
    ok &= check("(e) reconcile entries", report["entries"] == want["entries"])
    ok &= check("(e) reconcile open_inputs", report["open_inputs"] == want["open_inputs"])
    ok &= check("(e) reconcile matched_pairs", report["matched_pairs"] == want["matched_pairs"])
    ok &= check("(e) reconcile chain_intact", report["chain_intact"] == want["chain_intact"])
    ok &= check("(e) reconcile continuity_intact",
                report["continuity_intact"] == want["continuity_intact"])
    ok &= check("(e) reconcile balanced", report["balanced"] == want["balanced"])
    ok &= assert_close("(e) total_surprise", report["total_surprise"],
                       want["total_surprise"], TOL_RECONCILE)
    ok &= assert_close("(e) mean_surprise", report["mean_surprise"],
                       want["mean_surprise"], TOL_RECONCILE)
    return ok
end

# ---------------------------------------------------------------- supporting tiers

const GOLDEN_YAML = """
id: bilge-reflex
description: The golden sheet (YAML round-trip of the contract sheet)
cells:
  - id: bilge.level
    kind: sensor
    source: simulated
    default: 40.0
  - id: bilge.threshold
    kind: value
    value: 80.0
  - id: pump.should_run
    kind: formula
    expr: "=bilge.level >= bilge.threshold"
  - id: pump.relay_cmd
    kind: formula
    expr: "=clamp((bilge.level - bilge.threshold) * 0.5, -30.0, 30.0)"
  - id: status
    kind: value
    value: idle
"""

function support_canonical()
    ok = true
    ok &= check("canonical: compact + sorted keys",
                canonical_json(Dict{String,Any}("b" => 1, "a" => [2.5, true, nothing, "x"]))
                == "{\"a\":[2.5,true,null,\"x\"],\"b\":1}")
    ok &= check("canonical: int 85", canonical_json(85) == "85")
    ok &= check("canonical: float 85.0", canonical_json(85.0) == "85.0")
    ok &= check("canonical: float 2.5", canonical_json(2.5) == "2.5")
    ok &= check("canonical: 1e-5 (not 1e-05)", canonical_json(1e-5) == "1e-5")
    ok &= check("canonical: 1e16 (not 1e+16)", canonical_json(1e16) == "1e16")
    ok &= check("canonical: insertion order irrelevant",
                canonical_json(Dict{String,Any}("x" => 1, "y" => 2)) ==
                canonical_json(Dict{String,Any}("y" => 2, "x" => 1)))
    ok &= check("sha256: empty", sha256_hex("") ==
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
    ok &= check("sha256: abc", sha256_hex("abc") ==
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
    # value_distance — vectors from the Rust unit tests
    cases = [(3.0, 5.0, 2.0), (true, true, 0.0), (false, 0, 1.0),
             ([1.0, 2.0], [3.0, 2.0], 1.0), ([1.0], [1.0, 5.0], 0.5),
             (Dict{String,Any}("a" => 1), Dict{String,Any}("a" => 1, "b" => 2), 0.5),
             (Dict{String,Any}(), Dict{String,Any}(), 0.0)]
    for (a, b, want) in cases
        ok &= assert_close("value_distance($a, $b)", value_distance(a, b), want, 1e-12)
    end
    return ok
end

function support_yaml_sheet()
    ok = true
    sheet = parse_sheet(GOLDEN_YAML)
    golden = sheet_from_dict(GOLDEN["sheet"])
    ok &= check("yaml: sheet id", sheet.id == golden.id)
    ok &= check("yaml: cell ids", [c.id for c in sheet.cells] == [c.id for c in golden.cells])
    ok &= check("yaml: cell kinds", [c.kind for c in sheet.cells] == [c.kind for c in golden.cells])
    for (a, b) in zip(sheet.cells, golden.cells)
        ok &= check("yaml: $(a.id) value", cell_value(a) == cell_value(b))
        ok &= check("yaml: $(a.id) expr", cell_expr(a) == cell_expr(b))
        ok &= check("yaml: $(a.id) default", cell_default(a) == cell_default(b))
    end
    # The YAML-built engine behaves identically (op b spot check).
    e = QuiltEngine(sheet)
    ok &= check("yaml engine: should_run initial", get(e, "pump.should_run").data == false)
    ok &= check("yaml engine: relay_cmd initial", get(e, "pump.relay_cmd").data == -20.0)
    # Bad sheets are rejected.
    for (src, what) in [
        ("id: dup\ncells:\n  - id: a\n    kind: value\n    value: 1\n  - id: a\n    kind: value\n    value: 2\n", "duplicate id"),
        ("id: bad\ncells:\n  - id: x\n    kind: value\n", "value cell requires value"),
        ("id: bad\ncells:\n  - id: x\n    kind: formula\n", "formula cell requires expr"),
    ]
        rejected = try
            parse_sheet(src)
            false
        catch err
            err isa ArgumentError || err isa YamlError
        end
        ok &= check("yaml: rejects $what", rejected)
    end
    return ok
end

function support_dependency_detection()
    ok = true
    deps = detect_dependencies(
        "=clamp((bilge.level - bilge.threshold) * 0.5, -30.0, 30.0)",
        ["bilge.level", "bilge.threshold", "pump.relay_cmd"])
    ok &= check("deps: golden formula", deps == ["bilge.level", "bilge.threshold"])
    deps = detect_dependencies("=compass.heading > 10 ? a : b",
                               ["compass", "compass.heading", "a", "b"])
    ok &= check("deps: longest id wins", deps == ["compass.heading", "a", "b"])
    deps = detect_dependencies("='temp' + temp", ["temp"])
    ok &= check("deps: string literals not references", deps == ["temp"])
    return ok
end

function support_engine_ledger()
    ok = true
    # Push records the golden scalar edge.
    e = fresh_engine()
    push!(e, "bilge.level", 85.0; ts = 2000.0)
    es = wire_edges(e.ledgers["bilge.level"])
    ok &= check("engine: one wire edge after push", length(es) == 1)
    ed = es[1]
    ok &= check("engine: edge before/after", (ed["before"], ed["after"]) == (40.0, 85.0))
    ok &= check("engine: edge delta", ed["delta"] == 45.0)
    ok &= check("engine: edge imbalance", ed["imbalance"] == 45.0)
    ok &= check("engine: edge provenance", ed["provenance"] == wire_provenance(Any[85.0]))
    ok &= check("engine: edge chain is genesis root",
                ed["chain"] == genesis_commit(e.ledgers["bilge.level"]))
    ok &= check("engine: edge ts", ed["ts"] == 2000.0)
    ok &= check("engine: edge v", ed["v"] == 1)
    ok &= check("engine: edge seq", ed["seq"] == 1)

    # Formula recompute posts the dependency snapshot.
    e2 = fresh_engine()
    push!(e2, "bilge.level", 85.0; ts = 2000.0)
    get(e2, "pump.relay_cmd"; ts = 2001.0)
    head = head_of(e2.ledgers["pump.relay_cmd"])
    ok &= check("engine: formula input = dep snapshot",
                le_input_value(head) == Any[85.0, 80.0])
    ok &= check("engine: formula output", le_output_value(head) == 2.5)
    ok &= check("engine: formula provenance origin",
                head.body["provenance"]["origin"] == "get")

    # The books balance after the scenario.
    e3 = fresh_engine()
    push!(e3, "bilge.level", 85.0; ts = 2000.0)
    for cell in ("pump.should_run", "pump.relay_cmd")
        get(e3, cell; ts = 2001.0)
    end
    for (cid, ledger) in e3.ledgers
        r = reconcile(ledger)
        ok &= check("engine: books balance ($cid)", r["balanced"] == true)
        ok &= check("engine: chain intact ($cid)", verify_chain(ledger)["intact"] == true)
    end
    return ok
end

# ---------------------------------------------------------------- reporting

function run_section(label, f)
    ok = try
        f()
    catch err
        push!(failures, "$label — UNCAUGHT: $(sprint(showerror, err))")
        false
    end
    println("  $(rpad(label, 34)) $(ok ? "PASS" : "FAIL")")
    return ok
end

function report_numbers()
    g = GOLDEN
    println("─"^76)
    println("golden numbers (compat/golden.json)")
    println("─"^76)
    println("(a) value reads:")
    for v in g["op_a_value_read"]
        println("    $(rpad(v["cell"], 16)) = $(repr(v["expect"]))")
    end
    b = g["op_b_formula_eval"]
    println("(b) formula eval:")
    for v in b["initial"]
        println("    $(rpad(v["cell"], 16)) = $(repr(v["expect"]))   (initial)")
    end
    println("    push bilge.level -> $(repr(b["after_push"]["value"]))")
    for v in b["post"]
        println("    $(rpad(v["cell"], 16)) = $(repr(v["expect"]))   (post-push)")
    end
    c = g["op_c_propagation"]
    println("(c) propagation order after $(c["mutate"]["cell"])=$(c["mutate"]["value"]):")
    println("    $(join(c["expected_order"], ", "))")
    println("(d) wire edges:")
    for v in g["op_d_edge"]
        println("    $(rpad(v["name"], 20)) delta=$(repr(v["expect"]["delta"]))  " *
                "imb=$(repr(v["expect"]["imbalance"]))  " *
                "prov=$(v["expect"]["provenance"][1:16])…")
    end
    e = g["op_e_chain"]
    println("(e) chain ($(e["transcript"]["cell"]), " *
            "genesis $(repr(e["transcript"]["genesis"])) @ $(e["transcript"]["genesis_ts"])):")
    for entry in e["entries"]
        println("    seq $(entry["seq"])  prev $(entry["prev_hash"][1:16])…  " *
                "seal $(entry["hash"][1:16])…")
    end
    println("    chain_hash          = $(e["chain_hash"])")
    rec = e["reconcile"]
    println("    reconcile: entries=$(rec["entries"]) balanced=$(rec["balanced"]) " *
            "total_surprise=$(rec["total_surprise"]) mean_surprise=$(rec["mean_surprise"])")
    println("─"^76)
end

function main()
    if GOLDEN["contract"] != "quilt-compat/1"
        println("golden.json contract $(repr(GOLDEN["contract"])) != 'quilt-compat/1' — " *
                "this tier implements quilt-compat/1; fail loudly, never guess (§7).")
        return 1
    end
    if get(GOLDEN["spec"], "edge_schema_v", nothing) != 1
        println("golden.json edge_schema_v != 1 — refusing to guess (§7).")
        return 1
    end

    println("=== quilt-jl conformance (tier: julia) ===")
    println("contract: $(GOLDEN["contract"])  golden: compat/golden.json")
    println()

    ok = true
    ok &= run_section("[a] value cell read", op_a_value_read)
    ok &= run_section("[b] formula cell eval", op_b_formula_eval)
    ok &= run_section("[c] propagation order", op_c_propagation)
    ok &= run_section("[d] edge record", op_d_edge)
    ok &= run_section("[e] chain + reconcile", op_e_chain)
    println()
    ok &= run_section("[+] canonical serialization", support_canonical)
    ok &= run_section("[+] sheet parsing (YAML)", support_yaml_sheet)
    ok &= run_section("[+] dependency detection", support_dependency_detection)
    ok &= run_section("[+] engine-ledger integration", support_engine_ledger)

    println()
    report_numbers()

    println()
    println("chain_hash: $(GOLDEN["op_e_chain"]["chain_hash"])")
    println()

    if !isempty(failures)
        println("failures:")
        for f in unique(failures)
            println("  - $f")
        end
        println()
    end

    if ok && isempty(failures)
        println("RESULT: PASS — julia tier conforms to quilt-compat/1")
        return 0
    end
    println("RESULT: FAIL — see failures above")
    return 1
end

exit(main())
