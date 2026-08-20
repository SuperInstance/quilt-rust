// Package golden runs the fixed golden-vector scenario for quilt-go: the
// same core ops the other tiers reproduce — value eval, formula eval,
// edge delta, chain hash — over a fixed sheet with caller-supplied
// timestamps, so every number and hash is deterministic bit-for-bit.
package golden

import (
	"quilt-go/internal/engine"
	"quilt-go/internal/sheet"
	"quilt-go/internal/value"
)

// Sheet is the fixed golden sheet: three independent inputs at level 0,
// two independent formulas at level 1 (the goroutine fan-out), one
// dependent at level 2.
const Sheet = `id: golden
version: "1"
cells:
  - id: temp
    kind: value
    value: 21
  - id: threshold
    kind: value
    value: 200
  - id: ambient.light
    kind: value
    value: 400
  - id: fahrenheit
    kind: formula
    expr: =temp * 9 / 5 + 32
  - id: is_dark
    kind: formula
    expr: =ambient.light < threshold
  - id: light.state
    kind: formula
    expr: =is_dark ? 'ON' : 'OFF'
`

// SpecVersion tags the emitted golden.json.
const SpecVersion = "quilt-go-golden/1"

// Run executes the golden op sequence and returns the report as a value
// tree (render with value.Pretty / value.Canonical). The ops:
//
//  1. EvalAll(ts=1000)          — value eval + formula eval, first edges
//  2. Set(temp, 30, ts=2000)    — value edge + reactive recompute
//  3. Set(ambient.light, 120, ts=3000) — two-level propagation
//
// plus fixed value_distance cases (the edge-delta metric).
func Run() (value.Value, *engine.Engine, error) {
	def, err := sheet.Parse(Sheet)
	if err != nil {
		return value.Value{}, nil, err
	}
	eng, err := engine.LoadSheet(def)
	if err != nil {
		return value.Value{}, nil, err
	}

	ops := []string{
		"eval_all ts=1000",
		"set temp = 30 ts=2000",
		"set ambient.light = 120 ts=3000",
	}
	if err := eng.EvalAll(1000); err != nil {
		return value.Value{}, nil, err
	}
	if err := eng.Set("temp", value.IntV(30), 2000); err != nil {
		return value.Value{}, nil, err
	}
	if err := eng.Set("ambient.light", value.IntV(120), 3000); err != nil {
		return value.Value{}, nil, err
	}

	// Final values.
	values := map[string]value.Value{}
	for _, id := range eng.IDs() {
		v, err := eng.Get(id)
		if err != nil {
			return value.Value{}, nil, err
		}
		values[id] = v
	}

	// Edge-delta metric cases.
	distances := map[string]value.Value{
		"d_num":  value.FloatV(value.Distance(value.IntV(40), value.IntV(85))),
		"d_arr":  value.FloatV(value.Distance(value.ArrV([]value.Value{value.IntV(1), value.IntV(2), value.IntV(3)}), value.ArrV([]value.Value{value.IntV(1), value.IntV(2)}))),
		"d_obj":  value.FloatV(value.Distance(value.ObjV(map[string]value.Value{"a": value.IntV(1)}), value.ObjV(map[string]value.Value{"a": value.IntV(2), "b": value.IntV(1)}))),
		"d_str":  value.FloatV(value.Distance(value.StrV("OFF"), value.StrV("ON"))),
		"d_null": value.FloatV(value.Distance(value.NullV(), value.FloatV(69.8))),
		"d_eq":   value.FloatV(value.Distance(value.IntV(1), value.FloatV(1.0))),
	}

	// Full edge history per cell (the cross-tier replay payload).
	edges := map[string]value.Value{}
	chains := map[string]value.Value{}
	for _, id := range eng.IDs() {
		c := eng.Cell(id)
		var list []value.Value
		for _, e := range c.Ledger.Entries() {
			var imb value.Value
			if e.Imbalance != nil {
				imb = value.FloatV(*e.Imbalance)
			} else {
				imb = value.NullV()
			}
			list = append(list, value.ObjV(map[string]value.Value{
				"after":      e.After,
				"before":     e.Before,
				"cell":       value.StrV(e.Cell),
				"chain":      value.StrV(e.Chain),
				"delta":      value.FloatV(e.Delta),
				"imbalance":  imb,
				"provenance": value.StrV(e.Provenance),
				"ts":         value.IntV(e.Ts),
				"v":          value.IntV(1),
			}))
		}
		if list == nil {
			list = []value.Value{}
		}
		edges[id] = value.ArrV(list)
		chains[id] = value.StrV(c.Ledger.ChainHash())
	}

	opList := make([]value.Value, len(ops))
	for i, op := range ops {
		opList[i] = value.StrV(op)
	}
	report := value.ObjV(map[string]value.Value{
		"spec":        value.StrV(SpecVersion),
		"ops":         value.ArrV(opList),
		"values":      value.ObjV(values),
		"distances":   value.ObjV(distances),
		"edges":       value.ObjV(edges),
		"chains":      value.ObjV(chains),
		"sheet_chain": value.StrV(eng.SheetChainHash()),
	})
	return report, eng, nil
}
