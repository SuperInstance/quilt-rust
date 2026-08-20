package engine

import (
	"testing"

	"quilt-go/internal/sheet"
	"quilt-go/internal/value"
)

const testSheet = `id: t
cells:
  - id: a
    kind: value
    value: 2
  - id: b
    kind: value
    value: 3
  - id: sum
    kind: formula
    expr: =a + b
  - id: product
    kind: formula
    expr: =a * b
  - id: total
    kind: formula
    expr: =sum + product
`

func load(t *testing.T) *Engine {
	t.Helper()
	def, err := sheet.Parse(testSheet)
	if err != nil {
		t.Fatal(err)
	}
	e, err := LoadSheet(def)
	if err != nil {
		t.Fatal(err)
	}
	return e
}

func TestDependencyDetectionAndLevels(t *testing.T) {
	e := load(t)
	if len(e.Cell("sum").Deps) != 2 {
		t.Errorf("sum deps: %v", e.Cell("sum").Deps)
	}
	if e.Cell("total").level != 2 || e.Cell("sum").level != 1 || e.Cell("a").level != 0 {
		t.Errorf("levels: total=%d sum=%d a=%d", e.Cell("total").level, e.Cell("sum").level, e.Cell("a").level)
	}
}

func TestEvalAllAndPropagation(t *testing.T) {
	e := load(t)
	if err := e.EvalAll(1); err != nil {
		t.Fatal(err)
	}
	if v, _ := e.Get("total"); !value.Equal(v, value.IntV(11)) {
		t.Errorf("total: got %s, want 11", value.Canonical(v))
	}
	if err := e.Set("a", value.IntV(10), 2); err != nil {
		t.Fatal(err)
	}
	if v, _ := e.Get("total"); !value.Equal(v, value.IntV(43)) {
		t.Errorf("total after set: got %s, want 43 (13+30)", value.Canonical(v))
	}
	// Edges recorded: sum re-evaluated once by EvalAll, once by Set.
	if n := len(e.Cell("sum").Ledger.Entries()); n != 2 {
		t.Errorf("sum edges: got %d, want 2", n)
	}
	if !e.VerifyChains() {
		t.Error("chains failed verification")
	}
}

func TestCycleRejected(t *testing.T) {
	def, err := sheet.Parse("id: c\ncells:\n  - id: x\n    kind: formula\n    expr: =y + 1\n    deps: [y]\n  - id: 'y'\n    kind: formula\n    expr: =x + 1\n    deps: [x]\n")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := LoadSheet(def); err == nil {
		t.Error("cycle accepted")
	}
}
