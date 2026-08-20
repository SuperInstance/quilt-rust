package main

import (
	"fmt"
	"math"
	"os"
	"testing"

	"quilt-go/internal/golden"
	"quilt-go/internal/value"
)

// TestGoldenVectors runs the core ops — value eval, formula eval, edge
// delta, chain hash — asserts the golden numbers, writes golden.json, and
// prints the values for cross-tier cross-checking.
func TestGoldenVectors(t *testing.T) {
	report, eng, err := golden.Run()
	if err != nil {
		t.Fatalf("golden run: %v", err)
	}

	// --- value eval + formula eval ---------------------------------------
	get := func(id string) value.Value {
		return report.O["values"].O[id]
	}
	checks := []struct {
		id   string
		want value.Value
	}{
		{"temp", value.IntV(30)},
		{"threshold", value.IntV(200)},
		{"ambient.light", value.IntV(120)},
		{"fahrenheit", value.FloatV(86.0)}, // 30*9/5+32
		{"is_dark", value.BoolV(true)},     // 120 < 200
		{"light.state", value.StrV("ON")},
	}
	for _, c := range checks {
		if got := get(c.id); !value.Equal(got, c.want) {
			t.Errorf("value %s: got %s, want %s", c.id, value.Canonical(got), value.Canonical(c.want))
		}
	}

	// --- edge delta (value_distance cases) --------------------------------
	dist := func(name string) float64 { return report.O["distances"].O[name].F }
	distChecks := map[string]float64{
		"d_num":  45.0,
		"d_arr":  1.0 / 3.0,
		"d_obj":  1.0,
		"d_str":  1.0,
		"d_null": 1.0,
		"d_eq":   0.0,
	}
	for name, want := range distChecks {
		if got := dist(name); math.Abs(got-want) > 1e-12 {
			t.Errorf("distance %s: got %v, want %v", name, got, want)
		}
	}

	// --- edges recorded by the ops ---------------------------------------
	// temp: one set edge 21 -> 30, delta = imbalance = 9.0 (persistence prior).
	tempEdges := report.O["edges"].O["temp"].A
	if len(tempEdges) != 1 {
		t.Fatalf("temp edges: got %d, want 1", len(tempEdges))
	}
	te := tempEdges[0].O
	if !value.Equal(te["before"], value.IntV(21)) || !value.Equal(te["after"], value.IntV(30)) {
		t.Errorf("temp edge: before/after wrong: %s -> %s", value.Canonical(te["before"]), value.Canonical(te["after"]))
	}
	if te["delta"].F != 9.0 || te["imbalance"].F != 9.0 {
		t.Errorf("temp edge: delta=%v imbalance=%v, want 9.0/9.0", te["delta"].F, te["imbalance"].F)
	}
	// fahrenheit: first eval (null -> 69.8, delta 1.0), then recompute (69.8 -> 86.0).
	fEdges := report.O["edges"].O["fahrenheit"].A
	if len(fEdges) != 2 {
		t.Fatalf("fahrenheit edges: got %d, want 2", len(fEdges))
	}
	if fEdges[0].O["delta"].F != 1.0 {
		t.Errorf("fahrenheit first edge delta: got %v, want 1.0 (null -> number type shift)", fEdges[0].O["delta"].F)
	}
	if got := fEdges[1].O["delta"].F; math.Abs(got-16.2) > 1e-12 {
		t.Errorf("fahrenheit second edge delta: got %v, want 16.2", got)
	}
	// light.state: OFF -> ON, string change scores 1.0.
	lsEdges := report.O["edges"].O["light.state"].A
	if len(lsEdges) != 2 || lsEdges[1].O["delta"].F != 1.0 {
		t.Errorf("light.state edges wrong: %v", lsEdges)
	}

	// --- chain hashes ------------------------------------------------------
	if !eng.VerifyChains() {
		t.Fatal("chain verification failed (links or seals broken)")
	}
	// Chains are 64 lowercase hex chars.
	for id, cv := range report.O["chains"].O {
		if len(cv.S) != 64 {
			t.Errorf("chain %s: got %d chars, want 64", id, len(cv.S))
		}
	}
	if len(report.O["sheet_chain"].S) != 64 {
		t.Errorf("sheet_chain: got %d chars, want 64", len(report.O["sheet_chain"].S))
	}

	// Determinism: running the whole scenario again reproduces every hash.
	report2, _, err := golden.Run()
	if err != nil {
		t.Fatalf("golden rerun: %v", err)
	}
	if value.Canonical(report) != value.Canonical(report2) {
		t.Fatal("golden run is not deterministic")
	}

	if err := os.WriteFile("golden.json", []byte(value.Pretty(report)), 0o644); err != nil {
		t.Fatalf("write golden.json: %v", err)
	}

	// --- print the golden numbers -----------------------------------------
	fmt.Print(value.Pretty(report))
	fmt.Println("GOLDEN chains:")
	for _, id := range eng.IDs() {
		fmt.Printf("  %-14s %s\n", id, eng.Cell(id).Ledger.ChainHash())
	}
	fmt.Printf("  %-14s %s\n", "SHEET", report.O["sheet_chain"].S)
}
