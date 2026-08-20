# quilt-go — the parallel/optimizing tier of the quilt runtime

An independent Go implementation of the quilt cell runtime: **standard
library only**, no third-party dependencies (the YAML subset parser and the
formula evaluator are hand-rolled, per the tier's constraint). Goroutines
are the substrate for reactive propagation: cells in the same topological
level are independent, so they evaluate concurrently; results are applied
serially in sorted id order, keeping every ledger chain deterministic
bit-for-bit.

Scope: `value` and `formula` cells. Sheets using other kinds (`sensor`,
`api`, `program`, `listener`, `router`, `io`) are rejected at parse time —
loudly, not silently.

## Layout

```
main.go                  CLI: run <sheet.yaml> | golden [out.json]
golden_test.go           golden-vector test: prints the numbers, writes golden.json
golden.json              generated golden vectors (quilt-go-golden/1)
internal/value/          value model: int/float distinction, canonical JSON, value_distance
internal/expr/           formula evaluator (lexer + recursive descent + tree-walk)
internal/sheet/          YAML-subset parser for the sheet format
internal/ledger/         canonical edge schema + sha256 provenance + chain hash
internal/engine/         reactive engine: dep detection, topological levels, parallel fan-out
internal/golden/         the fixed golden scenario
```

## Run it

```sh
go test ./...            # everything
go test -v -run TestGoldenVectors .   # prints the golden numbers
go run . run sheet.yaml               # evaluate a sheet
go run . golden golden.json           # emit golden vectors
```

## Canonical edge schema (v1)

Every cell transaction records one edge:

```jsonc
{"v":1, "cell":"temp", "ts":2000, "before":21, "after":30,
 "delta":9.0, "imbalance":9.0,
 "provenance":"<sha256 hex>", "chain":"<sha256 hex>"}
```

Pinned semantics (a port that implements these reproduces the hashes in
`golden.json` bit-for-bit):

- **`delta`** = `value_distance(before, after)` — the edge magnitude, the
  total metric from `docs/cell-ledger.md`: numbers score `|a−b|`; arrays
  the mean element-wise distance (missing elements cost 1.0); objects the
  mean over the key union (missing keys cost 1.0); equal values score 0;
  everything else (type shift, unequal strings/bools) scores 1.0.
- **`imbalance`** = `value_distance(expected, after)` under the
  persistence prior (`expected = before`), so `imbalance == delta` by
  construction. `null` when the cell has no prior (no genesis, no edges).
  Never fake a number.
- **`provenance`** = `sha256_hex(canonical_json({"v":1,"cell","ts",
  "before","after","delta","imbalance","origin","caller","trace"}))` —
  the seal over the edge body plus who touched the cell. `origin` is an
  engine verb (`get`|`set`|`call`|`push`|`system`); `caller` is null when
  unknown; `trace` is the ancestor chain, outermost first.
- **`chain_i`** = `sha256_hex(chain_{i-1} + ":" + provenance_i)`;
  `chain_0` is the genesis commit
  `sha256_hex(canonical_json({"kind":"quilt-edge/1","cell","genesis",
  "genesis_ts"}))`.
- **Sheet chain** = `sha256_hex(canonical_json({"chains":{cell:chain},
  "sheet":sheet_id}))`.

`canonical_json` is the form pinned in `docs/cell-ledger.md`: compact, no
whitespace, object keys sorted by UTF-8 byte order, standard JSON string
escaping (no HTML escaping, no `/` escape), and **integers render as
integers while floats keep a decimal point or exponent** (`40` vs `40.0`,
ryū/serde_json semantics — the JS `40.0 → 40` hazard is handled here).

## Engine semantics

- **Dependency auto-detection**: formula expressions are scanned for
  dotted-identifier tokens naming known cells (same rule as the Rust
  engine's token scan); declared `deps:` merge in.
- **Reactive recompute**: `Set` on a value cell records its edge, then
  recomputes the transitive dependents level by level — each level's cells
  in their own goroutines, applied in sorted id order.
- **Every touch is an edge**: initial `EvalAll` records origin `get`,
  `Set` records `set`, propagation recomputes record `system`.
- **Genesis**: value cells declare their initial value at `ts=0`; formula
  cells declare `null` at `ts=0`, so a formula's first evaluation is a
  scored edge (`null → value`, delta 1.0) like any other.
- **Timestamps are caller-supplied** (millis). The engine has no clocks;
  golden vectors pin `ts` to fixed values for determinism.

### Documented numeric divergences (pinned by the golden vectors)

- `+ - * %` on two ints stay int; `/` is **always float** (TS semantics,
  not rhai's truncating integer division); mixed int/float promotes to float.
- `==` on numbers compares numerically (`1 == 1.0` is true); `Equal`
  (used for edge `changed` detection) is type-strict (`1` vs `1.0`
  differ, matching serde_json's variant equality).
- Helper functions: `abs min max clamp floor ceil round sqrt pow`.
- Extreme float magnitudes (|exp10| ≥ 21) may spell their exponent
  differently from ryū; golden vectors stay in the plain-decimal range.

## Toolchain note

`go` was not installed on the build host. Go 1.27.0 was fetched into
`target/tmp/go` (gitignored, inside this repo — nothing system-wide):

```sh
export PATH=$PWD/target/tmp/go/bin:$PATH
```

## Golden numbers (current)

```
values:     temp=30  threshold=200  ambient.light=120  fahrenheit=86.0
            is_dark=true  light.state="ON"
distances:  d_num=45.0  d_arr=0.3333333333333333  d_obj=1.0
            d_str=1.0   d_null=1.0                d_eq=0.0
sheet_chain: c5982e722b03f76a0ad93f60493b321b524b4e90fbae7f7781591b3aaeecd655
```

Per-cell chain heads and every edge body are in `golden.json`; regenerate
with `go test -v -run TestGoldenVectors .` or `go run . golden`.
